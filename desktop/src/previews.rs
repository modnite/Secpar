//! Live computer previews: one background connection per online computer that this device has
//! connected to before. Each asks for thumbnails (`thumb.sub`) and sends back decoded frames.
//! Nothing here runs on the render thread, and a computer being streamed is never watched.
use crate::vault;
use eframe::egui::{self, ColorImage};
use secpar_client::control::{self, Computer, Connected, Incoming};
use serde_json::json;
use secpar_client::connlog;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch, Notify};

/// Logical size of a preview in a computer card (16:9). It sits beside the text.
pub const WIDTH: f32 = 80.;
pub const HEIGHT: f32 = 45.;
/// Card corners are 7 with an 8 margin, so the picture steps in to 4.
const RADIUS: u8 = 4;
const RETRY: Duration = Duration::from_secs(10);
/// Pause before asking a host again that cannot give previews (no thumbnails, credential gone).
const UNSUPPORTED_RETRY: Duration = Duration::from_secs(300);
/// A preview that was just used for the Files page starts again soon, not after the usual pause.
const AFTER_HANDOFF: Duration = Duration::from_secs(1);
/// How long the Files page waits for a preview to hand over its connection before it connects itself.
pub const HANDOFF_WAIT: Duration = Duration::from_millis(1500);
/// Hosts send 320 px wide frames. Anything much larger is not a thumbnail.
const MAX_SIDE: u32 = 1280;

#[derive(Clone, PartialEq, Debug)]
pub struct Target {
    pub id: String,
    /// Vault entry that holds the credential saved by an earlier connection.
    pub key: String,
    /// Identity from the account. The saved credential must match it.
    pub fingerprint: [u8; 32],
}
#[derive(Clone, PartialEq, Default, Debug)]
pub struct Wanted {
    pub targets: Vec<Target>,
}

type Pending = Arc<Mutex<HashMap<String, Option<ColorImage>>>>;

/// Open connections that can change hands. A preview keeps an authenticated connection to a
/// computer; the Files page asks for it instead of connecting a second time. The connection comes
/// back with its welcome already done, so the page starts listing at once.
pub struct Handoffs<T> {
    slots: Mutex<HashMap<String, mpsc::Sender<oneshot::Sender<T>>>>,
}
/// A connection on offer. Dropping it takes the offer back.
pub struct Offer<'a, T> {
    pub requests: mpsc::Receiver<oneshot::Sender<T>>,
    owner: &'a Handoffs<T>,
    key: String,
}
impl<T> Drop for Offer<'_, T> {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.owner.slots.lock() {
            slots.remove(&self.key);
        }
    }
}
impl<T> Default for Handoffs<T> {
    fn default() -> Self {
        Self { slots: Mutex::new(HashMap::new()) }
    }
}
impl<T> Handoffs<T> {
    pub fn offer(&self, key: &str) -> Offer<'_, T> {
        let (tx, requests) = mpsc::channel(1);
        if let Ok(mut slots) = self.slots.lock() {
            slots.insert(key.to_string(), tx);
        }
        Offer { requests, owner: self, key: key.to_string() }
    }
    /// The connection offered under `key`, or None at once when nothing is offered, or after
    /// `wait` when the owner does not answer.
    pub async fn take(&self, key: &str, wait: Duration) -> Option<T> {
        let tx = self.slots.lock().ok()?.get(key)?.clone();
        let (reply, answer) = oneshot::channel();
        tx.try_send(reply).ok()?;
        tokio::time::timeout(wait, answer).await.ok()?.ok()
    }
}
/// The connections the previews have open, offered to the Files page.
pub fn handoffs() -> &'static Handoffs<Connected> {
    static SHELF: std::sync::OnceLock<Handoffs<Connected>> = std::sync::OnceLock::new();
    SHELF.get_or_init(Handoffs::default)
}

