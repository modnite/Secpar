//! One PRS stream to one phone: the UDP socket, PROBE address learning, video/audio threads,
//! input, REPORT-driven bitrate and the 5 s timeout. Dropping it ends everything.

use super::bitrate::{notch_size, notches_below, Bitrate, Target};
use super::capture::{self, SharedCapture};
use super::convert::{fit, Convert};
use super::encode::{codec_name, parse_codec, Encoder};
use super::input::Dedup;
use super::{audio, cursor, now_us, Com};
use crate::proto::{ClientMsg, ServerMsg};
use secpar_network::link::{self, Link};
use crate::prs::frame::{packetize, Codec, FrameHeader};
use crate::prs::{Crypt, Dir, Header, Type};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};


/// Enforce the host's advertised resolution/frame-rate ceiling for every client.
pub fn quality_limit(profile: &str, w:u32, h:u32, fps:u32)->(u32,u32,u32) {
    let (mw,mh,mfps)=match profile {"battery"=>(1280,720,60),"smooth"=>(1920,1080,60),"sharp"=>(2560,1440,60),"2160p60"=>(3840,2160,60),_=>(0,0,0)};
    if mw==0 {return (w,h,fps)}
    (if w==0 {mw}else{w.min(mw)},if h==0 {mh}else{h.min(mh)},fps.min(mfps))
}

pub const TIMEOUT: Duration = Duration::from_secs(5);
const IDR_EVERY: Duration = Duration::from_millis(250);
/// Hole punching (docs/remote-protocol.md): PROBE every phone candidate this often, for this long.
const PUNCH_EVERY: Duration = Duration::from_millis(100);
const PUNCH_FOR: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct Params {
    pub w: u32,
    pub h: u32,
    pub fps: u32,
    pub codecs: Vec<String>,
    pub max_kbps: u32,
    pub encoder: super::encode::Settings,
    pub audio: bool,
    /// The phone's PRS addresses to punch towards.
    pub cands: Vec<SocketAddr>,
    /// The router's public IP when UPnP mapped the stream port.
    pub public: Option<IpAddr>,
    /// Windows device name of the display to stream; empty = primary.
    pub display: String,
    /// Set for a friend's stream: frames stop when it closes, loses view, or the desktop locks.
    pub gate: Option<secpar_host_core::guest_policy::Gate>,
    /// Where this session's absolute input lands: set to the captured display while streaming.
    pub rect: crate::input::Rect,
    /// The client can show 4:4:4 packed into a double-height 4:2:0 frame (convert_ps).
    pub chroma444: bool,
}

/// Packed 4:4:4 doubles the frame height; encoders handle up to 4096 in each direction.
fn can_pack((w, h): (u32, u32)) -> bool {
    w <= 4096 && h * 2 <= 4096
}

/// The UDP port streams use when it's free (mapped on the router by `net`).
pub const STREAM_PORT: u16 = 48998;

/// Pushes an agent message to the phone (`cursor.shape`, a new `stream.ready`).
pub type Notify = Arc<dyn Fn(ServerMsg) + Send + Sync>;
/// Applies one deduplicated input event.
pub type InputSink = Box<dyn FnMut(ClientMsg) + Send>;

pub struct Shared {
    _stream_slot: Option<secpar_host_core::limits::StreamPermit>,
    crypt: Crypt,
    sock: UdpSocket,
    seq: AtomicU32,
    peer: Mutex<Option<SocketAddr>>,
    stop: AtomicBool,
    idr: AtomicBool,
    target: Mutex<Target>,
    punch_every: Duration,
    punch_for: Duration,
    /// Phone candidates, the punching window and our own TURN allocation (shared with every host).
    link: Arc<Link>,
    /// A display the phone asked to switch to (`stream.display`), picked up by the video thread.
    switch: Mutex<Option<String>>,
    /// A size the client asked for (`stream.size`), picked up by the video thread.
    resize: Mutex<Option<(u32, u32)>>,
    /// Sealed video packets of the last frames, sent again when the client asks (`Type::Nack`).
    history: Mutex<secpar_stream::nack::History>,
}

fn hdr(typ: Type, frame_id: u32) -> Header {
    Header { typ, flags: 0, k: 0, seq: 0, frame_id, frag: 0, count: 0 }
}

impl Shared {
    pub fn new(sock: UdpSocket, key: &[u8; 32], session: u32, target: Target) -> Shared {
        Shared {
            _stream_slot: None,
            crypt: Crypt::new(key, session),
            sock,
            seq: AtomicU32::new(0),
            peer: Mutex::new(None),
            stop: AtomicBool::new(false),
            idr: AtomicBool::new(false),
            target: Mutex::new(target),
            punch_every: PUNCH_EVERY,
            punch_for: PUNCH_FOR,
            link: Arc::new(Link::new()),
            switch: Mutex::default(),
            resize: Mutex::default(),
            history: Mutex::default(),
        }
    }

    /// Adds phone candidates and (re)starts the punching window.
    pub fn punch(&self, add: impl IntoIterator<Item = SocketAddr>) {
        self.link.punch(add, self.punch_for);
    }

    /// Still looking for the phone: no authenticated PROBE yet and inside the window.
    fn punching(&self) -> bool {
        self.link.punching(self.peer().is_some())
    }

