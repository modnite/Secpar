//! Viewers of one screen share its capture and encoder, as on Windows hosts: a laptop's GPU runs
//! one 60 fps encode easily and three side by side poorly.
use super::*;
use secpar_host_core::guest_policy::Gate;
use secpar_host_core::limits::StreamPermit;

/// One frame's packets, shared by every viewer's sending thread.
pub(super) type Packets = Arc<Vec<(Header, Vec<u8>)>>;

/// What a pipeline streams; a joining viewer's `stream.ready` comes from it.
#[derive(Clone)]
pub(super) struct Info {
    pub codec: Codec,
    pub codec_name: String,
    pub size: (u32, u32),
    pub fps: u32,
    pub packed: bool,
}

/// One viewer of a pipeline: its own socket, keys and messages, fed by the shared encoder.
pub(super) struct Viewer {
    pub shared: Arc<Shared>,
    pub out: Outbox,
    /// Its `stream.ready`, repeated with a new size when the picture changes.
    pub ready: ServerMsg,
    pub gate: Option<Gate>,
    pub picture: Arc<AtomicU64>,
    pub resize: Arc<Mutex<Option<(u32, u32)>>>,
    /// To its sending thread, which paces the packets out.
    pub send: std::sync::mpsc::SyncSender<Packets>,
    pub announced: ((u32, u32), bool),
    /// Frames go out from the first key frame on: a viewer joining mid-stream can't decode before it.
    pub started: bool,
    pub key_asked: bool,
    pub _permit: StreamPermit,
}

impl Viewer {
    fn peer(&self) -> bool {
        self.shared.peer.lock().unwrap().is_some()
    }

    fn keep(&self) -> bool {
        if self.shared.stop.load(Ordering::Relaxed) {
            return false;
        }
        if self.gate.as_ref().is_some_and(|g| !g.view()) {
            self.end();
            return false;
        }
        true
    }

    fn end(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.send(header(Type::Bye, 0), &[]);
    }

    /// A `stream.ready` with the picture's size when it differs from the last one this viewer got.
    fn announce(&mut self, size: (u32, u32), packed: bool) {
        if self.announced == (size, packed) {
            return;
        }
        self.announced = (size, packed);
        self.picture.store(pack_size(size.0, size.1), Ordering::Relaxed);
        if let ServerMsg::StreamReady { codec, fps, port, session, key, cands, .. } = &self.ready {
            self.out.send(ServerMsg::StreamReady { chroma: packed.then(|| "444".into()), codec: codec.clone(), w: size.0, h: size.1, fps: *fps, port: *port, session: *session, key: key.clone(), cands: cands.clone() });
        }
    }

    fn deliver(&mut self, packets: &Packets, key: bool) {
        if !self.peer() || !(self.started || key) {
            return;
        }
        self.started = true;
        // Full only when its sending is frames behind; the client asks for what it missed.
        let _ = self.send.try_send(packets.clone());
    }
}

/// One frame's packets over about 4 ms, so a burst doesn't overflow Wi-Fi queues.
pub(super) fn pace(shared: &Shared, packets: &[(Header, Vec<u8>)]) {
    let count = packets.len().div_ceil(4).max(1);
    let start = Instant::now();
    for (index, chunk) in packets.chunks(count).enumerate() {
        if let Some(wait) = (start + Duration::from_millis(index as u64)).checked_duration_since(Instant::now()) {
            std::thread::sleep(wait)
        }
        for (h, p) in chunk {
            shared.send(*h, p)
        }
    }
}

pub(super) struct Group {
    display: String,
    info: Mutex<Info>,
    /// The first viewer's screen-sharing session, which the capture reads from.
    portal: Arc<Portal>,
    /// That viewer left while others watched: the pipeline closes the session when it ends.
    orphaned: AtomicBool,
    joining: Mutex<Vec<Viewer>>,
    closed: AtomicBool,
}

static GROUPS: Mutex<Vec<Arc<Group>>> = Mutex::new(Vec::new());

/// The open pipeline a viewer of `display` can join: a codec it decodes, 4:4:4 only if it shows it.
/// A new screen pick ("new") always gets its own.
pub(super) fn find(display: &str, codecs: &[String], chroma444: bool) -> Option<(Arc<Group>, Info)> {
    if display == "new" {
        return None;
    }
    GROUPS.lock().unwrap().iter().find_map(|g| {
        let info = g.info.lock().unwrap().clone();
        let fits = !g.closed.load(Ordering::Relaxed)
            && g.display == display
            && codecs.iter().any(|c| c.eq_ignore_ascii_case(&info.codec_name))
            && (!info.packed || chroma444);
        fits.then(|| (g.clone(), info))
    })
}