pub struct Previews {
    wanted: Option<watch::Sender<Wanted>>,
    pending: Pending,
}
impl Previews {
    pub fn new(ctx: egui::Context) -> Self {
        let (wanted, rx) = watch::channel(Wanted::default());
        let pending = Pending::default();
        let shared = pending.clone();
        std::thread::spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                runtime.block_on(manage(rx, shared, ctx));
            }
        });
        Self { wanted: Some(wanted), pending }
    }
    /// No worker. Used by the screenshot review.
    #[cfg(feature = "ui-review")]
    pub fn inert() -> Self {
        Self { wanted: None, pending: Pending::default() }
    }
    pub fn set(&self, wanted: Wanted) {
        if let Some(tx) = &self.wanted {
            tx.send_if_modified(|current| {
                let changed = *current != wanted;
                if changed {
                    *current = wanted;
                }
                changed
            });
        }
    }
    /// Newest frame per computer since the last call. `None` means the preview is gone.
    pub fn take(&self) -> Vec<(String, Option<ColorImage>)> {
        self.pending.lock().map(|mut p| p.drain().collect()).unwrap_or_default()
    }
}

/// Part of the picture that fills a 16:9 box without stretching it. Wider or taller pictures
/// lose the same amount from both edges.
pub fn cover_uv(width: usize, height: usize) -> egui::Rect {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    let (picture, frame) = (w / h, WIDTH / HEIGHT);
    let (kx, ky) = if picture > frame { (frame / picture, 1.) } else { (1., picture / frame) };
    egui::Rect::from_min_max(egui::pos2((1. - kx) / 2., (1. - ky) / 2.), egui::pos2((1. + kx) / 2., (1. + ky) / 2.))
}

/// The preview in a computer card: 80 x 45, rounded one step inside the card, thin border.
pub fn show(ui: &mut egui::Ui, texture: &egui::TextureHandle) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(WIDTH, HEIGHT), egui::Sense::hover());
    let [w, h] = texture.size();
    egui::Image::new(texture).uv(cover_uv(w, h)).corner_radius(RADIUS).paint_at(ui, rect);
    ui.painter().rect_stroke(rect, RADIUS, egui::Stroke::new(1_f32, crate::design::LINE), egui::StrokeKind::Inside);
}

/// Decodes a thumbnail JPEG. Rejects empty or oversized pictures before decoding them.
pub fn decode(jpeg: &[u8]) -> Option<ColorImage> {
    let reader = || image::ImageReader::with_format(Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let (w, h) = reader().into_dimensions().ok()?;
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let rgba = reader().decode().ok()?.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw()))
}

/// Which previews to start and which to stop. `running` pairs a key with "already stopping".
/// A stopping one is left alone until it ends, then it can start again.
pub fn plan(wanted: &[String], running: &[(String, bool)]) -> (Vec<String>, Vec<String>) {
    let start = wanted
        .iter()
        .filter(|key| !running.iter().any(|(k, _)| k == *key))
        .cloned()
        .collect();
    let stop = running
        .iter()
        .filter(|(key, stopping)| !stopping && !wanted.contains(key))
        .map(|(key, _)| key.clone())
        .collect();
    (start, stop)
}