    pub fn peer(&self) -> Option<SocketAddr> {
        *self.peer.lock().unwrap()
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Seals and sends to the phone's learned address; false before the first PROBE.
    pub fn send(&self, h: Header, plain: &[u8]) -> bool {
        self.peer().is_some_and(|to| self.send_to(to, h, plain))
    }

    fn send_to(&self, to: SocketAddr, mut h: Header, plain: &[u8]) -> bool {
        h.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let pkt = self.crypt.seal(Dir::HostToPhone, &h, plain);
        if matches!(h.typ, Type::Video | Type::Fec) {
            self.history.lock().unwrap().record(h.frame_id, h.frag, h.count, &pkt);
        }
        self.link.transmit(&self.sock, to, &pkt)
    }

    /// Sends the packets a client lost again, as they were sealed the first time.
    fn resend(&self, frame_id: u32, frags: &[u16]) -> usize {
        let Some(to) = self.peer() else { return 0 };
        let pkts = self.history.lock().unwrap().take(frame_id, frags);
        for pkt in &pkts {
            self.link.throttle(&to);
            self.link.transmit(&self.sock, to, pkt);
        }
        pkts.len()
    }

    /// Punching PROBE to one candidate: direct, and through our relay when we have one.
    fn punch_one(&self, to: SocketAddr) {
        self.send_to(to, hdr(Type::Probe, 0), &probe(0));
        if self.link.turn().is_some() {
            let mut h = hdr(Type::Probe, 0);
            h.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            self.link.transmit_punch_copy(&self.sock, to, &self.crypt.seal(Dir::HostToPhone, &h, &probe(0)));
        }
    }

    /// Allocates our own relay (blocking, ~100 ms), permits the phone's candidates and keeps the
    /// allocation alive until the stream stops. Returns the relayed address to tell the phone.
    pub fn start_turn(self: &Arc<Self>, server: SocketAddr, user: String, pass: String) -> Option<std::net::SocketAddrV4> {
        let weak = Arc::downgrade(self);
        let relayed = self.link.start_turn(&self.sock, move || weak.upgrade().is_none_or(|sh| sh.stopped()), server, user, pass, Some(crate::log::line))?;
        self.punch(std::iter::empty());
        Some(relayed)
    }

    /// [`Shared::start_turn`] over TLS, for networks that block UDP to the relay.
    pub fn start_turn_tls(self: &Arc<Self>, host: String, port: u16, user: String, pass: String) -> Option<std::net::SocketAddrV4> {
        let weak = Arc::downgrade(self);
        let relayed = self.link.start_turn_tls(&self.sock, move || weak.upgrade().is_none_or(|sh| sh.stopped()), host, port, user, pass, Some(crate::log::line))?;
        self.punch(std::iter::empty());
        Some(relayed)
    }

    /// A new phone candidate: punch to it, and permit it on our relay if we have one.
    pub fn add_cand(self: &Arc<Self>, a: SocketAddr) {
        self.punch([a]);
        self.link.permit_in_background(&self.sock, a);
    }

    /// One frame's packets, spread over at most ~4 ms so a burst doesn't overflow Wi-Fi queues.
    /// Through a TURN relay, at most [`link::RELAY_PPS`] packets a second instead: relays drop bursts
    /// above their packet-rate limit (Cloudflare: 5–10 kpps per allocation), which keyframes hit.
    fn send_paced_at(&self, pkts: &[(Header, Vec<u8>)], frame_due: Instant) {
        if self.peer().is_some_and(|p| self.link.is_relayed(&p)) {
            if let Some(wait)=frame_due.checked_duration_since(Instant::now()) {std::thread::sleep(wait);}
            link::send_relay_paced(pkts, |(h, p)| {
                self.send(*h, p);
            });
            return;
        }
        let (width, offset) = video_packet_schedule(pkts);
        let lead=Duration::from_millis(3-offset);
        // Finish systematic data at the frame deadline, not several ms after it.
        // A late encoder still retains packet spacing rather than bursting.
        let first=frame_due.checked_sub(lead).unwrap_or(frame_due).max(Instant::now());
        for (g, chunk) in pkts.chunks(width).enumerate() {
            if let Some(wait) = (first + Duration::from_millis(g as u64)).checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            for (h, p) in chunk {
                self.send(*h, p);
            }
        }
    }
}

/// Without loss, the last systematic shard completes the frame. Keep its batch
/// at 3 ms for every frame size; otherwise small frames arrive immediately and
/// larger ones arrive 2–3 ms later despite identical encoder frame spacing.
/// Parity after that batch retains its spacing for recovery.
fn video_packet_schedule(pkts: &[(Header, Vec<u8>)]) -> (usize, u64) {
    let groups = if pkts.len() <= 8 { 1 } else { 4 };
    let width = pkts.len().div_ceil(groups).max(1);
    let last_data = pkts.iter().rposition(|(h, _)| h.typ == Type::Video)
        .unwrap_or(pkts.len().saturating_sub(1));
    (width, 3 - (last_data / width) as u64)
}

fn encoded_packets(id:u32,e:&super::encode::Encoded,codec:Codec,fec:usize)->Vec<(Header,Vec<u8>)> {
    let fh=FrameHeader{len:e.data.len() as u32,capture_us:e.capture_us,keyframe:e.key,codec};
    let timing=secpar_stream::timing::begin();
    let packets=packetize(id,&fh,&e.data,fec);
    secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Packetize,timing);
    packets
}

fn idle_repeat_due(since_real: Duration, since_send: Duration, interval: Duration) -> bool {
    // Allow half a frame of timer jitter around the two-slot idle cadence.
    // Comparing to exactly two intervals can skip that slot and produce 20 Hz.
    since_real <= Duration::from_millis(500) || since_send >= interval + interval / 2
}

/// Near-equal display rates are matched (e.g. 59.94 rather than 60) so a
/// nominal stream clock cannot slowly run ahead of the captured display.
fn stream_interval(fps: u32, refresh: Option<(u32,u32)>) -> Duration {
    let fps = fps.max(1) as u128;
    if let Some((num,den)) = refresh.filter(|(n,d)| *n>0 && *d>0) {
        let (num,den) = (num as u128,den as u128);
        let divisor = num.div_ceil(fps*den).max(1);
        let requested = fps*den*divisor;
        if num <= requested && num*100 >= requested*99 {
            return Duration::from_nanos((1_000_000_000*den*divisor/num) as u64);
        }
    }
    Duration::from_nanos((1_000_000_000/fps) as u64)
}

fn source_deadline(capture_us:u64, clock_us:u64, now:Instant, budget:Duration) -> Option<Instant> {
    let age = Duration::from_micros(clock_us.checked_sub(capture_us)?);
    if age > budget { return None; } // initial duplication can contain an old still image
    now.checked_sub(age)?.checked_add(budget)
}

fn source_in_slot(capture_us:u64, clock_us:u64, now:Instant, slot:Instant) -> bool {
    let target = if slot >= now { clock_us.saturating_add((slot-now).as_micros() as u64) }
        else { clock_us.saturating_sub((now-slot).as_micros() as u64) };
    // Tolerate small compositor timestamp variation, not a whole older frame.
    capture_us.saturating_add(2_000) >= target
}

/// REPORT plaintext: (lossPermille, rttMs).
/// (loss ‰, RTT ms, frame queueing delay ms: bytes 12-13, zero from clients before 4.2.0-r22).
fn parse_report(p: &[u8]) -> Option<(u16, u16, u16)> {
    (p.len() >= 16).then(|| (u16::from_be_bytes([p[4], p[5]]), u16::from_be_bytes([p[10], p[11]]), u16::from_be_bytes([p[12], p[13]])))
}

fn probe(echo: u64) -> [u8; 16] {
    let mut p = [0u8; 16];
    p[..8].copy_from_slice(&now_us().to_be_bytes());
    p[8..].copy_from_slice(&echo.to_be_bytes());
    p
}

