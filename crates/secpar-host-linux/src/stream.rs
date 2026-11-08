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
        guard: secpar_host_core::limits::StreamPermit,
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
                tokio::select! {msg=input_rx.recv()=>match msg {Some(msg)=>{
                    if target.apply(msg).await.is_err(){break}
                    if let Some((x,y))=target.take_pos() {
                        let mut packet=[0u8;5];
                        packet[..2].copy_from_slice(&(x.clamp(i16::MIN as i32,i16::MAX as i32) as i16).to_be_bytes());
                        packet[2..4].copy_from_slice(&(y.clamp(i16::MIN as i32,i16::MAX as i32) as i16).to_be_bytes());
                        packet[4]=1;
                        input_shared.send(header(Type::Cursor,0),&packet);
                    }
                },None=>break},_=tokio::time::sleep(Duration::from_millis(20))=>{}}
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
        // The portal hides the pointer in the video; clients draw this one at the position we send.
        out.send(ServerMsg::CursorShape { png: STANDARD.encode(include_bytes!("arrow.png")), hx: 1, hy: 1 });
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
    pub(super) fn default_monitor() -> Result<String,String> {
        gst::init().map_err(|_| "Could not initialize desktop audio")?;
        let provider=gst::DeviceProviderFactory::by_name("pulsedeviceprovider")
            .ok_or("Desktop audio device discovery is unavailable")?;
        provider.start().map_err(|_|"Could not connect to the desktop sound service")?;
        let sink=provider.devices().into_iter().find(|device|
            device.device_class().as_str()=="Audio/Sink" && device.properties()
                .and_then(|p|p.get::<bool>("is-default").ok())==Some(true))
            .and_then(|device|device.create_element(None).ok())
            .and_then(|element|element.property::<Option<String>>("device"));
        provider.stop();
        // Select only the default speaker's monitor, never the default source
        // (which can be a microphone). No external pactl executable is needed.
        sink.filter(|s|!s.is_empty()).map(|s|format!("{s}.monitor"))
            .ok_or_else(||"No default system audio output is available".into())
    }
    pub fn run(shared: Arc<Shared>) -> Result<(), String> {
        let monitor=default_monitor()?;
        let source = gst::ElementFactory::make("pulsesrc")
            .property("device", monitor)
            .property("do-timestamp", true)
            .build()
            .map_err(|_| "Install GStreamer PulseAudio support")?;
        run_source(shared,source)
    }
    pub(super) fn run_source(shared: Arc<Shared>,source: gst::Element) -> Result<(),String> {
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
        if pipeline.set_state(gst::State::Playing).is_err() {
            let error=output_error(&pipeline).unwrap_or_else(||"Could not start system audio capture".into());
            let _=pipeline.set_state(gst::State::Null);
            return Err(error);
        }
        let result = (|| {
            let mut encoder =
                opus::Encoder::new(48000, opus::Channels::Stereo, opus::Application::LowDelay)
                    .map_err(|_| "Could not start Opus encoder")?;
            encoder
                .set_bitrate(opus::Bitrate::Bits(128000))
                .map_err(|_| "Could not configure Opus encoder")?;
            let mut frames = secpar_host_core::audio::AudioFrames::default();
            let mut id = 0u32;
            while !shared.stop.load(Ordering::Relaxed) {
                let Some(sample) = appsink.try_pull_sample(gst::ClockTime::from_mseconds(100))
                else {
                    if let Some(error)=output_error(&pipeline) {return Err(error);}
                    continue;
                };
                let Some(buffer) = sample.buffer() else {
                    continue;
                };
                let map = buffer
                    .map_readable()
                    .map_err(|_| "Could not read captured audio")?;
                let lag = pipeline.current_running_time()
                    .and_then(|n| buffer.pts().map(|p| n.saturating_sub(p)))
                    .map(|d| d.useconds()).unwrap_or(0);
                let capture_us = now_us().saturating_sub(lag);
                let pcm: Vec<_> = map.as_slice().chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
                frames.push(&pcm, capture_us);
                while let Some((frame, capture_us)) = frames.next_frame() {
                    let mut packet = [0u8; 1100];
                    let n = encoder
                        .encode_float(&frame, &mut packet)
                        .map_err(|_| "Could not encode audio")?;
                    let mut payload = capture_us.to_be_bytes().to_vec();
                    payload.extend_from_slice(&packet[..n]);
                    shared.send(header(Type::Audio, id), &payload);
                    id = id.wrapping_add(1);
                }
            }
            Ok(())
        })();
        let _ = pipeline.set_state(gst::State::Null);
        result
    }
    fn output_error(pipeline:&gst::Pipeline)->Option<String> {
        let message=pipeline.bus()?.pop_filtered(&[gst::MessageType::Error,gst::MessageType::Eos])?;
        Some(match message.view() {
            gst::MessageView::Error(error)=>format!("System audio capture stopped: {}",error.error()),
            _=>"System audio capture ended".into(),
        })
    }
}
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "Requires isolated synthetic Pulse service and named output monitor"]
    fn desktop_output_monitor_is_discovered_without_pactl() {
        assert_eq!(std::env::var("SECPAR_AUDIO_REVIEW").as_deref(),Ok("1"));
        assert_eq!(super::audio::default_monitor().unwrap(),"secpar_review.monitor");
    }
    use super::*;
    use gstreamer::{self as gst,prelude::*};
    #[test]
    fn udp_input_deduplicates_and_cannot_start_other_operations() {
        let mut last = None;
        let p=br#"[{"i":2,"t":"mouse.move","dx":5,"dy":0},{"i":1,"t":"mouse.move","dx":1,"dy":0},{"i":3,"t":"power","a":"shutdown"}]"#;
        assert_eq!(dedup_inputs(p, &mut last).len(), 2);
        assert!(dedup_inputs(p, &mut last).is_empty());
    }
    fn synthetic_shared()->Arc<Shared> {
        Arc::new(Shared {socket:UdpSocket::bind("127.0.0.1:0").unwrap(),crypt:Crypt::new(&[7;32],9),peer:Mutex::new(None),seq:AtomicU32::new(0),stop:AtomicBool::new(false),idr:AtomicBool::new(false)})
    }
    #[test]
    fn unavailable_audio_source_preserves_video_transport() {
        gst::init().unwrap();
        // Explicit nonexistent server: never connect to this machine's audio session.
        let source=gst::ElementFactory::make("pulsesrc")
            .property("server","unix:/tmp/secpar-no-audio-review-server")
            .property("device","secpar_missing_monitor.monitor").build().unwrap();
        let shared=synthetic_shared();
        let error=audio::run_source(shared.clone(),source).expect_err("Missing virtual output must fail");
        eprintln!("Synthetic host audio startup failure: {error}");
        assert!(!shared.stop.load(Ordering::Relaxed),"Optional audio must not stop media");
        let receiver=UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        *shared.peer.lock().unwrap()=Some(receiver.local_addr().unwrap());
        shared.send(header(Type::Video,1),b"synthetic-video-after-audio-failure");
        let mut packet=[0u8;2048];
        let size=receiver.recv(&mut packet).unwrap();
        let (h,p)=shared.crypt.open(Dir::HostToPhone,&packet[..size]).unwrap();
        assert_eq!(h.typ,Type::Video);assert_eq!(p,b"synthetic-video-after-audio-failure");
    }
    #[test]
    fn asynchronous_audio_pipeline_error_is_reported_instead_of_hanging() {
        gst::init().unwrap();
        let source=gst::parse::bin_from_description("audiotestsrc is-live=true ! identity error-after=2",true).unwrap();
        let shared=synthetic_shared();
        let started=Instant::now();
        let error=audio::run_source(shared.clone(),source.upcast()).expect_err("Synthetic source must fail");
        eprintln!("Synthetic asynchronous host audio failure: {error}");
        assert!(started.elapsed()<Duration::from_secs(3),"Asynchronous failure must not loop forever");
        assert!(error.contains("System audio capture stopped"),"{error}");
        assert!(!shared.stop.load(Ordering::Relaxed));
    }
}
