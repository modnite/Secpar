//! Owner-authenticated PRS media transport; one Linux stream at a time.
use crate::{
    capture::{Capture, VideoOptions},
    input::{pack_size, Input},
    now_us,
    pointer::CursorMode,
    portal::Portal,
    store::random,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use secpar_host_core::{
    agent::Outbox,
    proto::{ClientMsg, ServerMsg},
    Display,
};
use secpar_network::{link::Link, reach};
use secpar_stream::{
    frame::{packetize, Codec, FrameHeader},
    Crypt, Dir, Header, Type,
};
use std::{
    collections::HashSet,
    net::{SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
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
    /// Phone candidates, punching window and our TURN allocation (shared with every host).
    link: Arc<Link>,
}
/// How long a new candidate keeps the punching going (docs/remote-protocol.md).
const PUNCH_FOR: Duration = Duration::from_secs(10);
/// Credentials for a TURN relay of our own (`/v1/turn?role=host`); None when this host has no relay access.
pub type RelayProvider = Arc<dyn Fn() -> Option<secpar_host_core::relay::TurnServer> + Send + Sync>;
impl Shared {
    fn send(&self, mut h: Header, plain: &[u8]) {
        let peer = *self.peer.lock().unwrap();
        if let Some(peer) = peer {
            h.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            let pkt = self.crypt.seal(Dir::HostToPhone, &h, plain);
            // through a relay at most RELAY_PPS packets a second, like every host
            self.link.throttle(&peer);
            self.link.transmit(&self.socket, peer, &pkt);
        }
    }
    fn probe_to(&self, peer: SocketAddr) {
        let mut h = header(Type::Probe, 0);
        h.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut p = [0; 16];
        p[..8].copy_from_slice(&now_us().to_be_bytes());
        let pkt = self.crypt.seal(Dir::HostToPhone, &h, &p);
        self.link.transmit(&self.socket, peer, &pkt);
        self.link.transmit_punch_copy(&self.socket, peer, &pkt);
    }
}
pub struct Stream {
    shared: Arc<Shared>,
    input: mpsc::Sender<ClientMsg>,
    portal: Arc<Portal>,
    out: Outbox,
    relay: Option<RelayProvider>,
    mapped: Option<secpar_network::upnp::Mapped>,
    /// A size the client asked for (`stream.size`), picked up by the video thread.
    resize: Arc<Mutex<Option<(u32, u32)>>>,
}
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
    pub codecs: Vec<String>,
    pub candidates: Vec<SocketAddr>,
    pub audio: bool,
    /// Where to get TURN credentials for a relay of our own; None until this host has relay access.
    pub relay: Option<RelayProvider>,
    /// The session's virtual controllers: `pad` events that arrive over the media channel go here.
    pub pads: SharedPads,
}
/// Shared between the control connection and the media channel (both carry `pad`).
pub type SharedPads = Arc<Mutex<Option<secpar_host_core::pad::Pads>>>;
/// This host's IPv4 addresses, best first. The phone tries them in this order and waits two
/// seconds on each dead one, so the order decides how fast it connects.
pub fn local_addresses() -> Vec<String> {
    let found = get_if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(ip) => Some((i.name, ip)),
            _ => None,
        })
        .collect();
    rank_addresses(found, default_route_interface().as_deref())
}