/// The receive loop: runs until BYE, `timeout` of silence (new candidates and relay setup extend
/// it), or `stop`.
pub fn control(sh: &Shared, input: &mut dyn FnMut(ClientMsg), br: &mut Bitrate, timeout: Duration) {
    let start = Instant::now();
    let (mut wait, mut next_punch) = (Duration::ZERO, start);
    let (mut last_rx, mut last_idr, mut probe_seq) = (Instant::now(), None::<Instant>, None::<u32>);
    let mut dedup = Dedup::default();
    let (mut input_pkts, mut input_log) = (0u64, Instant::now());
    let (mut resent, mut resent_log) = (0u64, Instant::now());
    let mut buf = [0u8; 2048];
    loop {
        let punching = sh.punching();
        if sh.stopped() || (last_rx.elapsed() >= timeout && !punching && !sh.link.searching()) {
            break;
        }
        let now = Instant::now();
        if punching && now >= next_punch {
            for to in sh.link.cands() {
                sh.punch_one(to);
            }
            next_punch = now + sh.punch_every;
        }
        let w = if punching { next_punch.saturating_duration_since(now).max(Duration::from_millis(1)) } else { Duration::from_millis(100) };
        if w != wait {
            wait = w;
            let _ = sh.sock.set_read_timeout(Some(w));
        }
        // Errors include Windows' WSAECONNRESET after an ICMP "port unreachable": keep going.
        let Ok((n, mut from)) = sh.sock.recv_from(&mut buf) else { continue };
        let Some((peer, relayed)) = sh.link.unwrap(from, &buf[..n]) else { continue }; // else a TURN answer, delivered to the waiting request
        from = peer;
        let pkt = relayed.as_deref().unwrap_or(&buf[..n]);
        if crate::remote::stun::is_stun(pkt) {
            continue; // a late STUN answer
        }
        let Some((h, p)) = sh.crypt.open(Dir::PhoneToHost, pkt) else { continue };
        if relayed.is_some() && sh.link.mark_relayed(from) {
            crate::log::line(&format!("turn: phone {from} reached us through the relay"));
        }
        last_rx = Instant::now();
        match h.typ {
            Type::Probe if p.len() >= 16 && h.flags & secpar_stream::header::PATH_TEST != 0 => {
                // A client timing another path: answer there, the stream stays put.
                let mut r = hdr(Type::Probe, 0);
                r.flags = secpar_stream::header::PATH_TEST;
                sh.send_to(from, r, &probe(u64::from_be_bytes(p[..8].try_into().unwrap())));
            }
            Type::Probe if p.len() >= 16 && probe_seq.is_none_or(|s| h.seq > s) => {
                probe_seq = Some(h.seq);
                let prev = sh.peer.lock().unwrap().replace(from);
                if prev != Some(from) {
                    crate::log::line(&format!("stream: phone at {from}{}", if prev.is_some() { " (moved)" } else { "" }));
                }
                let first = prev.is_none();
                if first {
                    sh.idr.store(true, Ordering::Relaxed);
                }
                sh.send(hdr(Type::Probe, 0), &probe(u64::from_be_bytes(p[..8].try_into().unwrap())));
            }
            Type::Input => {
                let evs = dedup.take(&p);
                if input_log.elapsed() >= Duration::from_secs(5) || input_pkts == 0 {
                    crate::log::line(&format!("stream: input packet {} bytes, {} new events (packets so far {input_pkts})", p.len(), evs.len()));
                    input_log = Instant::now();
                }
                input_pkts += 1;
                evs.into_iter().for_each(&mut *input)
            }
            Type::Report => {
                if let Some((loss, rtt, queue)) = parse_report(&p) {
                    if let Some(t) = br.on_report_queue(start.elapsed(), loss, rtt, queue) {
                        if t.notch != sh.target.lock().unwrap().notch || t.kbps * 4 < sh.target.lock().unwrap().kbps * 3 {
                            crate::log::line(&format!("bitrate: {} kbps, fec {}%, notch {} (loss {loss}‰, rtt {rtt} ms, queue {queue} ms)", t.kbps, t.fec, t.notch));
                        }
                        *sh.target.lock().unwrap() = t;
                    }
                }
            }
            Type::Idr if last_idr.is_none_or(|t| t.elapsed() >= IDR_EVERY) => {
                last_idr = Some(Instant::now());
                sh.idr.store(true, Ordering::Relaxed);
                if secpar_stream::timing::enabled() {eprintln!("frame_timing stage=host.key_request");}
            }
            Type::Nack => {
                if let Some((frame_id, frags)) = secpar_stream::nack::decode(&p) {
                    let sent = sh.resend(frame_id, &frags);
                    resent += sent as u64;
                    if resent_log.elapsed() >= Duration::from_secs(10) && resent > 0 {
                        crate::log::line(&format!("stream: resent {resent} lost packets in the last 10 s"));
                        (resent, resent_log) = (0, Instant::now());
                    }
                }
            }
            Type::Bye => break,
            _ => {}
        }
    }
    sh.stop.store(true, Ordering::Relaxed);
}

/// What `stream.ready` repeats on every resolution change.
#[derive(Clone)]
struct ReadyInfo {
    codec: Codec,
    fps: u32,
    port: u16,
    session: u32,
    key: String,
    cands: Vec<String>,
}

impl ReadyInfo {
    fn msg(&self, (w, h): (u32, u32), packed: bool) -> ServerMsg {
        ServerMsg::StreamReady {
            chroma: packed.then(|| "444".into()),
            codec: codec_name(self.codec).into(),
            w,
            h,
            fps: self.fps,
            port: self.port,
            session: self.session,
            key: self.key.clone(),
            cands: self.cands.clone(),
        }
    }
}

/// Capture → convert → encode → packetize, on its own thread (D3D/MF objects stay on it).
struct Video {
    dev: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    ctx: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    luid: i64,
    cap: SharedCapture,
    enc: Encoder,
    /// Stream size at notch 0.
    base: (u32, u32),
    fps: u32,
    /// The display being captured.
    shown: capture::Display,
    /// Frames carry 4:4:4 packed: the encoder's frame is twice the picture's height.
    packed: bool,
}

