//! One PRS stream to one phone: the UDP socket, PROBE address learning, video/audio threads,
//! input, REPORT-driven bitrate and the 5 s timeout. Dropping it ends everything.

use super::bitrate::{notch_size, notches_below, Bitrate, Target};
use super::capture::{self, SharedCapture};
use super::convert::{fit, Convert};
use super::encode::{codec_name, codecs_available, parse_codec, Encoder};
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
    let (mw,mh,mfps)=match profile {"battery"=>(1280,720,30),"smooth"=>(1920,1080,60),"sharp"=>(2560,1440,60),"2160p60"=>(3840,2160,60),_=>(0,0,0)};
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
    pub audio: bool,
    /// The phone's PRS addresses to punch towards.
    pub cands: Vec<SocketAddr>,
    /// The router's public IP when UPnP mapped the stream port.
    pub public: Option<IpAddr>,
    /// Windows device name of the display to stream; empty = primary.
    pub display: String,
    /// Set for a friend's stream: frames stop when it closes, loses view, or the desktop locks.
    pub gate: Option<secpar_host_core::guest_policy::Gate>,
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
        self.link.transmit(&self.sock, to, &pkt)
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

/// REPORT plaintext: (lossPermille, rttMs).
fn parse_report(p: &[u8]) -> Option<(u16, u16)> {
    (p.len() >= 16).then(|| (u16::from_be_bytes([p[4], p[5]]), u16::from_be_bytes([p[10], p[11]])))
}

fn probe(echo: u64) -> [u8; 16] {
    let mut p = [0u8; 16];
    p[..8].copy_from_slice(&now_us().to_be_bytes());
    p[8..].copy_from_slice(&echo.to_be_bytes());
    p
}

