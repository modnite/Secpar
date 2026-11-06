//! Owner-authenticated PRS media transport (same wire format as the Linux host).
//! Capture, encoding and input are supplied through traits so the transport is
//! tested without a Mac; `native` provides the real sources.
use crate::platform::{Sink, StreamHandle};
use secpar_host_core::{
    audio::AudioFrames,
    limits::StreamPermit,
    proto::{ClientMsg, ServerMsg},
    Display,
};
use secpar_host_linux::store::random;
use secpar_stream::{
    frame::{packetize, Codec, FrameHeader},
    Crypt, Dir, Header, Type,
};
use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

/// Monotonic microseconds. Sources must stamp `capture_us` with this clock.
pub fn now_us() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_micros() as u64 + 1
}
/// One complete Annex-B access unit.
pub struct Frame {
    pub data: Vec<u8>,
    pub key: bool,
    pub capture_us: u64,
}
pub trait VideoSource: Send {
    fn size(&self) -> (u32, u32);
    fn fps(&self) -> u32;
    fn codec(&self) -> &'static str;
    fn request_keyframe(&mut self);
    /// Ok(None) on timeout; Err ends the stream.
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>, String>;
}
/// Interleaved stereo f32 at 48 kHz.
pub struct AudioChunk {
    pub pcm: Vec<f32>,
    pub capture_us: u64,
}
pub trait AudioSource: Send {
    fn next(&mut self, timeout: Duration) -> Result<Option<AudioChunk>, String>;
}
pub trait InputSink: Send {
    fn apply(&mut self, message: ClientMsg);
    /// Release every key and button this session still holds.
    fn release_all(&mut self);
}
pub struct Sources {
    pub video: Box<dyn VideoSource>,
    pub audio: Option<Box<dyn AudioSource>>,
    pub input: Option<Box<dyn InputSink>>,
    pub display: Display,
}

