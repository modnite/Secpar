//! Desktop preview for the phone's home screen: the primary monitor as a 320 px wide JPEG
//! (`[0x02][JPEG]`, quality 60) once on subscribe and then every 5 s, like the Windows host.
//!
//! Wayland offers no screenshot without the user's consent, so the preview reuses the screen
//! sharing permission that the first stream saved (the portal restore token). With no saved
//! permission nothing is sent and nothing is asked. While a stream runs the preview pauses, and
//! the capture session is closed first so the stream has the screen to itself.
use crate::push::Push;
use secpar_host_core::agent::BoxFut;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub const INTERVAL: Duration = Duration::from_secs(5);
pub const WIDTH: u32 = 320;
/// A frame this young is handed to a phone that subscribes while another phone keeps the loop running.
const FRESH: Duration = Duration::from_secs(10);

/// The binary frame the phone expects.
pub fn frame(jpeg: Vec<u8>) -> Vec<u8> {
    let mut f = Vec::with_capacity(1 + jpeg.len());
    f.push(2);
    f.extend(jpeg);
    f
}

/// An open capture session that can produce a JPEG of the screen. Blocking.
pub trait Snapshot: Send {
    fn jpeg(&mut self) -> Option<Vec<u8>>;
}

pub enum Open {
    Ready(Box<dyn Snapshot>),
    /// No saved screen-sharing permission yet: nothing to do until a stream creates one.
    Unavailable,
    /// The session could not be opened. It is not tried again until the next subscription.
    Failed,
}

pub trait Source: Send + Sync + 'static {
    fn open(&self) -> BoxFut<Open>;
}

struct State<P> {
    subs: Vec<(u64, P)>,
    running: bool,
    last: Option<(Instant, Vec<u8>)>,
}

struct Inner<P: Push> {
    source: Arc<dyn Source>,
    /// True while a stream owns the screen.
    busy: Box<dyn Fn() -> bool + Send + Sync>,
    interval: Duration,
    state: Mutex<State<P>>,
    capture: tokio::sync::Mutex<Option<Box<dyn Snapshot>>>,
    failed: AtomicBool,
    next: AtomicU64,
    /// Wakes the loop when a phone leaves, so the last one to go closes the capture at once.
    wake: tokio::sync::Notify,
}

/// One preview loop for the whole host, shared by every subscribed phone.
pub struct Thumbs<P: Push>(Arc<Inner<P>>);

/// Dropping it unsubscribes the phone.
pub struct Subscription<P: Push> {
    inner: Arc<Inner<P>>,
    id: u64,
}

impl<P: Push> Drop for Subscription<P> {
    fn drop(&mut self) {
        self.inner.state.lock().unwrap().subs.retain(|(id, _)| *id != self.id);
        self.inner.wake.notify_one();
    }
}

impl<P: Push> Thumbs<P> {
    pub fn new(source: Arc<dyn Source>, busy: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self::with_interval(source, busy, INTERVAL)
    }

    fn with_interval(source: Arc<dyn Source>, busy: impl Fn() -> bool + Send + Sync + 'static, interval: Duration) -> Self {
        Thumbs(Arc::new(Inner {
            source,
            busy: Box::new(busy),
            interval,
            state: Mutex::new(State { subs: vec![], running: false, last: None }),
            capture: tokio::sync::Mutex::new(None),
            failed: AtomicBool::new(false),
            next: AtomicU64::new(1),
            wake: tokio::sync::Notify::new(),
        }))
    }

    /// Starts sending previews to `out` until the subscription is dropped.
    pub fn subscribe(&self, out: P) -> Subscription<P> {
        let inner = self.0.clone();
        let id = inner.next.fetch_add(1, Ordering::Relaxed);
        let (fresh, start) = {
            let mut state = inner.state.lock().unwrap();
            let fresh = state.last.as_ref().filter(|(at, _)| at.elapsed() < FRESH).map(|(_, f)| f.clone());
            state.subs.push((id, out.clone()));
            let start = !state.running;
            state.running = true;
            (fresh, start)
        };
        if let Some(f) = fresh {
            tokio::spawn(async move {
                out.bin(f).await;
            });
        }
        if start {
            inner.failed.store(false, Ordering::Relaxed);
            tokio::spawn(run(inner.clone()));
        }
        Subscription { inner, id }
    }