/// The receive loop: runs until BYE, `timeout` of silence (punching extends it), or `stop`.
pub fn control(sh: &Shared, input: &mut dyn FnMut(ClientMsg), br: &mut Bitrate, timeout: Duration) {
    let start = Instant::now();
    let (mut wait, mut next_punch) = (Duration::ZERO, start);
    let (mut last_rx, mut last_idr, mut probe_seq) = (Instant::now(), None::<Instant>, None::<u32>);
    let mut dedup = Dedup::default();
    let (mut input_pkts, mut input_log) = (0u64, Instant::now());
    let mut buf = [0u8; 2048];
    loop {
        let punching = sh.punching();
        if sh.stopped() || (last_rx.elapsed() >= timeout && !punching) {
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
                if let Some((loss, rtt)) = parse_report(&p) {
                    if let Some(t) = br.on_report(start.elapsed(), loss, rtt) {
                        if t.notch != sh.target.lock().unwrap().notch || t.kbps * 4 < sh.target.lock().unwrap().kbps * 3 {
                            crate::log::line(&format!("bitrate: {} kbps, fec {}%, notch {} (loss {loss}‰, rtt {rtt} ms)", t.kbps, t.fec, t.notch));
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
    fn msg(&self, (w, h): (u32, u32)) -> ServerMsg {
        ServerMsg::StreamReady {
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
}

impl Video {
    #[allow(clippy::result_large_err)] // once per stream start
    fn open(p: &Params, kbps: u32) -> Result<Video, ServerMsg> {
        let err = |code: &str, msg: &str| ServerMsg::StreamError { code: code.into(), msg: msg.into() };
        crate::win::desk::wake_display(); // a display asleep from inactivity has nothing to capture
        let cap = SharedCapture::open(&p.display).ok_or_else(|| err("capture_failed", "the desktop can't be captured right now (locked?)"))?;
        let (dev, ctx, luid, shown) = cap.device();
        let avail = codecs_available(luid);
        let codec = p
            .codecs
            .iter()
            .filter_map(|c| parse_codec(c))
            .find(|c| avail.contains(c))
            .ok_or_else(|| err("no_encoder", "this PC's GPU can't encode any codec the phone decodes"))?;
        let base = fit(cap.size().unwrap_or((1,1)), (p.w, p.h));
        let fps = p.fps.clamp(1, 240);
        let enc = Encoder::new(&dev, luid, codec, base, fps, kbps).map_err(|e| err("no_encoder", &e.message()))?;
        Ok(Video { dev, ctx, luid, cap, enc, base, fps, shown })
    }

    fn shown(&self, notify: &Notify) {
        *crate::input::STREAM_RECT.lock().unwrap() = Some(self.shown.rect);
        notify(ServerMsg::StreamDisplays { list: capture::displays(), current: self.shown.id.clone() });
    }

    fn run(mut self, sh: &Shared, ready: &ReadyInfo, notify: &Notify, p: &Params) {
        self.shown(notify);
        let interval = Duration::from_secs(1) / self.fps;
        let mut pending = false;
        // One send per frame interval, sampled from the newest captured image. Sending on DXGI
        // delivery made arrival spacing follow capture jitter, which a client on a fixed refresh
        // turns into held and doubled pictures.
        let mut next_due = Instant::now();
        let mut started = false;
        let mut last_real = Instant::now() - interval * 2;
        let mut last_send = Instant::now() - interval * 2;
        // Encoded units wait here for their send slot. Encoding never runs at the deadline.
        let mut prepared: std::collections::VecDeque<super::encode::Encoded> = std::collections::VecDeque::new();
        let mut prepared_packets: Option<Vec<(Header,Vec<u8>)>> = None;
        let mut img: Option<Arc<capture::FrameTexture>> = None;
        let mut conv: Option<Convert> = None;
        let (mut frame_id, mut cap_us) = (0u32, 0u64);
        let mut applied = *sh.target.lock().unwrap();
        let mut last_cursor = None;
        let mut want = (p.w, p.h);
        let mut lats: Vec<u64> = Vec::new();
        let mut warm: Vec<u64> = Vec::new();
        let mut phase_done = false;
        while !sh.stopped() {
            if let Some(g) = &p.gate {
                // Checked before every frame: a friend never follows the owner onto the lock screen.
                if crate::win::desk::locked_or_unknown() {
                    g.close_because("locked");
                }
                if !g.view() {
                    break;
                }
            }
            let t = *sh.target.lock().unwrap();
            let new_display = sh.switch.lock().unwrap().take().filter(|id| *id != self.shown.id);
            let new_size = sh.resize.lock().unwrap().take().filter(|s| *s != want);
            if new_display.is_some() || new_size.is_some() {
                want = new_size.unwrap_or(want);
                // same codec (the decoder is set up for it), then the current notch below
                let p = Params { display: new_display.unwrap_or_else(|| self.shown.id.clone()), w: want.0, h: want.1, codecs: vec![codec_name(self.enc.codec()).into()], ..p.clone() };
                if let Ok(v) = Video::open(&p, t.kbps) {
                    crate::log::line(&format!("stream: switched to {}", v.shown.id));
                    self = v;
                    prepared.clear();
                    prepared_packets = None;
                    (img, conv, pending, last_cursor) = (None, None, false, None);
                    applied.notch = 0;
                    sh.idr.store(true, Ordering::Relaxed);
                    notify(ready.msg(self.enc.size));
                    self.shown(notify);
                    let [l, t, r, b] = self.shown.rect;
                    // SAFETY: plain cursor move; the pointer follows the view to the new display.
                    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos((l + r) / 2, (t + b) / 2) };
                }
            }
            if t.notch != applied.notch {
                let size = notch_size(self.base.0, self.base.1, t.notch);
                match Encoder::new(&self.dev, self.luid, self.enc.codec(), size, self.fps, t.kbps) {
                    Ok(e) => {
                        self.enc = e;
                        prepared.clear();
                        prepared_packets = None;
                        conv = None;
                        notify(ready.msg(size));
                    }
                    Err(_) => break,
                }
            } else if t.kbps != applied.kbps {
                let _ = self.enc.set_bitrate(t.kbps);
            }
            applied = t;

            // Pull the newest capture without waiting, then convert and encode it at once. The
            // send deadline then only pays for packetising, so arrival spacing is the grid.
            if let Some(f) = self.cap.next(Duration::ZERO) {
                let desk = self.cap.size().unwrap_or(self.base);
                if let Some(p) = &f.pointer {
                    let (sw, sh_) = self.enc.size;
                    let c = (p.x * sw as i32 / desk.0.max(1) as i32, p.y * sh_ as i32 / desk.1.max(1) as i32, p.visible);
                    if last_cursor != Some(c) {
                        last_cursor = Some(c);
                        sh.send(hdr(Type::Cursor, 0), &cursor::packet(c.0, c.1, c.2));
                    }
                }
                if let Some(png) = f.shape.as_ref().and_then(|s| cursor::png(s).map(|p| (p, s.hx, s.hy))) {
                    notify(ServerMsg::CursorShape { png: STANDARD.encode(&png.0), hx: png.1, hy: png.2 });
                }
                if f.tex.is_some() {
                    img = f.tex.clone();
                    cap_us = f.capture_us;
                    pending = true;
                    if !started {
                        // Phase-lock the grid just after the first capture so a vsynced source
                        // lands in the next slot instead of waiting a whole frame.
                        started = true;
                        next_due = Instant::now() + Duration::from_millis(1);
                    }
                }
            }
            if std::mem::take(&mut self.cap.reinit) {
                conv = None;
                sh.idr.store(true, Ordering::Relaxed);
            }
            // Encode as soon as a picture arrives, not at the send deadline.
            // Keep encoded reference pictures in order. If capture is faster than
            // the negotiated stream, retain only the newest unencoded capture
            // while the prepared picture waits for its send slot.
            // Sample the newest unencoded capture near this slot. Encoding immediately
            // after the previous send can freeze an older image for a whole interval.
            let encode_due = next_due.checked_sub(Duration::from_millis(6)).unwrap_or(next_due);
            if pending && prepared.is_empty() && sh.peer().is_some() && Instant::now() >= encode_due {
                if let (Some(src), Some(size)) = (img.as_ref(), self.cap.size()) {
                    if conv.is_none() {
                        conv = Convert::new(&self.dev, &self.ctx, size, self.enc.size).ok();
                    }
                    let timing = secpar_stream::timing::begin();
                    if let Some(nv12) = conv.as_mut().and_then(|c| c.convert(src).ok()) {
                        secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Convert, timing);
                        let timing = secpar_stream::timing::begin();
                        let force = sh.idr.swap(false, Ordering::Relaxed);
                        if force && secpar_stream::timing::enabled() {eprintln!("frame_timing stage=host.key_encode repeat=false");}
                        if let Ok(outs) = self.enc.encode(&nv12, cap_us, force) {
                            secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Encode, timing);
                            prepared.extend(outs);
                            pending = false;
                        }
                    }
                }
            }
            if prepared_packets.is_none() {
                if let Some(e)=prepared.front() {prepared_packets=Some(encoded_packets(frame_id,e,self.enc.codec(),t.fec as usize));}
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
            }
            let repeat = prepared.is_empty();
            if repeat && !idle_repeat_due(last_real.elapsed(), last_send.elapsed(), interval) {
                continue;
            }
            if repeat {
                let (Some(src), Some(size)) = (img.as_ref().filter(|_| sh.peer().is_some()), self.cap.size()) else { continue };
                if conv.is_none() {
                    conv = Convert::new(&self.dev, &self.ctx, size, self.enc.size).ok();
                }
                let timing = secpar_stream::timing::begin();
                let Some(nv12) = conv.as_mut().and_then(|c| c.convert(src).ok()) else { continue };
                secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Convert, timing);
                let timing = secpar_stream::timing::begin();
                let force = sh.idr.swap(false, Ordering::Relaxed);
                if force && secpar_stream::timing::enabled() {eprintln!("frame_timing stage=host.key_encode repeat=true");}
                let Ok(outs) = self.enc.encode(&nv12, now_us(), force) else { break };
                secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Encode, timing);
                prepared.extend(outs);
            }
            let Some(e) = prepared.pop_front() else { continue };
            if e.key && secpar_stream::timing::enabled() {eprintln!("frame_timing stage=host.key_output");}
            secpar_stream::timing::mark(secpar_stream::timing::Stage::Encoded);
            let packets=prepared_packets.take().unwrap_or_else(||encoded_packets(frame_id,&e,self.enc.codec(),t.fec as usize));
            let timing=secpar_stream::timing::begin();
            sh.send_paced_at(&packets,frame_due);
            secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Send,timing);
            secpar_stream::timing::mark(secpar_stream::timing::Stage::Sent);
            last_send = Instant::now();
            if !repeat {
                last_real = last_send;
                let age = now_us().saturating_sub(e.capture_us);
                // The grid starts a little behind capture, which costs display latency. Measure
                // the settled phase once after startup, shift the grid, then lock it.
                if !phase_done {
                    warm.push(age);
                    if warm.len() == 120 {
                        // Ignore the first 30 sends while the encoder warms up.
                        let settled = &warm[30..];
                        let mean = settled.iter().sum::<u64>() / settled.len() as u64;
                        // Preserve half a frame for encoder timing variation, and
                        // never adjust the startup grid by more than half a slot.
                        let target = (interval.as_micros() / 2).max(1) as i64;
                        let delta = (mean as i64 - target).clamp(-target, target);
                        if secpar_stream::timing::enabled() {
                            let mut w = settled.to_vec(); w.sort_unstable();
                            eprintln!("frame_timing stage=host.warm_phase n={} p50_us={} mean_us={} shift_us={delta}", w.len(), w[w.len()/2], mean);
                        }
                        if delta > 0 {
                            next_due = next_due.checked_sub(Duration::from_micros(delta as u64)).unwrap_or(next_due);
                        } else if delta < 0 {
                            next_due += Duration::from_micros((-delta) as u64);
                        }
                        if next_due <= Instant::now() {
                            next_due = Instant::now() + interval;
                        }
                        phase_done = true;
                    }
                }
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
        sh.stop.store(true, Ordering::Relaxed);
    }
}

/// Frame-pacing scheduling for the video thread, as Sunshine does it: a 1 ms system timer so
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

        // The video thread opens its pipeline, reports the codec and size it got, then waits for
        // the full `stream.ready` (it repeats it on resolution changes).
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<ReadyInfo>(1);
        let (vsh, vp, vn) = (sh.clone(), p.clone(), notify.clone());
        let video = std::thread::spawn(move || {
            let _com = Com::init();
            let _prio = Realtime::enter();
            let kbps = vsh.target.lock().unwrap().kbps;
            match Video::open(&vp, kbps) {
                Ok(v) => {
                    let _ = tx.send(Ok((v.enc.codec(), v.fps, v.base)));
                    if let Ok(ready) = ready_rx.recv() {
                        v.run(&vsh, &ready, &vn, &vp);
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });
        // While the encoder opens: this socket's public address, for the phone to punch towards.
        let reflexive = (!bind.is_loopback()).then(|| crate::remote::stun::public_addr(&sh.sock)).flatten();
        let (codec, fps, base) = match rx.recv() {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(err("capture_failed", "video pipeline crashed")),
        };
        let lan = crate::commands::rank_ips(crate::win::sys::adapters().0);
        let cands = host_cands(&lan, port, p.public, reflexive);
        let ready = ReadyInfo { codec, fps, port, session, key: STANDARD.encode(key), cands };
        crate::log::line(&format!("stream: ready {codec:?} {}x{} on {port}, host cands {:?}, phone cands {:?}", base.0, base.1, ready.cands, p.cands));
        let _ = ready_tx.send(ready.clone());
        sh.punch(p.cands.iter().copied());
        // step-down notches follow the size actually streamed
        let mut br = Bitrate::new(p.max_kbps, notches_below(base.1).len() as u8);
        let mut threads = vec![video];
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
        Ok((ready.msg(base), StreamSession { sh, threads }))
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
        *crate::input::STREAM_RECT.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests;