/// Interfaces a phone cannot be on: containers, VMs and their bridges.
fn is_virtual(name: &str) -> bool {
    ["docker", "br-", "veth", "virbr", "vmnet", "vboxnet", "lxc", "lxd", "cni", "flannel", "podman", "incus"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Loopback and link-local dropped. Real network first, private ranges before others, the default
/// route's interface first of those; container bridges last. Stable, so each address appears once.
pub fn rank_addresses(mut found: Vec<(String, std::net::Ipv4Addr)>, default: Option<&str>) -> Vec<String> {
    found.retain(|(_, ip)| !ip.is_loopback() && !ip.is_unspecified() && !ip.is_link_local());
    found.sort_by_key(|(name, ip)| (is_virtual(name), !ip.is_private(), Some(name.as_str()) != default));
    let mut seen = HashSet::new();
    found.into_iter().filter(|(_, ip)| seen.insert(*ip)).map(|(_, ip)| ip.to_string()).collect()
}

/// The interface the IPv4 default route leaves through, from `/proc/net/route`.
fn default_route_interface() -> Option<String> {
    let table = std::fs::read_to_string("/proc/net/route").ok()?;
    table.lines().skip(1).find_map(|line| {
        let mut columns = line.split_whitespace();
        let (name, destination) = (columns.next()?, columns.next()?);
        (destination == "00000000").then(|| name.to_string())
    })
}

#[cfg(test)]
mod address_order_tests {
    use super::*;

    fn ip(s: &str) -> std::net::Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn real_network_comes_first_and_bridges_last() {
        let found = vec![
            ("docker0".into(), ip("172.17.0.1")),
            ("lo".into(), ip("127.0.0.1")),
            ("tailscale0".into(), ip("100.101.102.103")),
            ("wlan0".into(), ip("192.168.1.20")),
            ("enp0s25".into(), ip("10.0.0.5")),
            ("wlan0".into(), ip("169.254.3.4")),
            ("br-1a2b".into(), ip("172.18.0.1")),
        ];
        assert_eq!(
            rank_addresses(found.clone(), Some("wlan0")),
            ["192.168.1.20", "10.0.0.5", "100.101.102.103", "172.17.0.1", "172.18.0.1"]
        );
        assert_eq!(rank_addresses(found, None)[0], "192.168.1.20", "without a default route the order is stable");
    }

    #[test]
    fn the_same_address_is_listed_once() {
        let found = vec![("wlan0".into(), ip("192.168.1.20")), ("wlan0:1".into(), ip("192.168.1.20"))];
        assert_eq!(rank_addresses(found, None), ["192.168.1.20"]);
    }
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
            link: Arc::new(Link::new()),
        });
        // Before the receive thread starts: STUN reads its answer from this socket. The router
        // mapping and the reflexive address are found at once.
        let gathered = {
            let s = shared.clone();
            tokio::task::spawn_blocking(move || reach::gather(&s.socket, port, true))
                .await
                .unwrap_or_default()
        };
        shared
            .socket
            .set_read_timeout(Some(Duration::from_millis(25)))
            .map_err(|_| "Could not configure streaming socket")?;
        shared.link.punch(options.candidates.iter().copied(), PUNCH_FOR);
        let (input, input_rx) = mpsc::channel::<ClientMsg>(128);
        let input_portal = portal.clone();
        let input_shared = shared.clone();
        let picture = Arc::new(AtomicU64::new(pack_size(width, height)));
        let input_picture = picture.clone();
        let pads = options.pads.clone();
        tokio::spawn(async move {
            let mut input_rx = input_rx;
            let mut target = Input::new(input_portal, input_picture);
            loop {
                if input_shared.stop.load(Ordering::Relaxed) {
                    break;
                }
                tokio::select! {msg=input_rx.recv()=>match msg {Some(msg)=>{
                    if let Some(pad) = pad_message(&msg) {
                        pad.apply(&pads);
                        continue;
                    }
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
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            let mut seen = None::<u64>;
            let mut last_rx = Instant::now();
            let begun = Instant::now();
            let mut last_punch = Instant::now() - Duration::from_secs(1);
            while !rx_shared.stop.load(Ordering::Relaxed) {
                let known = rx_shared.peer.lock().unwrap().is_some();
                let punching = rx_shared.link.punching(known);
                if punching && last_punch.elapsed() > Duration::from_millis(100) {
                    for peer in rx_shared.link.cands() {
                        rx_shared.probe_to(peer)
                    }
                    last_punch = Instant::now();
                }
                if begun.elapsed() > Duration::from_secs(10)
                    && last_rx.elapsed() > Duration::from_secs(5)
                    && !punching
                {
                    break;
                }
                let Ok((n, from)) = rx_shared.socket.recv_from(&mut buf) else {
                    continue;
                };
                // Packets from our TURN server: answers go to the waiting request, relayed data
                // carries the phone's own address.
                let Some((from, relayed)) = rx_shared.link.unwrap(from, &buf[..n]) else {
                    continue;
                };
                let pkt = relayed.as_deref().unwrap_or(&buf[..n]);
                let Some((h, p)) = rx_shared.crypt.open(Dir::PhoneToHost, pkt) else {
                    continue;
                };
                if relayed.is_some() {
                    rx_shared.link.mark_relayed(from);
                }
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
        // `stream.ready` as sent now and again whenever the size changes mid-stream
        let ready_template = ServerMsg::StreamReady {
            codec: codec_name.clone(),
            w: width,
            h: height,
            fps,
            port,
            session,
            key: STANDARD.encode(key),
            // LAN, router-mapped public, STUN-reflexive: the same order as the Windows host
            cands: secpar_network::link::host_cands(
                &local_addresses(),
                port,
                gathered.mapped.as_ref().map_or(0, |m| m.port),
                gathered.mapped.as_ref().and_then(|m| m.public),
                gathered.reflexive,
            ),
        };
        let resize = Arc::new(Mutex::new(None::<(u32, u32)>));
        let (video_resize, video_out, video_ready) = (resize.clone(), out.clone(), ready_template.clone());
        let video_picture = picture.clone();
        let video_shared = shared.clone();
        std::thread::spawn(move || {
            let _guard = guard;
            let failed = video_out.clone();
            let ended = video_shared.clone();
            // A panic in here must end this stream and say why. It would otherwise end only this thread and leave the
            // session open with a picture that never moves.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let mut id = 0u32;
                while !video_shared.stop.load(Ordering::Relaxed) {
                    if video_shared.peer.lock().unwrap().is_none() {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    if let Some(wanted) = video_resize.lock().unwrap().take() {
                        // the encoder renegotiates on the new size and opens with a keyframe
                        if let (Some((w, h)), ServerMsg::StreamReady { codec, fps, port, session, key, cands, .. }) = (capture.set_size(wanted), &video_ready) {
                            video_picture.store(pack_size(w, h), Ordering::Relaxed);
                            video_out.send(ServerMsg::StreamReady { codec: codec.clone(), w, h, fps: *fps, port: *port, session: *session, key: key.clone(), cands: cands.clone() });
                            video_shared.idr.store(true, Ordering::Relaxed);
                        }
                    }
                    if video_shared.idr.swap(false, Ordering::Relaxed) {
                        capture.request_keyframe();
                    }
                    let frame = match capture.next_frame(Duration::from_millis(100)) {
                        Ok(Some(f)) => f,
                        Ok(None) => continue,
                        Err(error) => return Some(error),
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
                None
            }));
            let why = match outcome {
                Ok(error) => error,
                Err(_) => Some("Desktop capture stopped unexpectedly.".to_string()),
            };
            if let Some(why) = why {
                failed.send(ServerMsg::error("capture_failed", &why, Some("stream.start")));
            }
            ended.stop.store(true, Ordering::Relaxed);
            ended.send(header(Type::Bye, 0), &[]);
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
        // When the frames carry no pointer, clients draw this one at the position we send. When they do,
        // the video already shows the real pointer and a second one would be a fake.
        if portal.cursor == CursorMode::Hidden {
            out.send(ServerMsg::CursorShape { png: STANDARD.encode(include_bytes!("arrow.png")), hx: 1, hy: 1 });
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
        let ready = ready_template;
        Ok((
            Self {
                shared,
                input,
                portal,
                out,
                relay: options.relay,
                mapped: gathered.mapped,
                resize,
            },
            ready,
        ))
    }
    pub fn input(&self, m: ClientMsg) {
        if self.input.try_send(m).is_err() {
            self.shared.stop.store(true, Ordering::Relaxed);
        }
    }
    /// `stream.size`: resize the running stream (0x0 = the display's own size).
    pub fn resize(&self, w: u32, h: u32) {
        *self.resize.lock().unwrap() = Some((w.min(7680), h.min(4320)));
    }
    pub fn alive(&self) -> bool {
        !self.shared.stop.load(Ordering::Relaxed)
    }
    /// A phone candidate from `stream.cand`: punch to it (restarting the window) and permit it on
    /// our relay. When we have no relay yet, get one too so both ends talk relay-to-relay
    /// (carrier NATs), and tell the phone our relayed address.
    pub fn add_candidate(&self, peer: SocketAddr) {
        self.shared.link.punch([peer], PUNCH_FOR);
        self.shared.probe_to(peer);
        self.shared.link.permit_in_background(&self.shared.socket, peer);
        let Some(relay) = self.relay.clone() else { return };
        if self.shared.link.turn().is_some() {
            return;
        }
        let (shared, out) = (self.shared.clone(), self.out.clone());
        std::thread::spawn(move || {
            let Some(server) = relay() else { return };
            let weak = Arc::downgrade(&shared);
            let stopped = move || weak.upgrade().is_none_or(|s| s.stop.load(Ordering::Relaxed));
            if let Some(relayed) = shared.link.start_turn(&shared.socket, stopped, server.addr, server.user, server.pass, None) {
                shared.link.punch([], PUNCH_FOR);
                out.send(ServerMsg::StreamCand { c: relayed.to_string() });
            }
        });
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.send(header(Type::Bye, 0), &[]);
        if let Some(m) = self.mapped.take() {
            std::thread::spawn(move || reach::release(&m));
        }
        let p = self.portal.clone();
        tokio::spawn(async move {
            let _ = p.close().await;
        });
    }
}
/// A gamepad message from the media channel, ready to apply to the session's controllers.
pub enum PadMessage {
    State(u8, secpar_host_core::pad::PadState),
    Off(u8),
}
pub fn pad_message(msg: &ClientMsg) -> Option<PadMessage> {
    match *msg {
        ClientMsg::Pad { id, btn, lt, rt, lx, ly, rx, ry } => Some(PadMessage::State(id, secpar_host_core::pad::PadState { btn, lt, rt, lx, ly, rx, ry })),
        ClientMsg::PadOff { id } => Some(PadMessage::Off(id)),
        _ => None,
    }
}
impl PadMessage {
    pub fn apply(&self, pads: &SharedPads) {
        if let Some(p) = pads.lock().unwrap().as_mut() {
            match self {
                PadMessage::State(id, state) => {
                    let _ = p.pad(*id, state);
                }
                PadMessage::Off(id) => p.off(*id),
            }
        }
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
                    | ClientMsg::Pad { .. }
                    | ClientMsg::PadOff { .. }
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
    #[test]
    fn udp_input_carries_gamepad_state_to_the_session_controllers() {
        let mut last = None;
        let p=br#"[{"i":1,"t":"pad","id":1,"btn":4096,"lt":0,"rt":255,"lx":0,"ly":0,"rx":0,"ry":0},{"i":2,"t":"pad.off","id":1},{"i":3,"t":"pad","id":9,"btn":0,"lt":0,"rt":0,"lx":0,"ly":0,"rx":0,"ry":0}]"#;
        let msgs = dedup_inputs(p, &mut last);
        assert_eq!(msgs.len(), 3);
        let pads: SharedPads = Arc::new(Mutex::new(None));
        for m in &msgs { pad_message(m).unwrap().apply(&pads); } // no controllers yet: ignored
        assert!(pad_message(&ClientMsg::Ping).is_none());
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        struct Bus(Arc<Mutex<Vec<String>>>);
        struct Target(u8, Arc<Mutex<Vec<String>>>);
        impl secpar_host_core::pad::Bus for Bus {
            fn plug(&self, id: u8, _: secpar_host_core::pad::Rumble) -> Option<Box<dyn secpar_host_core::pad::Target>> { self.0.lock().unwrap().push(format!("plug {id}")); Some(Box::new(Target(id, self.0.clone()))) }
        }
        impl secpar_host_core::pad::Target for Target {
            fn update(&mut self, s: &secpar_host_core::pad::PadState) -> bool { self.1.lock().unwrap().push(format!("update {} {} {}", self.0, s.btn, s.rt)); true }
        }
        impl Drop for Target { fn drop(&mut self) { self.1.lock().unwrap().push(format!("unplug {}", self.0)); } }
        *pads.lock().unwrap() = Some(secpar_host_core::pad::Pads::new(Arc::new(Bus(log.clone())), Box::new(|_, _, _| {})));
        for m in &msgs { pad_message(m).unwrap().apply(&pads); }
        assert_eq!(*log.lock().unwrap(), ["plug 1", "update 1 4096 255", "unplug 1"], "id 9 is out of range and ignored");
    }
    fn synthetic_shared()->Arc<Shared> {
        Arc::new(Shared {socket:UdpSocket::bind("127.0.0.1:0").unwrap(),crypt:Crypt::new(&[7;32],9),peer:Mutex::new(None),seq:AtomicU32::new(0),stop:AtomicBool::new(false),idr:AtomicBool::new(false),link:Arc::new(Link::new())})
    }
    /// The receive loop's job for a relay: the TURN server's answers go to the link.
    fn serve_relay_answers(shared: &Arc<Shared>) {
        let (s, sock) = (shared.clone(), shared.socket.try_clone().unwrap());
        sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !s.stop.load(Ordering::Relaxed) {
                if let Ok((n, from)) = sock.recv_from(&mut buf) {
                    let _ = s.link.unwrap(from, &buf[..n]);
                }
            }
        });
    }
    #[test]
    fn media_goes_through_the_relay_when_the_phone_came_in_through_it() {
        use secpar_network::testing::FakeServer;
        let server = FakeServer::start();
        let shared = synthetic_shared();
        serve_relay_answers(&shared);
        let phone: SocketAddr = "203.0.113.9:4000".parse().unwrap();
        shared.link.punch([phone], PUNCH_FOR);
        let stop = shared.clone();
        let weak = Arc::downgrade(&shared);
        let relayed = shared.link.start_turn(&shared.socket, move || weak.upgrade().is_none_or(|s| s.stop.load(Ordering::Relaxed)), server.addr, "u".into(), "p".into(), None);
        assert!(relayed.is_some(), "the relay answered the allocation");
        assert!(server.log().contains(&"permit 203.0.113.9:4000".to_string()), "the phone's candidate is permitted: {:?}", server.log());

        // the phone's first PROBE arrived as relayed data: replies go back the same way
        *shared.peer.lock().unwrap() = Some(phone);
        assert!(shared.link.mark_relayed(phone));
        shared.send(header(Type::Video, 1), b"video through the relay");
        let deadline = Instant::now() + Duration::from_secs(1);
        while server.relayed().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let got = server.relayed();
        assert_eq!(got.len(), 1, "{:?}", server.log());
        let (h, p) = shared.crypt.open(Dir::HostToPhone, &got[0]).expect("the relay carries the sealed packet unchanged");
        assert_eq!((h.typ, p.as_slice()), (Type::Video, &b"video through the relay"[..]));

        // the same pacing rule as the Windows host: at most RELAY_PPS packets a second
        let t0 = Instant::now();
        for i in 0..300 {
            shared.send(header(Type::Video, i), &[0u8; 8]);
        }
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(80), "300 packets at 3000 pps take 100 ms, took {took:?}");
        stop.stop.store(true, Ordering::Relaxed);
    }
    #[test]
    fn direct_peers_are_not_paced_or_wrapped() {
        let shared = synthetic_shared();
        let phone = UdpSocket::bind("127.0.0.1:0").unwrap();
        phone.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        *shared.peer.lock().unwrap() = Some(phone.local_addr().unwrap());
        let t0 = Instant::now();
        for i in 0..300 {
            shared.send(header(Type::Video, i), &[1u8; 8]);
        }
        assert!(t0.elapsed() < Duration::from_millis(60));
        let mut b = [0u8; 2048];
        let n = phone.recv(&mut b).unwrap();
        assert_eq!(shared.crypt.open(Dir::HostToPhone, &b[..n]).unwrap().0.typ, Type::Video, "plain sealed packet to the phone");
    }
    #[test]
    fn stream_cand_punches_through_the_relay_and_restarts_the_window() {
        use secpar_network::testing::FakeServer;
        let server = FakeServer::start();
        let shared = synthetic_shared();
        serve_relay_answers(&shared);
        let weak = Arc::downgrade(&shared);
        shared.link.start_turn(&shared.socket, move || weak.upgrade().is_none_or(|s| s.stop.load(Ordering::Relaxed)), server.addr, "u".into(), "p".into(), None).unwrap();
        let relayed_phone: SocketAddr = "198.51.100.5:6000".parse().unwrap();
        shared.link.punch([relayed_phone], Duration::from_millis(50));
        std::thread::sleep(Duration::from_millis(80));
        assert!(!shared.link.punching(false), "window over");
        // what Stream::add_candidate does
        shared.link.punch([relayed_phone], PUNCH_FOR);
        shared.probe_to(relayed_phone);
        shared.link.permit_in_background(&shared.socket, relayed_phone);
        assert!(shared.link.punching(false));
        std::thread::sleep(Duration::from_millis(400));
        let log = server.log();
        assert!(log.contains(&"permit 198.51.100.5:6000".to_string()), "{log:?}");
        let n = server.relayed().len();
        assert!(n >= 1, "the PROBE also went out through our relay: {log:?}");
        let probe = server.relayed().into_iter().find_map(|d| shared.crypt.open(Dir::HostToPhone, &d)).expect("a sealed PROBE");
        assert_eq!(probe.0.typ, Type::Probe);
        shared.stop.store(true, Ordering::Relaxed);
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