    /// Closes the capture session, waiting for a grab in progress. Called before a stream starts.
    pub async fn release(&self) {
        self.0.capture.lock().await.take();
    }
}

/// Marks the loop as stopped when nobody is subscribed. Decided under the same lock as `subscribe`.
fn no_subscribers<P: Push>(inner: &Inner<P>) -> bool {
    let mut state = inner.state.lock().unwrap();
    if state.subs.is_empty() {
        state.running = false;
    }
    !state.running
}

async fn run<P: Push>(inner: Arc<Inner<P>>) {
    loop {
        tick(&inner).await;
        let deadline = tokio::time::Instant::now() + inner.interval;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                _ = inner.wake.notified() => {
                    if no_subscribers(&inner) {
                        inner.capture.lock().await.take();
                        return;
                    }
                }
            }
        }
        if no_subscribers(&inner) {
            inner.capture.lock().await.take();
            return;
        }
    }
}

async fn tick<P: Push>(inner: &Arc<Inner<P>>) {
    if (inner.busy)() {
        inner.capture.lock().await.take();
        return;
    }
    let mut slot = inner.capture.lock().await;
    if slot.is_none() {
        if inner.failed.load(Ordering::Relaxed) {
            return;
        }
        match inner.source.open().await {
            Open::Ready(capture) => *slot = Some(capture),
            Open::Unavailable => return,
            Open::Failed => {
                inner.failed.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
    let Some(mut capture) = slot.take() else { return };
    let Ok((capture, jpeg)) = tokio::task::spawn_blocking(move || {
        let jpeg = capture.jpeg();
        (capture, jpeg)
    })
    .await
    else {
        return;
    };
    *slot = Some(capture);
    drop(slot);
    let Some(jpeg) = jpeg else { return };
    let frame = frame(jpeg);
    let subs: Vec<(u64, P)> = {
        let mut state = inner.state.lock().unwrap();
        state.last = Some((Instant::now(), frame.clone()));
        state.subs.clone()
    };
    for (id, out) in subs {
        if !out.bin(frame.clone()).await {
            inner.state.lock().unwrap().subs.retain(|(i, _)| *i != id);
        }
    }
}

// ---- Real capture: the saved portal permission and a small GStreamer pipeline ----

use crate::{portal::Portal, store::Store};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::AppSink;

/// 320 px wide with the screen aspect ratio. A width alone would leave the height unscaled.
pub fn scaled_size(w: u32, h: u32) -> (u32, u32) {
    let h = if w == 0 { 1 } else { ((h as f64 * WIDTH as f64 / w as f64).round() as u32).max(1) };
    (WIDTH, h)
}

/// PipeWire frames, one every two seconds, scaled to 320 px wide and JPEG encoded.
/// The rate limit comes first so conversion and encoding run on a handful of frames only.
pub fn pipeline_text(source: &str, native: (u32, u32)) -> String {
    let (w, h) = scaled_size(native.0, native.1);
    format!(
        "{source} ! queue leaky=downstream max-size-buffers=1 max-size-bytes=0 max-size-time=0 ! \
         videorate drop-only=true ! video/x-raw,framerate=1/2 ! videoconvert ! videoscale ! \
         video/x-raw,width={w},height={h} ! jpegenc quality=60 ! \
         appsink name=thumb sync=false max-buffers=1 drop=true enable-last-sample=false"
    )
}

/// The preview needs the PipeWire source and the JPEG encoder (gst-plugins-good).
pub fn available() -> bool {
    // The first lookup loads the GStreamer plugin registry, so it is done once per process.
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        gst::init().is_ok() && ["pipewiresrc", "jpegenc", "videorate", "videoscale", "videoconvert"].iter().all(|f| gst::ElementFactory::find(f).is_some())
    })
}

pub struct PipelineSnapshot {
    pipeline: gst::Pipeline,
    sink: AppSink,
    _portal: Option<Arc<Portal>>,
}

impl PipelineSnapshot {
    /// Starts `source` (any GStreamer source description) and waits for the first JPEG.
    pub fn start(source: &str, native: (u32, u32), portal: Option<Arc<Portal>>) -> Result<Self, String> {
        gst::init().map_err(|_| "Could not initialize GStreamer.")?;
        let pipeline = gst::parse::launch(&pipeline_text(source, native))
            .map_err(|e| e.to_string())?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "Invalid preview pipeline")?;
        let sink = pipeline.by_name("thumb").and_then(|e| e.downcast::<AppSink>().ok()).ok_or("Missing preview output")?;
        if pipeline.set_state(gst::State::Playing).is_err() {
            let _ = pipeline.set_state(gst::State::Null);
            return Err("Could not start the preview".into());
        }
        let snapshot = PipelineSnapshot { pipeline, sink, _portal: portal };
        // The portal sends frames only when the screen changes. `keepalive-time` repeats the last one.
        match snapshot.pull(Duration::from_secs(6)) {
            Some(_) => Ok(snapshot),
            None => Err("No preview frame arrived".into()),
        }
    }

    /// The newest JPEG: frames that piled up are skipped, else wait up to `wait` for the next.
    fn pull(&self, wait: Duration) -> Option<Vec<u8>> {
        let mut newest = None;
        while let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::ZERO) {
            newest = Some(sample);
        }
        let sample = newest.or_else(|| self.sink.try_pull_sample(gst::ClockTime::from_nseconds(wait.as_nanos() as u64)))?;
        let map = sample.buffer()?.map_readable().ok()?;
        let bytes = map.as_slice();
        (bytes.starts_with(&[0xff, 0xd8])).then(|| bytes.to_vec())
    }
}