/// One texture per computer on the render thread, keyed by computer id.
pub struct Cache<T> {
    items: HashMap<String, T>,
}
impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self { items: HashMap::new() }
    }
}
impl<T> Cache<T> {
    pub fn apply<I>(&mut self, updates: Vec<(String, Option<I>)>, mut make: impl FnMut(&str, I) -> T) {
        for (id, image) in updates {
            match image {
                Some(image) => {
                    let item = make(&id, image);
                    self.items.insert(id, item);
                }
                None => {
                    self.items.remove(&id);
                }
            }
        }
    }
    /// Drops what is no longer shown, such as a computer that went offline or is being streamed.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.items.retain(|id, _| keep(id));
    }
    pub fn get(&self, id: &str) -> Option<&T> {
        self.items.get(id)
    }
    #[cfg(feature = "ui-review")]
    pub fn insert(&mut self, id: &str, item: T) {
        self.items.insert(id.into(), item);
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

struct Task {
    id: String,
    handle: tokio::task::JoinHandle<()>,
    stop: Arc<Notify>,
    live: Arc<AtomicBool>,
    /// Set when the host cannot give previews, so it is not asked again every few seconds.
    unsupported: Arc<AtomicBool>,
    /// The connection went to the Files page.
    handed: Arc<AtomicBool>,
    stopping: bool,
}

fn publish(pending: &Pending, ctx: &egui::Context, id: &str, image: Option<ColorImage>) {
    if let Ok(mut p) = pending.lock() {
        p.insert(id.into(), image);
    }
    ctx.request_repaint();
}

async fn manage(mut wanted: watch::Receiver<Wanted>, pending: Pending, ctx: egui::Context) {
    let mut tasks: HashMap<String, Task> = HashMap::new();
    let mut retry: HashMap<String, Instant> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            changed = wanted.changed() => if changed.is_err() { break },
        }
        let want = wanted.borrow().clone();
        tasks.retain(|key, task| {
            if !task.handle.is_finished() {
                return true;
            }
            let wait = if task.unsupported.load(Ordering::Acquire) {
                UNSUPPORTED_RETRY
            } else if task.handed.load(Ordering::Acquire) {
                AFTER_HANDOFF
            } else {
                RETRY
            };
            retry.insert(key.clone(), Instant::now() + wait);
            publish(&pending, &ctx, &task.id, None);
            false
        });
        retry.retain(|key, _| want.targets.iter().any(|t| t.key == *key));
        let keys: Vec<String> = want.targets.iter().map(|t| t.key.clone()).collect();
        let running: Vec<(String, bool)> = tasks.iter().map(|(k, t)| (k.clone(), t.stopping)).collect();
        let (start, stop) = plan(&keys, &running);
        for key in stop {
            if let Some(task) = tasks.get_mut(&key) {
                task.live.store(false, Ordering::Release);
                task.stop.notify_one();
                task.stopping = true;
                publish(&pending, &ctx, &task.id, None);
            }
        }
        for key in start {
            if retry.get(&key).is_some_and(|at| *at > Instant::now()) {
                continue;
            }
            let Some(target) = want.targets.iter().find(|t| t.key == key).cloned() else { continue };
            let (stop, live, unsupported, handed) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
            let id = target.id.clone();
            let handle = tokio::spawn(watch_one(target, stop.clone(), live.clone(), unsupported.clone(), handed.clone(), pending.clone(), ctx.clone()));
            tasks.insert(key, Task { id, handle, stop, live, unsupported, handed, stopping: false });
        }
    }
    for task in tasks.values() {
        task.stop.notify_one();
    }
    // Let each task say thumb.sub off before the runtime goes away.
    tokio::time::sleep(Duration::from_millis(300)).await;
}