impl Video {
    #[allow(clippy::result_large_err)] // once per stream start
    fn open(p: &Params, kbps: u32) -> Result<Video, ServerMsg> {
        let err = |code: &str, msg: &str| ServerMsg::StreamError { code: code.into(), msg: msg.into() };
        crate::win::desk::wake_display(); // a display asleep from inactivity has nothing to capture
        let cap = SharedCapture::open(&p.display).ok_or_else(|| err("capture_failed", "the desktop can't be captured right now (locked?)"))?;
        let (capture_dev, capture_ctx, luid, shown) = cap.device();
        let (dev, ctx) = if capture::isolated_gpu() {
            // Conversion and encoding get their own device, avoiding DXGI
            // capture's immediate-context lock while the encoder waits for output.
            let (dev, ctx, _, adapter) = capture::device(&shown.id)
                .ok_or_else(|| err("no_encoder", "the encoder GPU device could not be opened"))?;
            if adapter != luid { return Err(err("no_encoder", "capture and encoder GPUs do not match")); }
            (dev, ctx)
        } else { (capture_dev, capture_ctx) };
        let base=fit(cap.size().unwrap_or((1,1)),(p.w,p.h));
        let fps=p.fps.clamp(1,240);
        let mut failure="No supported client codec".to_string();
        let mut encoder=None;
        // 4:4:4 first when asked for; an encoder that refuses the taller frame falls back to 4:2:0.
        let tries: &[bool] = if p.chroma444 && can_pack(base) { &[true, false] } else { &[false] };
        'tries: for &packed in tries {
            let frame = if packed { (base.0, base.1 * 2) } else { base };
            // The client's first codec that opens, unless the GPU can only run it in its slow mode
            // and a later one runs in its fast mode (HEVC on older Intel GPUs: about 30 fps).
            let mut slow=None;
            for codec in p.codecs.iter().filter_map(|c|parse_codec(c)) {
                match Encoder::selected(&dev,luid,codec,frame,fps,kbps,&p.encoder) {
                    Ok(found) if found.fast()=>{encoder=Some((found,packed));break 'tries;}
                    Ok(found)=>{if slow.is_none(){slow=Some(found);}}
                    Err(error)=>failure=error.message(),
                }
            }
            if let Some(found)=slow {encoder=Some((found,packed));break 'tries;}
        }
        let (enc,packed)=encoder.ok_or_else(||err("no_encoder",&failure))?;
        Ok(Video { dev, ctx, luid, cap, enc, base, fps, shown, packed })
    }

    /// The picture the client shows: the encoder's frame without the packing.
    fn picture(&self) -> (u32, u32) {
        let (w, h) = self.enc.size;
        if self.packed { (w, h / 2) } else { (w, h) }
    }

    /// After a failed encode: logs it and opens the encoder again, so one bad frame does not end
    /// the stream. False when the encoder can't be reopened or keeps failing.
    fn recover(&mut self, e: &windows::core::Error, fails: &mut u32, kbps: u32, settings: &super::encode::Settings) -> bool {
        *fails += 1;
        crate::log::line(&format!("stream: encode failed ({}): {}", *fails, e.message()));
        if *fails > 3 {
            crate::log::line("stream: the encoder keeps failing; ending the stream");
            return false;
        }
        match Encoder::selected(&self.dev, self.luid, self.enc.codec(), self.enc.size, self.fps, kbps, settings) {
            Ok(enc) => {
                self.enc = enc;
                true
            }
            Err(e) => {
                crate::log::line(&format!("stream: the encoder could not be reopened: {}", e.message()));
                false
            }
        }
    }

    fn converter(&self, input: (u32, u32)) -> Option<Convert> {
        Convert::with_chroma(&self.dev, &self.ctx, input, self.picture(), self.packed).ok()
    }

    fn info(&self) -> GroupInfo {
        GroupInfo { shown: self.shown.id.clone(), codec: self.enc.codec(), packed: self.packed, fps: self.fps, base: self.base, picture: self.picture() }
    }

    /// Points the viewer's input at the display and lists the displays, with the pointer's shape.
    fn show_to(&self, m: &Member, shape: &Option<ServerMsg>) {
        *m.p.rect.lock().unwrap() = Some(self.shown.rect);
        (m.notify)(ServerMsg::StreamDisplays { list: capture::displays(), current: self.shown.id.clone() });
        if let Some(s) = shape {
            (m.notify)(s.clone());
        }
    }

    /// A `stream.ready` when the viewer's picture changed (or `always`, after a display switch).
    fn announce(&self, m: &mut Member, always: bool) {
        let now = (self.enc.codec(), self.picture(), self.packed);
        if always || m.announced != Some(now) {
            m.ready.codec = now.0;
            (m.notify)(m.ready.msg(now.1, now.2));
            m.announced = Some(now);
        }
    }

    fn run(mut self, group: &Group, mut members: Vec<Member>) {
        // A host on Wi-Fi: no background scans while it streams (12-29% loss seen on a T480).
        let _wlan = secpar_network::wlan::hold(Some(crate::log::line));
        // The host's encoder settings: the same in every viewer's parameters.
        let settings = members[0].p.encoder.clone();
        let mut interval = stream_interval(self.fps, self.cap.refresh_rate());
        let mut work_budget = interval.min(Duration::from_millis(10));
        let mut pending = false;
        // One send per frame interval, sampled from the newest captured image. Sending on DXGI
        // delivery made arrival spacing follow capture jitter, which a client on a fixed refresh
        // turns into held and doubled pictures.
        let mut next_due = Instant::now();
        let mut started = false;
        let mut source_clock = false;
        let mut last_real = Instant::now() - interval * 2;
        let mut last_send = Instant::now() - interval * 2;
        // Encoded units wait here for their send slot. Encoding never runs at the deadline.
        let mut prepared: std::collections::VecDeque<super::encode::Encoded> = std::collections::VecDeque::new();
        let mut prepared_packets: Option<Arc<Vec<(Header,Vec<u8>)>>> = None;
        let mut img: Option<Arc<capture::FrameTexture>> = None;
        let mut conv: Option<Convert> = None;
        let (mut frame_id, mut cap_us) = (0u32, 0u64);
        let mut applied = combined(&members);
        let (mut pointer_visible, mut hotspot) = (true, (0i32, 0i32));
        // The pointer's shape, for viewers that join later.
        let mut shape: Option<ServerMsg> = None;
        let mut want = (members[0].p.w, members[0].p.h);
        let mut lats: Vec<u64> = Vec::new();
        let mut fails = 0u32;
        let mut key_all = false;
        let mut st = Pace::new();
        for m in &mut members {
            self.announce(m, false);
            self.show_to(m, &shape);
        }
        loop {
            st.maybe_log();
            let joined = std::mem::take(&mut *group.joining.lock().unwrap());
            for mut m in joined {
                self.announce(&mut m, false);
                self.show_to(&m, &shape);
                members.push(m);
                crate::log::line(&format!("stream: a viewer joined the {} stream ({} watching)", self.shown.id, members.len()));
            }
            members.retain(Member::keep);
            if members.is_empty() {
                if group.close(false) {
                    break;
                }
                continue;
            }
            for m in &mut members {
                // A viewer starts on a key frame: the first one after its address is known.
                if !m.started && !m.key_asked && m.sh.peer().is_some() {
                    m.key_asked = true;
                    key_all = true;
                }
            }
            let t = combined(&members);
            if members.len() == 1 {
                let (new_display, new_size, mp) = {
                    let m = &members[0];
                    let d = m.sh.switch.lock().unwrap().take().filter(|id| *id != self.shown.id);
                    let s = m.sh.resize.lock().unwrap().take().filter(|s| *s != want);
                    (d, s, m.p.clone())
                };
                if new_display.is_some() || new_size.is_some() {
                    want = new_size.unwrap_or(want);
                    let changed_display = new_display.is_some();
                    // same codec (the decoder is set up for it), then the current notch below
                    let p = Params { display: new_display.unwrap_or_else(|| self.shown.id.clone()), w: want.0, h: want.1, codecs: vec![codec_name(self.enc.codec()).into()], ..mp };
                    if let Ok(v) = Video::open(&p, t.kbps) {
                        crate::log::line(&format!("stream: switched to {}", v.shown.id));
                        self = v;
                        interval = stream_interval(self.fps, self.cap.refresh_rate());
                        work_budget = interval.min(Duration::from_millis(10));
                        source_clock = false;
                        prepared.clear();
                        prepared_packets = None;
                        (img, conv, pending, members[0].cursor) = (None, None, false, None);
                        applied.notch = 0;
                        key_all = true;
                        *group.info.lock().unwrap() = self.info();
                        self.announce(&mut members[0], true);
                        self.show_to(&members[0], &shape);
                        if changed_display {
                            let [l, t, r, b] = self.shown.rect;
                            // SAFETY: move only for an explicitly selected display, never a stream resize.
                            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos((l + r) / 2, (t + b) / 2) };
                        }
                    }
                }
            } else {
                // A shared picture keeps its size until one viewer has it alone. A viewer moving
                // to another display leaves for that display's pipeline.
                let mut i = 0;
                while i < members.len() {
                    let id = members[i].sh.switch.lock().unwrap().take().filter(|id| *id != self.shown.id);
                    match id {
                        Some(id) => {
                            let mut m = members.remove(i);
                            m.p.display = id;
                            (m.announced, m.started, m.key_asked, m.cursor) = (None, false, false, None);
                            place(m);
                        }
                        None => i += 1,
                    }
                }
            }
            if t.notch != applied.notch {
                let size = notch_size(self.base.0, self.base.1, t.notch);
                let frame = if self.packed { (size.0, size.1 * 2) } else { size };
                match Encoder::selected(&self.dev, self.luid, self.enc.codec(), frame, self.fps, t.kbps, &settings) {
                    Ok(e) => {
                        self.enc = e;
                        prepared.clear();
                        prepared_packets = None;
                        conv = None;
                        *group.info.lock().unwrap() = self.info();
                        for m in &mut members {
                            self.announce(m, true);
                        }
                    }
                    Err(e) => {
                        crate::log::line(&format!("stream: no encoder at the new size: {}", e.message()));
                        break;
                    }
                }
            } else if t.kbps != applied.kbps {
                let _ = self.enc.set_bitrate(t.kbps);
            }
            applied = t;

            // Pull the newest capture without waiting, then convert and encode it at once. The
            // send deadline then only pays for packetising, so arrival spacing is the grid.
            // The pointer straight from Windows, every pass: capture reports pointer moves late on
            // some displays (a virtual display gets them only with new frames), so the drawn cursor
            // trailed behind. Capture still says whether it is visible and gives its shape.
            let mut at = windows::Win32::Foundation::POINT::default();
            // SAFETY: plain out-param call. It fails off the input desktop; capture's position is used then.
            let polled = unsafe { windows::Win32::UI::WindowsAndMessaging::GetPhysicalCursorPos(&mut at) }.is_ok();
            if polled {
                // Desktop pixels → the picture: the pointer's hot spot, where clients put the shape's hotspot.
                let [l, t, r, b] = self.shown.rect;
                let (pw, ph) = self.picture();
                let inside = at.x >= l && at.x < r && at.y >= t && at.y < b;
                let c = ((at.x - l) * pw as i32 / (r - l).max(1), (at.y - t) * ph as i32 / (b - t).max(1), inside && pointer_visible);
                for m in &mut members {
                    m.pointer(c);
                }
            }
            if let Some(f) = self.cap.next(Duration::ZERO) {
                let desk = self.cap.size().unwrap_or(self.base);
                if let Some(p) = &f.pointer {
                    pointer_visible = p.visible;
                    if !polled {
                        // Positions are for the picture, not a packed (4:4:4) frame twice its height.
                        let (sw, sh_) = self.picture();
                        // Capture gives the shape's top-left corner; clients place the hot spot.
                        let c = ((p.x + hotspot.0) * sw as i32 / desk.0.max(1) as i32, (p.y + hotspot.1) * sh_ as i32 / desk.1.max(1) as i32, p.visible);
                        for m in &mut members {
                            m.pointer(c);
                        }
                    }
                }
                if let Some(s) = &f.shape {
                    hotspot = (s.hx, s.hy);
                }
                if let Some(png) = f.shape.as_ref().and_then(|s| cursor::png(s).map(|p| (p, s.hx, s.hy))) {
                    let msg = ServerMsg::CursorShape { png: STANDARD.encode(&png.0), hx: png.1, hy: png.2 };
                    for m in &members {
                        (m.notify)(msg.clone());
                    }
                    shape = Some(msg);
                }
                if f.tex.is_some() {
                    st.captured += 1;
                    img = f.tex.clone();
                    cap_us = f.capture_us;
                    pending = true;
                    if !source_clock {
                        if let Some(due) = source_deadline(cap_us, now_us(), Instant::now(), work_budget) {
                            next_due = due;
                            source_clock = true;
                            if secpar_stream::timing::enabled() { eprintln!("frame_timing stage=host.source_clock budget_us={} period_ns={}", work_budget.as_micros(), interval.as_nanos()); }
                        } else if !started {
                            next_due = Instant::now() + work_budget;
                        }
                        started = true;
                    }
                }
            }
            if std::mem::take(&mut self.cap.reinit) {
                conv = None;
                interval = stream_interval(self.fps, self.cap.refresh_rate());
                work_budget = interval.min(Duration::from_millis(10));
                source_clock = false;
                key_all = true;
            }
            let watched = members.iter().any(|m| m.sh.peer().is_some());
            // Sample the source slot, not an old capture encoded early and held
            // for a send timer. Faster capture rates keep only their newest image.
            let encode_due = next_due.checked_sub(work_budget).unwrap_or(next_due);
            let now = Instant::now();
            // A frame not sent yet also counts when it is at most one interval old. A game that
            // presents its own 60 fps drifts against this grid; requiring its frames to land inside
            // each slot turned about every other one into a repeat (30 fps from a 60 fps game).
            let unsent_fresh = pending && now_us().saturating_sub(cap_us) <= interval.as_micros() as u64;
            let current = !source_clock || unsent_fresh || source_in_slot(cap_us, now_us(), now, encode_due);
            if pending && prepared.is_empty() && watched && now >= encode_due && current {
                if let (Some(src), Some(size)) = (img.as_ref(), self.cap.size()) {
                    if conv.is_none() {
                        conv = self.converter(size);
                    }
                    let timing = secpar_stream::timing::begin();
                    if let Some(nv12) = conv.as_mut().and_then(|c| {let t0=Instant::now();let r=c.convert_frame(src).ok();st.convert.add(t0.elapsed());r}) {
                        secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Convert, timing);
                        let timing = secpar_stream::timing::begin();
                        let force = take_key(&members, &mut key_all);
                        if force && secpar_stream::timing::enabled() {eprintln!("frame_timing stage=host.key_encode repeat=false");}
                        let t0 = Instant::now();
                        let encoded = self.enc.encode(&nv12, cap_us, force);
                        st.encode.add(t0.elapsed());
                        match encoded {
                            Ok(outs) => {
                                secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Encode, timing);
                                prepared.extend(outs);
                                pending = false;
                                fails = 0;
                            }
                            Err(e) => {
                                if !self.recover(&e, &mut fails, t.kbps, &settings) {
                                    break;
                                }
                                key_all = true; // a new encoder starts with a key frame
                            }
                        }
                    }
                }
            }
            if prepared_packets.is_none() {
                if let Some(e)=prepared.front() {prepared_packets=Some(Arc::new(encoded_packets(frame_id,e,self.enc.codec(),t.fec as usize)));}
            }
            let first_due=prepared_packets.as_deref().map(|packets| {
                let (_,offset)=video_packet_schedule(packets);
                next_due.checked_sub(Duration::from_millis(3-offset)).unwrap_or(next_due)
            }).unwrap_or(next_due);
            let now = Instant::now();
            if now < first_due {
                // DXGI's own timeouts run on the coarse system tick: poll at 1 ms.
                std::thread::sleep((first_due - now).min(Duration::from_millis(1)));
                continue;
            }
            // Due: one encoded picture. A screen still for half a second repeats at half rate,
            // so an idle desktop does not spend a full band on identical pictures.
            secpar_stream::timing::mark(secpar_stream::timing::Stage::Selected);
            let frame_due=next_due;
            next_due += interval;
            if next_due <= now {
                next_due = now + interval;
                source_clock = false; // reacquire the capture phase after a real stall
            }
            let encode_now = prepared.is_empty();
            let repeat = encode_now && !pending;
            if repeat && !idle_repeat_due(last_real.elapsed(), last_send.elapsed(), interval) {
                continue;
            }
            if encode_now {
                let (Some(src), Some(size)) = (img.as_ref().filter(|_| watched), self.cap.size()) else { continue };
                if conv.is_none() {
                    conv = self.converter(size);
                }
                let timing = secpar_stream::timing::begin();
                let Some(nv12) = conv.as_mut().and_then(|c| {let t0=Instant::now();let r=c.convert_frame(src).ok();st.convert.add(t0.elapsed());r}) else { continue };
                secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Convert, timing);
                let timing = secpar_stream::timing::begin();
                let force = take_key(&members, &mut key_all);
                if force && secpar_stream::timing::enabled() {eprintln!("frame_timing stage=host.key_encode repeat=true");}
                let stamp = if pending { cap_us } else { now_us() };
                let t0 = Instant::now();
                let encoded = self.enc.encode(&nv12, stamp, force);
                st.encode.add(t0.elapsed());
                let outs = match encoded {
                    Ok(outs) => {
                        fails = 0;
                        outs
                    }
                    Err(e) => {
                        if !self.recover(&e, &mut fails, t.kbps, &settings) {
                            break;
                        }
                        key_all = true;
                        continue;
                    }
                };
                secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Encode, timing);
                pending = false;
                prepared.extend(outs);
            }
            let Some(e) = prepared.pop_front() else { continue };
            if e.key && secpar_stream::timing::enabled() {eprintln!("frame_timing stage=host.key_output");}
            secpar_stream::timing::mark(secpar_stream::timing::Stage::Encoded);
            let packets=prepared_packets.take().unwrap_or_else(||Arc::new(encoded_packets(frame_id,&e,self.enc.codec(),t.fec as usize)));
            let timing=secpar_stream::timing::begin();
            for m in &mut members {
                m.deliver(&packets, frame_due, e.key);
            }
            st.sent += 1;
            st.repeats += repeat as u32;
            secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Send,timing);
            secpar_stream::timing::mark(secpar_stream::timing::Stage::Sent);
            last_send = Instant::now();
            if !repeat {
                last_real = last_send;
                let age = now_us().saturating_sub(e.capture_us);
                if secpar_stream::timing::enabled() {
                    lats.push(age);
                    if lats.len() == 300 {
                        let mut s = lats.clone(); s.sort_unstable();
                        let g = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize];
                        eprintln!("frame_timing stage=host.capture_to_sent n={} p50_us={} p95_us={} max_us={}", s.len(), g(0.5), g(0.95), s.last().copied().unwrap_or(0));
                        lats.clear();
                    }
                }
            }
            frame_id = frame_id.wrapping_add(1);
        }
        group.close(true);
        for m in &members {
            m.sh.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Starts a pipeline for its first viewer and lets others join it.
    fn lead(self, first: Member) {
        let group = Arc::new(Group { info: Mutex::new(self.info()), joining: Mutex::default(), closed: AtomicBool::new(false) });
        GROUPS.lock().unwrap().push(group.clone());
        self.run(&group, vec![first]);
    }
}