impl Snapshot for PipelineSnapshot {
    fn jpeg(&mut self) -> Option<Vec<u8>> {
        self.pull(Duration::from_secs(3))
    }
}

impl Drop for PipelineSnapshot {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// The screen the user already allowed for streaming. Needs a stored restore token.
pub struct PortalSource {
    pub store: Arc<Mutex<Store>>,
}

impl Source for PortalSource {
    fn open(&self) -> BoxFut<Open> {
        let store = self.store.clone();
        Box::pin(async move {
            let Some(token) = store.lock().unwrap().portal_token() else { return Open::Unavailable };
            let Ok(portal) = Portal::open(Some(&token)).await else { return Open::Failed };
            if let Some(new) = portal.restore_token.clone() {
                let _ = store.lock().unwrap().set_portal_token(Some(new));
            }
            let base = format!("pipewiresrc fd={} path={} do-timestamp=true", portal.fd(), portal.node_id);
            let (p, native) = (portal.clone(), (portal.width, portal.height));
            let started = tokio::task::spawn_blocking(move || {
                // Copies first (always works), then the plain source.
                // Pinned to the display's size: a small preview size must not travel up to the compositor, which
                // on a virtual output would shrink the whole screen to it (see `capture::source_variants`).
                let fixed = crate::capture::pinned(native);
                // A scaled display reports a logical size that is not the pixel size, so unpinned sources follow.
                let mut sources = vec![format!("{base} always-copy=true keepalive-time=2000 ! video/x-raw{fixed}"), format!("{base} keepalive-time=2000 ! video/x-raw(ANY){fixed}")];
                if !fixed.is_empty() {
                    sources.push(format!("{base} always-copy=true keepalive-time=2000 ! video/x-raw"));
                    sources.push(format!("{base} keepalive-time=2000"));
                }
                sources
                    .iter()
                    .find_map(|source| PipelineSnapshot::start(source, native, Some(p.clone())).ok())
            })
            .await;
            match started {
                Ok(Some(snapshot)) => Open::Ready(Box::new(snapshot)),
                _ => {
                    portal.close().await;
                    Open::Failed
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::fake::Fake;
    use std::sync::atomic::AtomicUsize;

    struct Shot {
        n: Arc<AtomicUsize>,
        alive: Arc<AtomicUsize>,
    }
    impl Snapshot for Shot {
        fn jpeg(&mut self) -> Option<Vec<u8>> {
            Some(vec![0xff, 0xd8, self.n.fetch_add(1, Ordering::SeqCst) as u8, 0xff, 0xd9])
        }
    }
    impl Drop for Shot {
        fn drop(&mut self) {
            self.alive.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct Fakesource {
        opens: Arc<AtomicUsize>,
        shots: Arc<AtomicUsize>,
        alive: Arc<AtomicUsize>,
        mode: Mutex<Vec<&'static str>>,
    }
    impl Fakesource {
        fn new(mode: Vec<&'static str>) -> Arc<Self> {
            Arc::new(Fakesource { opens: Default::default(), shots: Default::default(), alive: Default::default(), mode: Mutex::new(mode) })
        }
    }
    impl Source for Fakesource {
        fn open(&self) -> BoxFut<Open> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            let mode = { let mut m = self.mode.lock().unwrap(); if m.len() > 1 { m.remove(0) } else { m[0] } };
            let (n, alive) = (self.shots.clone(), self.alive.clone());
            Box::pin(async move {
                match mode {
                    "ready" => {
                        alive.fetch_add(1, Ordering::SeqCst);
                        Open::Ready(Box::new(Shot { n, alive }))
                    }
                    "none" => Open::Unavailable,
                    _ => Open::Failed,
                }
            })
        }
    }

    const TICK: Duration = Duration::from_millis(30);

    async fn until(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..300 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn frames_carry_the_thumbnail_tag() {
        assert_eq!(frame(vec![0xff, 0xd8, 0xff, 0xd9]), vec![2, 0xff, 0xd8, 0xff, 0xd9]);
    }

    #[test]
    fn scaling_keeps_the_aspect_ratio_like_the_windows_host() {
        assert_eq!(scaled_size(2560, 1440), (320, 180));
        assert_eq!(scaled_size(1920, 1200), (320, 200));
        assert_eq!(scaled_size(1080, 1920), (320, 569));
        assert_eq!(scaled_size(200, 100), (320, 160));
        assert_eq!(scaled_size(1366, 768), (320, 180));
        assert_eq!(scaled_size(0, 0), (320, 1));
    }

    #[test]
    fn pipeline_is_rate_limited_before_conversion_and_scaled_to_320() {
        let text = pipeline_text("pipewiresrc fd=3 path=7", (1920, 1080));
        assert!(text.find("videorate").unwrap() < text.find("videoconvert").unwrap());
        assert!(text.find("framerate=1/2").unwrap() < text.find("jpegenc").unwrap());
        assert!(text.contains("width=320,height=180") && text.contains("jpegenc quality=60"));
        assert!(text.contains("max-buffers=1 drop=true"));
    }

    #[test]
    fn synthetic_screen_becomes_a_320_wide_jpeg() {
        let mut shot = PipelineSnapshot::start("videotestsrc is-live=true pattern=ball ! video/x-raw,width=1280,height=720,framerate=30/1", (1280, 720), None)
            .expect("jpegenc from gst-plugins-good is needed for the preview");
        let jpeg = shot.jpeg().expect("a frame");
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xff, 0xd9]);
        // the JPEG start-of-frame segment holds height then width
        let sof = jpeg.windows(2).position(|w| w == [0xff, 0xc0]).expect("baseline SOF");
        let (h, w) = (u16::from_be_bytes([jpeg[sof + 5], jpeg[sof + 6]]), u16::from_be_bytes([jpeg[sof + 7], jpeg[sof + 8]]));
        assert_eq!((w, h), (320, 180), "16:9 stays 16:9");
    }

    #[tokio::test]
    async fn sends_one_frame_at_once_then_one_per_interval() {
        let source = Fakesource::new(vec!["ready"]);
        let thumbs = Thumbs::with_interval(source.clone(), || false, TICK);
        let out = Fake::default();
        let _sub = thumbs.subscribe(out.clone());
        until("first frame", || !out.bins.lock().unwrap().is_empty()).await;
        assert_eq!(&out.bins.lock().unwrap()[0][..3], &[2, 0xff, 0xd8]);
        until("more frames", || out.bins.lock().unwrap().len() >= 3).await;
        assert_eq!(source.opens.load(Ordering::SeqCst), 1, "one capture session serves every tick");
    }

    #[tokio::test]
    async fn phones_share_the_loop_and_a_late_phone_gets_the_latest_frame_at_once() {
        let source = Fakesource::new(vec!["ready"]);
        let thumbs = Thumbs::with_interval(source.clone(), || false, Duration::from_secs(60));
        let first = Fake::default();
        let _a = thumbs.subscribe(first.clone());
        until("first frame", || first.bins.lock().unwrap().len() == 1).await;
        let late = Fake::default();
        let _b = thumbs.subscribe(late.clone());
        until("cached frame", || late.bins.lock().unwrap().len() == 1).await;
        assert_eq!(late.bins.lock().unwrap()[0], first.bins.lock().unwrap()[0]);
        assert_eq!(source.opens.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn without_a_saved_permission_nothing_is_sent_and_it_is_retried_later() {
        let source = Fakesource::new(vec!["none", "none", "none", "ready"]);
        let thumbs = Thumbs::with_interval(source.clone(), || false, TICK);
        let out = Fake::default();
        let _sub = thumbs.subscribe(out.clone());
        until("three tries", || source.opens.load(Ordering::SeqCst) >= 3).await;
        assert!(out.bins.lock().unwrap().is_empty());
        until("frame once a permission exists", || !out.bins.lock().unwrap().is_empty()).await;
    }

    #[tokio::test]
    async fn a_failed_session_is_not_retried_until_the_next_subscription() {
        let source = Fakesource::new(vec!["fail"]);
        let thumbs = Thumbs::with_interval(source.clone(), || false, TICK);
        let out = Fake::default();
        let sub = thumbs.subscribe(out.clone());
        until("one try", || source.opens.load(Ordering::SeqCst) == 1).await;
        tokio::time::sleep(TICK * 6).await;
        assert_eq!(source.opens.load(Ordering::SeqCst), 1, "a failing session can prompt the user, so it is tried once");
        drop(sub);
        until("loop stopped", || !thumbs.0.state.lock().unwrap().running).await;
        let _again = thumbs.subscribe(out.clone());
        until("second try", || source.opens.load(Ordering::SeqCst) == 2).await;
    }

    #[tokio::test]
    async fn a_running_stream_pauses_the_preview_and_frees_the_screen() {
        let source = Fakesource::new(vec!["ready"]);
        let streaming = Arc::new(AtomicBool::new(false));
        let flag = streaming.clone();
        let thumbs = Thumbs::with_interval(source.clone(), move || flag.load(Ordering::SeqCst), TICK);
        let out = Fake::default();
        let _sub = thumbs.subscribe(out.clone());
        until("frames", || out.bins.lock().unwrap().len() >= 2).await;
        assert_eq!(source.alive.load(Ordering::SeqCst), 1);
        streaming.store(true, Ordering::SeqCst);
        until("capture closed", || source.alive.load(Ordering::SeqCst) == 0).await;
        let sent = out.bins.lock().unwrap().len();
        tokio::time::sleep(TICK * 5).await;
        assert_eq!(out.bins.lock().unwrap().len(), sent, "no frames while streaming");
        streaming.store(false, Ordering::SeqCst);
        until("resumes", || out.bins.lock().unwrap().len() > sent).await;
    }

    #[tokio::test]
    async fn release_closes_the_session_and_the_last_unsubscribe_stops_the_loop() {
        let source = Fakesource::new(vec!["ready"]);
        let thumbs = Thumbs::with_interval(source.clone(), || false, Duration::from_secs(60));
        let out = Fake::default();
        let sub = thumbs.subscribe(out.clone());
        until("open", || source.alive.load(Ordering::SeqCst) == 1).await;
        thumbs.release().await;
        assert_eq!(source.alive.load(Ordering::SeqCst), 0);
        drop(sub);
        until("loop stopped", || !thumbs.0.state.lock().unwrap().running).await;
        assert_eq!(source.alive.load(Ordering::SeqCst), 0, "no capture session without a subscriber");
    }

    #[tokio::test]
    async fn a_phone_that_disconnects_is_dropped_from_the_loop() {
        let source = Fakesource::new(vec!["ready"]);
        let thumbs = Thumbs::with_interval(source.clone(), || false, TICK);
        let (gone, stays) = (Fake::default(), Fake::default());
        let _a = thumbs.subscribe(gone.clone());
        let _b = thumbs.subscribe(stays.clone());
        until("frames", || stays.bins.lock().unwrap().len() >= 1).await;
        gone.close();
        until("dropped", || thumbs.0.state.lock().unwrap().subs.len() == 1).await;
        until("others keep receiving", || stays.bins.lock().unwrap().len() >= 3).await;
    }
}