/// A stream ending: true when its screen-sharing session still feeds a pipeline, which then closes
/// it when the pipeline ends.
pub(super) fn keeps_portal(portal: &Arc<Portal>) -> bool {
    let groups = GROUPS.lock().unwrap();
    match groups.iter().find(|g| Arc::ptr_eq(&g.portal, portal) && !g.closed.load(Ordering::Relaxed)) {
        Some(g) => {
            g.orphaned.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

impl Group {
    /// Hands the viewer back when the pipeline closed in the meantime.
    pub(super) fn add(&self, v: Viewer) -> Result<(), Viewer> {
        let mut joining = self.joining.lock().unwrap();
        if self.closed.load(Ordering::Relaxed) {
            return Err(v);
        }
        joining.push(v);
        Ok(())
    }

    /// Takes the pipeline off the list. Without `force`, only while nobody is joining.
    fn close(&self, force: bool) -> bool {
        let mut groups = GROUPS.lock().unwrap();
        let mut joining = self.joining.lock().unwrap();
        if !force && !joining.is_empty() {
            return false;
        }
        self.closed.store(true, Ordering::Relaxed);
        groups.retain(|g| !std::ptr::eq(g.as_ref(), self));
        for v in joining.drain(..) {
            v.end();
        }
        true
    }
}

/// Starts the pipeline for its first viewer on its own thread.
pub(super) fn lead(mut capture: Capture, display: String, info: Info, portal: Arc<Portal>, first: Viewer) {
    let group = Arc::new(Group { display, info: Mutex::new(info.clone()), portal, orphaned: AtomicBool::new(false), joining: Mutex::default(), closed: AtomicBool::new(false) });
    GROUPS.lock().unwrap().push(group.clone());
    let runtime = tokio::runtime::Handle::try_current().ok();
    std::thread::spawn(move || {
        let mut viewers = vec![first];
        // A panic in here must end these streams and say why. It would otherwise end only this
        // thread and leave the sessions open with a picture that never moves.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (mut id, mut kbps, mut key) = (0u32, 0u32, true);
            loop {
                let joined = std::mem::take(&mut *group.joining.lock().unwrap());
                for mut v in joined {
                    v.announce(capture.size(), capture.packed());
                    viewers.push(v);
                    eprintln!("secpar: a viewer joined the shared stream ({} watching)", viewers.len());
                }
                viewers.retain(Viewer::keep);
                if viewers.is_empty() {
                    if group.close(false) {
                        return None;
                    }
                    continue;
                }
                for v in &mut viewers {
                    // A viewer starts on a key frame: the first one after its address is known.
                    if !v.started && !v.key_asked && v.peer() {
                        v.key_asked = true;
                        key = true;
                    }
                }
                if !viewers.iter().any(Viewer::peer) {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                // A shared picture keeps its size until one viewer has it alone.
                if viewers.len() == 1 {
                    let wanted = viewers[0].resize.lock().unwrap().take();
                    if let Some(wanted) = wanted {
                        // the encoder renegotiates on the new size and opens with a keyframe
                        if let Some((w, h)) = capture.set_size(wanted) {
                            eprintln!("secpar: stream is now {w}x{h} (asked for {}x{})", wanted.0, wanted.1);
                            let mut i = group.info.lock().unwrap();
                            (i.size, i.packed) = ((w, h), capture.packed());
                            drop(i);
                            viewers[0].announce((w, h), capture.packed());
                            key = true;
                        }
                    }
                }
                // The lowest bitrate any viewer's link takes.
                let wanted = viewers.iter().map(|v| v.shared.kbps.load(Ordering::Relaxed)).min().unwrap_or(kbps);
                if wanted != kbps {
                    kbps = wanted;
                    capture.set_bitrate(kbps);
                    eprintln!("secpar: bitrate {kbps} kbps");
                }
                for v in &viewers {
                    key |= v.shared.idr.swap(false, Ordering::Relaxed);
                }
                if std::mem::take(&mut key) {
                    capture.request_keyframe();
                }
                let frame = match capture.next_frame(Duration::from_millis(100)) {
                    Ok(Some(f)) => f,
                    Ok(None) => continue,
                    Err(error) => return Some(error),
                };
                // A friend never follows the owner onto the lock screen.
                let locked = viewers.iter().any(|v| v.gate.is_some()) && crate::lock::locked_or_unknown();
                for v in &viewers {
                    if let Some(g) = v.gate.as_ref().filter(|g| locked || !g.view()) {
                        g.close_because("locked");
                    }
                }
                let fh = FrameHeader { len: frame.data.len() as u32, capture_us: frame.capture_us, keyframe: frame.key, codec: info.codec };
                let packets: Packets = Arc::new(packetize(id, &fh, &frame.data, 15));
                for v in viewers.iter_mut().filter(|v| v.gate.as_ref().is_none_or(|g| g.view())) {
                    v.deliver(&packets, frame.key);
                }
                id = id.wrapping_add(1);
            }
        }));
        let why = match outcome {
            Ok(error) => error,
            Err(_) => Some("Desktop capture stopped unexpectedly.".to_string()),
        };
        group.close(true);
        for v in &viewers {
            if let Some(why) = &why {
                v.out.send(ServerMsg::error("capture_failed", why, Some("stream.start")));
            }
            v.end();
        }
        drop(viewers);
        drop(capture);
        if group.orphaned.load(Ordering::Relaxed) {
            let portal = group.portal.clone();
            if let Some(rt) = runtime {
                rt.spawn(async move {
                    let _ = portal.close().await;
                });
            }
        }
    });
}