/// Packets of one frame, and when its sending should finish.
type Outgoing = (Arc<Vec<(Header, Vec<u8>)>>, Instant);

/// One viewer of a pipeline: its own socket, keys, address and messages, fed by the shared encoder.
struct Member {
    sh: Arc<Shared>,
    p: Params,
    notify: Notify,
    ready: ReadyInfo,
    /// To this viewer's sending thread, which paces the packets out.
    out: std::sync::mpsc::SyncSender<Outgoing>,
    /// What its last `stream.ready` said; None when it needs a new one.
    announced: Option<(Codec, (u32, u32), bool)>,
    /// Frames go out from the first key frame on: a viewer that joins mid-stream can't decode before it.
    started: bool,
    key_asked: bool,
    cursor: Option<(i32, i32, bool)>,
}

impl Member {
    /// Still watching. Checked before every frame: a friend never follows the owner onto the lock screen.
    fn keep(&self) -> bool {
        if self.sh.stopped() {
            return false;
        }
        if let Some(g) = &self.p.gate {
            if crate::win::desk::locked_or_unknown() {
                g.close_because("locked");
            }
            if !g.view() {
                self.sh.stop.store(true, Ordering::Relaxed);
                return false;
            }
        }
        true
    }

    fn pointer(&mut self, c: (i32, i32, bool)) {
        if self.cursor != Some(c) && self.sh.send(hdr(Type::Cursor, 0), &cursor::packet(c.0, c.1, c.2)) {
            self.cursor = Some(c);
        }
    }

