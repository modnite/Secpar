//! Owner-authenticated PRS media transport; one Linux stream at a time.
use crate::{
    capture::{Capture, VideoOptions},
    input::Input,
    now_us,
    portal::Portal,
    store::random,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use secpar_host_core::{
    agent::Outbox,
    proto::{ClientMsg, ServerMsg},
    Display,
};
use secpar_stream::{
    frame::{packetize, Codec, FrameHeader},
    Crypt, Dir, Header, Type,
};
use std::{
    collections::HashSet,
    net::{SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

fn header(typ: Type, frame_id: u32) -> Header {
    Header {
        typ,
        frame_id,
        flags: 0,
        k: 0,
        seq: 0,
        frag: 0,
        count: 0,
    }
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
            let _ = self
                .socket
                .send_to(&self.crypt.seal(Dir::HostToPhone, &h, plain), peer);
        }
    }
    fn probe_to(&self, peer: SocketAddr) {
        let mut h = header(Type::Probe, 0);
        h.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut p = [0; 16];
        p[..8].copy_from_slice(&now_us().to_be_bytes());
        let _ = self
            .socket
            .send_to(&self.crypt.seal(Dir::HostToPhone, &h, &p), peer);
    }
}
pub struct ActiveGuard(pub Arc<AtomicBool>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
pub struct Stream {
    shared: Arc<Shared>,
    input: mpsc::Sender<ClientMsg>,
    portal: Arc<Portal>,
}
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
    pub codecs: Vec<String>,
    pub candidates: Vec<SocketAddr>,
    pub audio: bool,
}
pub fn local_addresses() -> Vec<String> {
    get_if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => {
                Some(ip.to_string())
            }
            _ => None,
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}
impl Stream {
    pub async fn start(
        portal: Arc<Portal>,
        options: Options,
        out: Outbox,
        guard: ActiveGuard,
    ) -> Result<(Self, ServerMsg), String> {
        let p = portal.clone();
        let video_options = VideoOptions {
            width: options.width,
            height: options.height,
            fps: options.fps,
            kbps: options.kbps,
            codecs: options.codecs,
        };
        let mut capture = tokio::task::spawn_blocking(move || Capture::start(p, video_options))
            .await
            .map_err(|_| "Video initialization failed")??;
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
        let port = socket
            .local_addr()
            .map_err(|_| "Could not read stream port")?
            .port();
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
        let (input, input_rx) = mpsc::channel::<ClientMsg>(128);
        let input_portal = portal.clone();
        let input_shared = shared.clone();
        tokio::spawn(async move {
            let mut input_rx = input_rx;
            let mut target = Input::new(input_portal);
            loop {
                if input_shared.stop.load(Ordering::Relaxed) {
                    break;
                }
                tokio::select! {msg=input_rx.recv()=>match msg {Some(msg)=>if target.apply(msg).await.is_err(){break},None=>break},_=tokio::time::sleep(Duration::from_millis(20))=>{}}
            }
            let _ = target.release_all().await;
        });
        let rx_shared = shared.clone();
        let rx_input = input.clone();
        let cands = options.candidates;
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
                    for peer in cands.iter().take(16) {
                        rx_shared.probe_to(*peer)
                    }
                    last_punch = Instant::now();
                }
                if begun.elapsed() > Duration::from_secs(10)
                    && last_rx.elapsed() > Duration::from_secs(5)
                {
                    break;
                }
                let Ok((n, from)) = rx_shared.socket.recv_from(&mut buf) else {
                    continue;
                };
                let Some((h, p)) = rx_shared.crypt.open(Dir::PhoneToHost, &buf[..n]) else {
                    continue;
                };
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
                            if rx_input.try_send(msg).is_err() {
                                rx_shared.stop.store(true, Ordering::Relaxed);
                                break;
                            }
                        }
                    }
                    Type::Idr => {
                        rx_shared.idr.store(true, Ordering::Relaxed);
                    }
                    Type::Bye => break,
                    _ => {}
                }
            }
            rx_shared.stop.store(true, Ordering::Relaxed);
        });
        let video_shared = shared.clone();
        std::thread::spawn(move || {
            let _guard = guard;
            let mut id = 0u32;
            while !video_shared.stop.load(Ordering::Relaxed) {
                if video_shared.peer.lock().unwrap().is_none() {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                if video_shared.idr.swap(false, Ordering::Relaxed) {
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
                    if let Some(wait) = (start + Duration::from_millis(index as u64))
                        .checked_duration_since(Instant::now())
                    {
                        std::thread::sleep(wait)
                    }
                    for (h, p) in chunk {
                        video_shared.send(*h, p)
                    }
                }
                id = id.wrapping_add(1);
            }
            video_shared.stop.store(true, Ordering::Relaxed);
            video_shared.send(header(Type::Bye, 0), &[]);
        });
        if options.audio {
            let audio_shared = shared.clone();
            let audio_out = out.clone();
            std::thread::spawn(move || {
                if let Err(e) = audio::run(audio_shared) {
                    audio_out.send(ServerMsg::error(
                        "audio_unavailable",
                        &e,
                        Some("stream.start"),
                    ));
                }
            });
        }
        out.send(ServerMsg::StreamDisplays {
            list: vec![Display {
                id: "portal".into(),
                name: "Shared display".into(),
                primary: true,
                rect: [0, 0, width as i32, height as i32],
            }],
            current: "portal".into(),
        });
        let ready = ServerMsg::StreamReady {
            codec: codec_name,
            w: width,
            h: height,
            fps,
            port,
            session,
            key: STANDARD.encode(key),
            cands: local_addresses()
                .into_iter()
                .map(|ip| format!("{ip}:{port}"))
                .collect(),
        };
        Ok((
            Self {
                shared,
                input,
                portal,
            },
            ready,
        ))
    }
    pub fn input(&self, m: ClientMsg) {
        if self.input.try_send(m).is_err() {
            self.shared.stop.store(true, Ordering::Relaxed);
        }
    }
    pub fn alive(&self) -> bool {
        !self.shared.stop.load(Ordering::Relaxed)
    }
    pub fn add_candidate(&self, peer: SocketAddr) {
        self.shared.probe_to(peer);
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.send(header(Type::Bye, 0), &[]);
        let p = self.portal.clone();
        tokio::spawn(async move {
            let _ = p.close().await;
        });
    }
}
fn dedup_inputs(bytes: &[u8], last: &mut Option<u64>) -> Vec<ClientMsg> {
    let Ok(items) = serde_json::from_slice::<Vec<serde_json::Value>>(bytes) else {
        return vec![];
    };
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

mod audio {
    use super::*;
    use gstreamer::{self as gst, prelude::*};
    pub fn run(shared: Arc<Shared>) -> Result<(), String> {
        // Never fall back to a default source: that can be the microphone.
        let output = std::process::Command::new("pactl")
            .arg("get-default-sink")
            .output()
            .map_err(|_| "Install pipewire-pulse/pactl for desktop audio")?;
        if !output.status.success() {
            return Err("No default system audio output is available".into());
        }
        let sink = String::from_utf8(output.stdout).map_err(|_| "Invalid audio output name")?;
        let sink = sink.trim();
        if sink.is_empty() {
            return Err("No default system audio output is available".into());
        }
        gst::init().map_err(|_| "Could not initialize desktop audio")?;
        let source = gst::ElementFactory::make("pulsesrc")
            .property("device", format!("{sink}.monitor"))
            .property("do-timestamp", true)
            .build()
            .map_err(|_| "Install GStreamer PulseAudio support")?;
        let convert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(|_| "Missing audio converter")?;
        let resample = gst::ElementFactory::make("audioresample")
            .build()
            .map_err(|_| "Missing audio resampler")?;
        let caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("audio/x-raw")
                    .field("format", "F32LE")
                    .field("rate", 48000i32)
                    .field("channels", 2i32)
                    .field("layout", "interleaved")
                    .build(),
            )
            .build()
            .map_err(|_| "Missing audio filter")?;
        let appsink = gstreamer_app::AppSink::builder()
            .sync(false)
            .max_buffers(8)
            .drop(true)
            .build();
        let pipeline = gst::Pipeline::new();
        pipeline
            .add_many([&source, &convert, &resample, &caps, appsink.upcast_ref()])
            .map_err(|_| "Could not build audio pipeline")?;
        gst::Element::link_many([&source, &convert, &resample, &caps, appsink.upcast_ref()])
            .map_err(|_| "Could not link audio pipeline")?;
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| "Could not start system audio capture")?;
        let result = (|| {
            let mut encoder =
                opus::Encoder::new(48000, opus::Channels::Stereo, opus::Application::LowDelay)
                    .map_err(|_| "Could not start Opus encoder")?;
            encoder
                .set_bitrate(opus::Bitrate::Bits(128000))
                .map_err(|_| "Could not configure Opus encoder")?;
            let mut pcm = std::collections::VecDeque::<f32>::new();
            let mut capture_us = 0;
            let mut id = 0u32;
            while !shared.stop.load(Ordering::Relaxed) {
                let Some(sample) = appsink.try_pull_sample(gst::ClockTime::from_mseconds(100))
                else {
                    continue;
                };
                let Some(buffer) = sample.buffer() else {
                    continue;
                };
                let map = buffer
                    .map_readable()
                    .map_err(|_| "Could not read captured audio")?;
                if pcm.is_empty() {
                    let lag = pipeline
                        .current_running_time()
                        .and_then(|n| buffer.pts().map(|p| n.saturating_sub(p)))
                        .map(|d| d.useconds())
                        .unwrap_or(0);
                    capture_us = now_us().saturating_sub(lag);
                }
                pcm.extend(
                    map.as_slice()
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes(b.try_into().unwrap())),
                );
                while pcm.len() >= 960 {
                    let frame: Vec<_> = pcm.drain(..960).collect();
                    let mut packet = [0u8; 1100];
                    let n = encoder
                        .encode_float(&frame, &mut packet)
                        .map_err(|_| "Could not encode audio")?;
                    let mut payload = capture_us.to_be_bytes().to_vec();
                    payload.extend_from_slice(&packet[..n]);
                    shared.send(header(Type::Audio, id), &payload);
                    id = id.wrapping_add(1);
                    capture_us += 10000;
                }
            }
            Ok(())
        })();
        let _ = pipeline.set_state(gst::State::Null);
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn udp_input_deduplicates_and_cannot_start_other_operations() {
        let mut last = None;
        let p=br#"[{"i":2,"t":"mouse.move","dx":5,"dy":0},{"i":1,"t":"mouse.move","dx":1,"dy":0},{"i":3,"t":"power","a":"shutdown"}]"#;
        assert_eq!(dedup_inputs(p, &mut last).len(), 2);
        assert!(dedup_inputs(p, &mut last).is_empty());
    }
}