fn header(typ: Type, frame_id: u32) -> Header {
    Header { typ, frame_id, flags: 0, k: 0, seq: 0, frag: 0, count: 0 }
}
struct Shared {
    socket: UdpSocket,
    crypt: Crypt,
    peer: Mutex<Option<SocketAddr>>,
    seq: AtomicU32,
    stop: AtomicBool,
    idr: AtomicBool,
}
impl Shared {
    fn send(&self, mut h: Header, plain: &[u8]) {
        if let Some(peer) = *self.peer.lock().unwrap() {
            h.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            let _ = self.socket.send_to(&self.crypt.seal(Dir::HostToPhone, &h, plain), peer);
        }
    }
    fn probe_to(&self, peer: SocketAddr) {
        let mut h = header(Type::Probe, 0);
        h.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut p = [0; 16];
        p[..8].copy_from_slice(&now_us().to_be_bytes());
        let _ = self.socket.send_to(&self.crypt.seal(Dir::HostToPhone, &h, &p), peer);
    }
}
pub struct MediaStream {
    shared: Arc<Shared>,
    input: Option<mpsc::SyncSender<ClientMsg>>,
}
impl MediaStream {
    /// Binds a UDP port and starts the receive, video, audio and input threads.
    /// `guard` is released when the video thread ends, after capture is dropped.
    pub fn start(
        sources: Sources,
        candidates: Vec<SocketAddr>,
        addresses: Vec<String>,
        out: Arc<dyn Sink>,
        guard: StreamPermit,
    ) -> Result<(Self, ServerMsg), String> {
        let Sources { video: mut capture, audio, input, display } = sources;
        let (width, height) = capture.size();
        let fps = capture.fps();
        let codec_name = capture.codec().to_string();
        let codec = match codec_name.as_str() {
            "h264" => Codec::H264,
            "hevc" => Codec::Hevc,
            _ => return Err("Unsupported negotiated codec".into()),
        };
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|_| "Could not open streaming socket")?;
        socket
            .set_read_timeout(Some(Duration::from_millis(25)))
            .map_err(|_| "Could not configure streaming socket")?;
        let port = socket.local_addr().map_err(|_| "Could not read stream port")?.port();
        let key = random::<32>();
        let session = u32::from_be_bytes(random::<4>());
        let shared = Arc::new(Shared {
            socket,
            crypt: Crypt::new(&key, session),
            peer: Mutex::new(None),
            seq: AtomicU32::new(0),
            stop: AtomicBool::new(false),
            idr: AtomicBool::new(true),
        });
        let input_tx = input.map(|mut target| {
            let (tx, rx) = mpsc::sync_channel::<ClientMsg>(128);
            let s = shared.clone();
            std::thread::spawn(move || {
                while !s.stop.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(m) => target.apply(m),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
                target.release_all();
            });
            tx
        });
        let (rx_shared, rx_input) = (shared.clone(), input_tx.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            let mut seen = None::<u64>;
            let mut last_rx = Instant::now();
            let begun = Instant::now();
            let mut last_punch = Instant::now() - Duration::from_secs(1);
            while !rx_shared.stop.load(Ordering::Relaxed) {
                if rx_shared.peer.lock().unwrap().is_none()
                    && begun.elapsed() < Duration::from_secs(10)
                    && last_punch.elapsed() > Duration::from_millis(100)
                {
                    for peer in candidates.iter().take(16) {
                        rx_shared.probe_to(*peer)
                    }
                    last_punch = Instant::now();
                }
                if begun.elapsed() > Duration::from_secs(10) && last_rx.elapsed() > Duration::from_secs(5) {
                    break;
                }
                let Ok((n, from)) = rx_shared.socket.recv_from(&mut buf) else { continue };
                let Some((h, p)) = rx_shared.crypt.open(Dir::PhoneToHost, &buf[..n]) else { continue };
                if h.typ != Type::Probe && *rx_shared.peer.lock().unwrap() != Some(from) {
                    continue;
                }
                last_rx = Instant::now();
                match h.typ {
                    Type::Probe if p.len() >= 16 => {
                        *rx_shared.peer.lock().unwrap() = Some(from);
                        let mut reply = [0u8; 16];
                        reply[..8].copy_from_slice(&now_us().to_be_bytes());
                        reply[8..].copy_from_slice(&p[..8]);
                        rx_shared.send(header(Type::Probe, 0), &reply);
                    }
                    Type::Input => {
                        for msg in dedup_inputs(&p, &mut seen) {
                            if let Some(tx) = &rx_input {
                                if tx.try_send(msg).is_err() {
                                    rx_shared.stop.store(true, Ordering::Relaxed);
                                    break;
                                }
                            }
                        }
                    }
                    Type::Idr => rx_shared.idr.store(true, Ordering::Relaxed),
                    Type::Bye => break,
                    _ => {}
                }
            }
            rx_shared.stop.store(true, Ordering::Relaxed);
        });
        let video = shared.clone();
        std::thread::spawn(move || {
            let _guard = guard;
            let mut id = 0u32;
            while !video.stop.load(Ordering::Relaxed) {
                if video.peer.lock().unwrap().is_none() {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                if video.idr.swap(false, Ordering::Relaxed) {
                    capture.request_keyframe();
                }
                let frame = match capture.next_frame(Duration::from_millis(100)) {
                    Ok(Some(f)) => f,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                let fh = FrameHeader {
                    len: frame.data.len() as u32,
                    capture_us: frame.capture_us,
                    keyframe: frame.key,
                    codec,
                };
                let packets = packetize(id, &fh, &frame.data, 15);
                let count = packets.len().div_ceil(4).max(1);
                let start = Instant::now();
                for (index, chunk) in packets.chunks(count).enumerate() {
                    if let Some(wait) = (start + Duration::from_millis(index as u64)).checked_duration_since(Instant::now()) {
                        std::thread::sleep(wait)
                    }
                    for (h, p) in chunk {
                        video.send(*h, p)
                    }
                }
                id = id.wrapping_add(1);
            }
            video.stop.store(true, Ordering::Relaxed);
            video.send(header(Type::Bye, 0), &[]);
            drop(capture);
        });
        if let Some(mut source) = audio {
            let (shared, out) = (shared.clone(), out.clone());
            std::thread::spawn(move || {
                if let Err(e) = run_audio(&mut *source, &shared) {
                    out.send(ServerMsg::error("audio_unavailable", &e, Some("stream.start")));
                }
            });
        }
        out.send(ServerMsg::StreamDisplays { current: display.id.clone(), list: vec![display] });
        let ready = ServerMsg::StreamReady {
            codec: codec_name,
            w: width,
            h: height,
            fps,
            port,
            session,
            key: base64_encode(&key),
            cands: addresses.into_iter().map(|ip| format!("{ip}:{port}")).collect(),
        };
        Ok((Self { shared, input: input_tx }, ready))
    }
}
impl StreamHandle for MediaStream {
    fn alive(&self) -> bool {
        !self.shared.stop.load(Ordering::Relaxed)
    }
    fn input(&self, m: ClientMsg) {
        if let Some(tx) = &self.input {
            if tx.try_send(m).is_err() {
                self.shared.stop.store(true, Ordering::Relaxed);
            }
        }
    }
    fn add_candidate(&self, peer: SocketAddr) {
        self.shared.probe_to(peer);
    }
}
impl Drop for MediaStream {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.send(header(Type::Bye, 0), &[]);
    }
}
fn run_audio(source: &mut dyn AudioSource, shared: &Shared) -> Result<(), String> {
    let mut encoder = opus::Encoder::new(48000, opus::Channels::Stereo, opus::Application::LowDelay)
        .map_err(|_| "Could not start Opus encoder")?;
    encoder.set_bitrate(opus::Bitrate::Bits(128000)).map_err(|_| "Could not configure Opus encoder")?;
    let mut frames = AudioFrames::default();
    let mut id = 0u32;
    while !shared.stop.load(Ordering::Relaxed) {
        let Some(chunk) = source.next(Duration::from_millis(100))? else { continue };
        frames.push(&chunk.pcm, chunk.capture_us);
        while let Some((frame, capture_us)) = frames.next_frame() {
            let mut packet = [0u8; 1100];
            let n = encoder.encode_float(&frame, &mut packet).map_err(|_| "Could not encode audio")?;
            let mut payload = capture_us.to_be_bytes().to_vec();
            payload.extend_from_slice(&packet[..n]);
            shared.send(header(Type::Audio, id), &payload);
            id = id.wrapping_add(1);
        }
    }
    Ok(())
}
fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char)
            } else {
                s.push('=')
            }
        }
    }
    s
}
/// Same filter as the Linux host: only pointer/keyboard messages, in order, no replays.
pub fn dedup_inputs(bytes: &[u8], last: &mut Option<u64>) -> Vec<ClientMsg> {
    let Ok(items) = serde_json::from_slice::<Vec<serde_json::Value>>(bytes) else { return vec![] };
    let mut result: Vec<_> = items
        .into_iter()
        .take(64)
        .filter_map(|v| {
            let id = v["i"].as_u64()?;
            let msg = serde_json::from_value::<ClientMsg>(v).ok()?.sanitize();
            matches!(
                msg,
                ClientMsg::MouseMove { .. }
                    | ClientMsg::MousePos { .. }
                    | ClientMsg::MouseButton { .. }
                    | ClientMsg::MouseScroll { .. }
                    | ClientMsg::Key { .. }
                    | ClientMsg::Text { .. }
            )
            .then_some((id, msg))
        })
        .filter(|(id, _)| last.is_none_or(|n| *id > n))
        .collect();
    result.sort_by_key(|(id, _)| *id);
    result.dedup_by_key(|(id, _)| *id);
    if let Some((id, _)) = result.last() {
        *last = Some(*id)
    }
    result.into_iter().map(|(_, m)| m).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use secpar_host_core::limits::StreamSlots;

    struct Video(Arc<AtomicBool>, u32);
    impl VideoSource for Video {
        fn size(&self) -> (u32, u32) { (64, 48) }
        fn fps(&self) -> u32 { 30 }
        fn codec(&self) -> &'static str { "h264" }
        fn request_keyframe(&mut self) {}
        fn next_frame(&mut self, _: Duration) -> Result<Option<Frame>, String> {
            std::thread::sleep(Duration::from_millis(10));
            self.1 += 1;
            Ok(Some(Frame { data: vec![0, 0, 0, 1, 0x65, self.1 as u8, 9, 9], key: true, capture_us: now_us() }))
        }
    }
    impl Drop for Video {
        fn drop(&mut self) { self.0.store(true, Ordering::SeqCst) }
    }
    struct Held(Arc<Mutex<Vec<String>>>);
    impl InputSink for Held {
        fn apply(&mut self, m: ClientMsg) { self.0.lock().unwrap().push(format!("{m:?}")) }
        fn release_all(&mut self) { self.0.lock().unwrap().push("released".into()) }
    }
    struct Quiet;
    impl Sink for Quiet {
        fn send(&self, _: ServerMsg) -> bool { true }
    }
    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
    }
    #[test]
    fn udp_input_deduplicates_and_cannot_start_other_operations() {
        let mut last = None;
        let p = br#"[{"i":2,"t":"mouse.move","dx":5,"dy":0},{"i":1,"t":"mouse.move","dx":1,"dy":0},{"i":3,"t":"power","a":"shutdown"}]"#;
        assert_eq!(dedup_inputs(p, &mut last).len(), 2);
        assert!(dedup_inputs(p, &mut last).is_empty());
    }
    /// Real UDP over loopback with the real PRS cipher: probe, encrypted video, input, teardown.
    #[test]
    fn stream_pairs_by_probe_sends_video_applies_input_and_releases_on_drop() {
        let slots = Arc::new(StreamSlots::default());
        let dropped = Arc::new(AtomicBool::new(false));
        let log = Arc::new(Mutex::new(vec![]));
        let display = Display { id: "1".into(), name: "d".into(), primary: true, rect: [0, 0, 64, 48] };
        let (stream, ready) = MediaStream::start(
            Sources { video: Box::new(Video(dropped.clone(), 0)), audio: None, input: Some(Box::new(Held(log.clone()))), display },
            vec![],
            vec![],
            Arc::new(Quiet),
            slots.try_acquire().unwrap(),
        )
        .unwrap();
        let ServerMsg::StreamReady { port, session, key, .. } = ready else { panic!() };
        assert_eq!(slots.count(), 1);
        let mut raw = [0u8; 32];
        // decode the standard base64 key back to bytes
        let table = |c: u8| b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().position(|x| *x == c).unwrap() as u32;
        for (i, c) in key.as_bytes().chunks(4).enumerate() {
            let n = c.iter().take_while(|b| **b != b'=').fold(0u32, |a, b| a << 6 | table(*b)) << (6 * (4 - c.iter().take_while(|b| **b != b'=').count()));
            for j in 0..3 {
                if i * 3 + j < 32 { raw[i * 3 + j] = (n >> (16 - 8 * j)) as u8 }
            }
        }
        let crypt = Crypt::new(&raw, session);
        let phone = UdpSocket::bind("127.0.0.1:0").unwrap();
        phone.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let host: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let mut probe = header(Type::Probe, 0);
        probe.seq = 1;
        phone.send_to(&crypt.seal(Dir::PhoneToHost, &probe, &[7u8; 16]), host).unwrap();
        let mut saw_video = false;
        let mut buf = [0u8; 2048];
        for _ in 0..50 {
            let (n, _) = phone.recv_from(&mut buf).unwrap();
            if let Some((h, _)) = crypt.open(Dir::HostToPhone, &buf[..n]) {
                if h.typ == Type::Video { saw_video = true; break }
            }
        }
        assert!(saw_video, "no encrypted video arrived");
        let mut h = header(Type::Input, 0);
        h.seq = 2;
        phone.send_to(&crypt.seal(Dir::PhoneToHost, &h, br#"[{"i":1,"t":"key","vk":65,"down":true}]"#), host).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(log.lock().unwrap().iter().any(|l| l.contains("Key")));
        assert!(stream.alive());
        drop(stream);
        std::thread::sleep(Duration::from_millis(500));
        assert!(dropped.load(Ordering::SeqCst), "capture source was not dropped");
        assert_eq!(slots.count(), 0, "slot not released");
        assert_eq!(log.lock().unwrap().last().unwrap(), "released");
    }
}