    fn deliver(&mut self, packets: &Arc<Vec<(Header, Vec<u8>)>>, due: Instant, key: bool) {
        if self.sh.peer().is_none() || !(self.started || key) {
            return;
        }
        self.started = true;
        // Full only when its sending is a few frames behind; the client asks for what it missed.
        let _ = self.out.try_send((packets.clone(), due));
    }
}

/// What the shared encoder runs at: the lowest bitrate and size any viewer's link takes, and the
/// most FEC any asks for.
fn combined(members: &[Member]) -> Target {
    let mut t = *members[0].sh.target.lock().unwrap();
    for m in &members[1..] {
        let o = *m.sh.target.lock().unwrap();
        t.kbps = t.kbps.min(o.kbps);
        t.notch = t.notch.max(o.notch);
        t.fec = t.fec.max(o.fec);
    }
    t
}

/// A key frame when any viewer asked for one.
fn take_key(members: &[Member], all: &mut bool) -> bool {
    let mut key = std::mem::take(all);
    for m in members {
        key |= m.sh.idr.swap(false, Ordering::Relaxed);
    }
    key
}

/// Viewers of one display share its capture and one encoder: a T480's Quick Sync ran two 60 fps
/// encodes but not three. Viewers joining wait in `joining` for the video thread.
struct Group {
    info: Mutex<GroupInfo>,
    joining: Mutex<Vec<Member>>,
    closed: AtomicBool,
}