async fn watch_one(target: Target, stop: Arc<Notify>, live: Arc<AtomicBool>, unsupported: Arc<AtomicBool>, handed: Arc<AtomicBool>, pending: Pending, ctx: egui::Context) {
    let Some(saved) = vault::load(&target.key)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<Computer>(&value).ok())
        .filter(|saved| saved.fingerprint == target.fingerprint)
    else {
        // No credential yet. The account worker pairs in the background, so look again soon.
        return;
    };
    let device = crate::device_name();
    let started = Instant::now();
    let connected = tokio::select! {
        _ = stop.notified() => return,
        connected = Connected::reconnect(saved, &device) => connected,
    };
    let Ok(mut conn) = connected else { return };
    connlog::note(&connlog::short_label(&conn.computer.id), "preview", "connected", started.elapsed().as_millis() as u64);
    if !conn.caps.iter().any(|cap| cap == "thumb") {
        unsupported.store(true, Ordering::Release);
        return;
    }
    if control::send(&mut conn.socket, json!({"t":"thumb.sub","on":true})).await.is_err() {
        return;
    }
    let mut offer = handoffs().offer(&target.key);
    let request = loop {
        tokio::select! {
            request = offer.requests.recv() => break request,
            _ = stop.notified() => {
                let off = control::send(&mut conn.socket, json!({"t":"thumb.sub","on":false}));
                let _ = tokio::time::timeout(Duration::from_secs(1), off).await;
                let _ = tokio::time::timeout(Duration::from_secs(1), conn.socket.close(None)).await;
                return;
            }
            message = control::next_incoming(&mut conn.socket) => match message {
                Ok(Incoming::Binary(frame)) => {
                    if let Some(image) = control::thumb_jpeg(&frame).and_then(decode) {
                        if live.load(Ordering::Acquire) {
                            publish(&pending, &ctx, &target.id, Some(image));
                        }
                    }
                }
                Ok(Incoming::Text(_)) => {}
                Err(_) => return,
            },
        }
    };
    // The Files page wants this connection: stop the thumbnails, then give it over.
    drop(offer);
    if let Some(reply) = request {
        if control::send(&mut conn.socket, json!({"t":"thumb.sub","on":false})).await.is_ok() {
            handed.store(true, Ordering::Release);
            let _ = reply.send(conn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let rgb = vec![90u8; (w * h * 3) as usize];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 60)
            .encode(&rgb, w, h, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }
    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn crops_to_sixteen_by_nine_without_stretching() {
        assert_eq!(cover_uv(320, 180), egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)));
        let wide = cover_uv(640, 180);
        assert!((wide.width() - 0.5).abs() < 1e-6 && wide.height() == 1. && wide.min.x == 0.25);
        let tall = cover_uv(320, 360);
        assert!(tall.width() == 1. && (tall.height() - 320. / 360. * 9. / 16.).abs() < 1e-6 && (tall.min.y - (1. - tall.height()) / 2.).abs() < 1e-6);
    }
    #[test]
    fn decodes_a_host_sized_thumbnail() {
        let image = decode(&jpeg(320, 180)).unwrap();
        assert_eq!(image.size, [320, 180]);
        assert_eq!(image.pixels.len(), 320 * 180);
    }
    #[test]
    fn rejects_garbage_and_oversized_pictures() {
        assert!(decode(&[]).is_none());
        assert!(decode(&[0xFF, 0xD8, 0xFF]).is_none());
        assert!(decode(b"not a picture").is_none());
        assert!(decode(&jpeg(1300, 16)).is_none());
    }
    #[test]
    fn starts_missing_and_stops_unwanted_previews() {
        let (start, stop) = plan(&names(&["a", "b"]), &[("b".into(), false), ("c".into(), false)]);
        assert_eq!((start, stop), (names(&["a"]), names(&["c"])));
    }
    #[test]
    fn a_stopping_preview_is_not_stopped_twice_or_restarted_early() {
        let (start, stop) = plan(&names(&["a"]), &[("a".into(), true), ("c".into(), true)]);
        assert!(start.is_empty() && stop.is_empty());
    }
    #[test]
    fn nothing_is_wanted_when_signed_out() {
        let (start, stop) = plan(&[], &[("a".into(), false)]);
        assert!(start.is_empty());
        assert_eq!(stop, names(&["a"]));
    }
    #[test]
    fn cache_replaces_frames_and_drops_gone_ones() {
        let mut cache = Cache::<u32>::default();
        cache.apply(vec![("a".into(), Some(1)), ("b".into(), Some(2))], |_, n| n);
        cache.apply(vec![("a".into(), Some(3)), ("b".into(), None::<u32>)], |_, n| n * 10);
        assert_eq!(cache.get("a"), Some(&30));
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.len(), 1);
    }
    #[test]
    fn cache_keeps_only_what_is_still_shown() {
        let mut cache = Cache::<u32>::default();
        cache.apply(vec![("a".into(), Some(1)), ("b".into(), Some(2))], |_, n| n);
        cache.retain(|id| id == "b");
        assert_eq!((cache.get("a"), cache.get("b")), (None, Some(&2)));
    }
    #[test]
    fn newest_frame_wins_in_the_pending_slot() {
        let previews = Previews { wanted: None, pending: Pending::default() };
        let ctx = egui::Context::default();
        publish(&previews.pending, &ctx, "a", Some(ColorImage::example()));
        publish(&previews.pending, &ctx, "a", None);
        let taken = previews.take();
        assert_eq!(taken.len(), 1);
        assert!(taken[0].1.is_none());
        assert!(previews.take().is_empty());
    }
}