#[derive(Clone)]
struct GroupInfo {
    shown: String,
    codec: Codec,
    packed: bool,
    fps: u32,
    base: (u32, u32),
    picture: (u32, u32),
}

static GROUPS: Mutex<Vec<Arc<Group>>> = Mutex::new(Vec::new());

impl GroupInfo {
    /// A viewer can watch this pipeline: same display, a codec it decodes, 4:4:4 only if it shows it.
    fn fits(&self, p: &Params, shown: Option<&str>) -> bool {
        shown == Some(self.shown.as_str())
            && p.codecs.iter().filter_map(|c| parse_codec(c)).any(|c| c == self.codec)
            && (!self.packed || p.chroma444)
    }
}

impl Group {
    /// Hands the viewer back when the pipeline closed in the meantime.
    fn add(&self, m: Member) -> Result<(), Member> {
        let mut joining = self.joining.lock().unwrap();
        if self.closed.load(Ordering::Relaxed) {
            return Err(m);
        }
        joining.push(m);
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
        for m in joining.drain(..) {
            m.sh.stop.store(true, Ordering::Relaxed);
        }
        true
    }
}

/// The display a stream of `display` captures (empty or unknown: the primary one).
fn shown_for(display: &str) -> Option<String> {
    let all = capture::displays();
    all.iter().find(|d| !display.is_empty() && d.id == display).or_else(|| all.iter().find(|d| d.primary)).map(|d| d.id.clone())
}

/// The open pipeline a viewer with these parameters can join.
fn find_group(p: &Params) -> Option<(Arc<Group>, GroupInfo)> {
    let shown = shown_for(&p.display);
    let groups = GROUPS.lock().unwrap();
    groups.iter().find_map(|g| {
        let info = g.info.lock().unwrap().clone();
        (!g.closed.load(Ordering::Relaxed) && info.fits(p, shown.as_deref())).then(|| (g.clone(), info))
    })
}

/// Puts a viewer into the pipeline already streaming its display, or starts one for it.
fn place(m: Member) {
    let m = match find_group(&m.p) {
        Some((g, _)) => match g.add(m) {
            Ok(()) => return,
            Err(m) => m,
        },
        None => m,
    };
    std::thread::spawn(move || {
        let _com = Com::init();
        let _prio = Realtime::enter();
        let kbps = m.sh.target.lock().unwrap().kbps;
        match Video::open(&m.p, kbps) {
            Ok(v) => v.lead(m),
            Err(e) => {
                crate::log::line(&format!("stream: no pipeline for {}: {e:?}", m.p.display));
                m.sh.stop.store(true, Ordering::Relaxed);
            }
        }
    });
}

/// Ten seconds of the video thread in one log line: where a host short of frames loses them
/// (capture, conversion, encoding), without the frame-timing build.
struct Pace {
    since: Instant,
    captured: u32,
    sent: u32,
    repeats: u32,
    convert: Spread,
    encode: Spread,
}

#[derive(Default)]
struct Spread {
    n: u32,
    total: Duration,
    max: Duration,
}

impl Spread {
    fn add(&mut self, d: Duration) {
        self.n += 1;
        self.total += d;
        self.max = self.max.max(d);
    }
    fn text(&self) -> String {
        let avg = if self.n == 0 { 0. } else { self.total.as_secs_f64() * 1000. / self.n as f64 };
        format!("{avg:.1} ms avg, {:.1} max", self.max.as_secs_f64() * 1000.)
    }
}

impl Pace {
    fn new() -> Self {
        Pace { since: Instant::now(), captured: 0, sent: 0, repeats: 0, convert: Spread::default(), encode: Spread::default() }
    }
    fn maybe_log(&mut self) {
        let secs = self.since.elapsed().as_secs_f64();
        if secs < 10. {
            return;
        }
        crate::log::line(&format!(
            "stream: per second {:.0} captured, {:.0} sent ({:.0} repeats); convert {}; encode {}",
            self.captured as f64 / secs, self.sent as f64 / secs, self.repeats as f64 / secs, self.convert.text(), self.encode.text()
        ));
        *self = Pace::new();
    }
}

/// Frame-pacing scheduling for the video thread: a 1 ms system timer so
/// sleeps land on the frame deadline (the default tick is 15.6 ms), and the "Games" MMCSS class
/// so a busy desktop doesn't delay capture and encode.
struct Realtime(Option<usize>);

impl Realtime {
    fn enter() -> Realtime {
        use windows::core::w;
        use windows::Win32::Media::timeBeginPeriod;
        use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
        let mut task = 0u32;
        // SAFETY: plain calls with a valid out pointer; undone in Drop.
        unsafe {
            timeBeginPeriod(1);
            let h = AvSetMmThreadCharacteristicsW(w!("Games"), &mut task).ok();
            Realtime(h.map(|h| h.0 as usize))
        }
    }
}

impl Drop for Realtime {
    fn drop(&mut self) {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Media::timeEndPeriod;
        use windows::Win32::System::Threading::AvRevertMmThreadCharacteristics;
        // SAFETY: balances the calls in enter.
        unsafe {
            if let Some(h) = self.0.take() {
                let _ = AvRevertMmThreadCharacteristics(HANDLE(h as *mut _));
            }
            timeEndPeriod(1);
        }
    }
}

/// `stream.ready` candidates: LAN addresses, the UPnP-mapped public one, the STUN-reflexive one.
fn host_cands(lan: &[String], port: u16, public: Option<IpAddr>, reflexive: Option<SocketAddr>) -> Vec<String> {
    link::host_cands(lan, port, STREAM_PORT, public, reflexive) // UPnP maps only the fixed port
}

static ACTIVE: OnceLock<Arc<secpar_host_core::limits::StreamSlots>> = OnceLock::new();

pub struct StreamSession {
    sh: Arc<Shared>,
    threads: Vec<std::thread::JoinHandle<()>>,
    rect: crate::input::Rect,
}

impl StreamSession {
    /// Opens capture and the encoder, binds the UDP socket on `bind` and starts streaming as
    /// soon as the phone's first PROBE arrives. Err is the `stream.error` to send.
    #[allow(clippy::result_large_err)] // once per stream start
    pub fn start(p: Params, bind: IpAddr, notify: Notify, mut input: InputSink) -> Result<(ServerMsg, StreamSession), ServerMsg> {
        let err = |code: &str, msg: &str| ServerMsg::StreamError { code: code.into(), msg: msg.into() };
        let permit = ACTIVE.get_or_init(Default::default).try_acquire()
            .ok_or_else(|| err("connection_limit", secpar_host_core::limits::LIMIT_MESSAGE))?;
        // The fixed port only matters when UPnP mapped it (access from anywhere). Otherwise a fresh
        // port per stream: a relay keeps a killed run's allocation for its address pair (437).
        let fixed = if p.public.is_some() { STREAM_PORT } else { 0 };
        let sock = UdpSocket::bind((bind, fixed))
            .or_else(|_| UdpSocket::bind((bind, 0)))
            .map_err(|e| err("capture_failed", &e.to_string()))?;
        let port = sock.local_addr().map_err(|e| err("capture_failed", &e.to_string()))?.port();
        let key: [u8; 32] = crate::store::random();
        let session = u32::from_be_bytes(crate::store::random());
        let mut shared = Shared::new(sock, &key, session, Bitrate::new(p.max_kbps, 0).target());
        shared._stream_slot = Some(permit);
        let sh = Arc::new(shared);

        // A display already streaming to another viewer: join its pipeline. Otherwise a video
        // thread opens one, reports the codec and size it got, then waits for its first viewer.
        let join = find_group(&p);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let (first_tx, first_rx) = std::sync::mpsc::sync_channel::<Member>(1);
        if join.is_none() {
            let vp = p.clone();
            let kbps = sh.target.lock().unwrap().kbps;
            std::thread::spawn(move || {
                let _com = Com::init();
                let _prio = Realtime::enter();
                match Video::open(&vp, kbps) {
                    Ok(v) => {
                        let _ = tx.send(Ok(v.info()));
                        if let Ok(m) = first_rx.recv() {
                            v.lead(m);
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            });
        }
        // While the encoder opens: this socket's public address, for the phone to punch towards.
        let reflexive = (!bind.is_loopback()).then(|| crate::remote::stun::public_addr(&sh.sock)).flatten();
        let info = match &join {
            Some((_, info)) => info.clone(),
            None => match rx.recv() {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(err("capture_failed", "video pipeline crashed")),
            },
        };
        let lan = crate::commands::rank_ips(crate::win::sys::adapters().0);
        let cands = host_cands(&lan, port, p.public, reflexive);
        let ready = ReadyInfo { codec: info.codec, fps: info.fps, port, session, key: STANDARD.encode(key), cands };
        let (w, h) = info.picture;
        let shared = if join.is_some() { " (shared encoder)" } else { "" };
        crate::log::line(&format!("stream: ready {:?} {w}x{h} on {port}{shared}, host cands {:?}, phone cands {:?}", info.codec, ready.cands, p.cands));
        // Its own thread paces this viewer's packets out, so a slow one never holds up the others.
        let (out, out_rx) = std::sync::mpsc::sync_channel::<Outgoing>(3);
        let member = Member { sh: sh.clone(), p: p.clone(), notify: notify.clone(), ready: ready.clone(), out, announced: Some((info.codec, info.picture, info.packed)), started: false, key_asked: false, cursor: None };
        match join {
            Some((g, _)) => {
                if let Err(m) = g.add(member) {
                    place(m);
                }
            }
            None => {
                let _ = first_tx.send(member);
            }
        }
        sh.punch(p.cands.iter().copied());
        // step-down notches follow the size actually streamed
        let mut br = Bitrate::new(p.max_kbps, notches_below(info.base.1).len() as u8);
        let ssh = sh.clone();
        let sender = std::thread::spawn(move || {
            while let Ok((packets, due)) = out_rx.recv() {
                ssh.send_paced_at(&packets, due);
            }
        });
        let mut threads = vec![sender];
        let csh = sh.clone();
        threads.push(std::thread::spawn(move || control(&csh, &mut input, &mut br, TIMEOUT)));
        if p.audio && p.gate.as_ref().is_none_or(|g| g.has("audio")) {
            let ash = sh.clone();
            threads.push(std::thread::spawn(move || {
                let _com = Com::init();
                let mut n = 0u32;
                audio::run(audio::Wasapi::open, &ash.stop, |opus, us| {
                    let mut plain = Vec::with_capacity(8 + opus.len());
                    plain.extend_from_slice(&us.to_be_bytes());
                    plain.extend_from_slice(opus);
                    if ash.send(hdr(Type::Audio, n), &plain) {
                        n = n.wrapping_add(1);
                    }
                });
            }));
        }
        Ok((ready.msg(info.picture, info.packed), StreamSession { sh, threads, rect: p.rect.clone() }))
    }

    /// A phone candidate from `stream.cand`: punch towards it too, restarting the window.
    pub fn add_cand(&self, a: SocketAddr) {
        self.sh.add_cand(a);
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.sh.clone()
    }

    /// `stream.display`: switch to another display without restarting the stream.
    pub fn switch_display(&self, id: String) {
        *self.sh.switch.lock().unwrap() = Some(id);
    }

    /// `stream.size`: change the stream's size without restarting it.
    pub fn resize(&self, w: u32, h: u32) {
        *self.sh.resize.lock().unwrap() = Some((w.min(7680), h.min(4320)));
    }

    pub fn alive(&self) -> bool {
        !self.sh.stopped()
    }
}

impl Drop for StreamSession {
    fn drop(&mut self) {
        self.sh.stop.store(true, Ordering::Relaxed);
        self.sh.send(hdr(Type::Bye, 0), &[]);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        *self.rect.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests;
