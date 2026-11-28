mod events;
use events::UiEvents;
use crate::{
    control::{self, Computer, Connected},
    receiver::{Receiver, VideoFrame},
};
use base64::Engine;
use secpar_network::{stun, turn::Turn};
use secpar_stream::Type;
use serde_json::{json, Value};
use std::{
    net::{SocketAddr, SocketAddrV4, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

pub enum Command {
    Input(Value),
    Stop,
    Keyframe,
    Display(String),
    /// Resize the running stream (0×0 = the host's native size).
    Size(u32,u32),
    Clipboard(String),
    ClipboardSharing(bool),
}
pub enum Event {
    Paired(Computer),
    Status(String),
    Video(VideoFrame),
    Audio {packet:Vec<u8>,received_at:Instant},
    CursorShape { png: Vec<u8>, hotspot: [i32; 2] },
    Displays(Value),
    Cursor { x: i16, y: i16, visible: bool },
    Ended(String),
    Clipboard(String),
    ClipboardUnavailable(String),
    AudioUnavailable(String),
    StreamInfo {width:u32,height:u32,fps:u32},
    /// The frames carry 4:4:4 packed into twice the stream's height (sent before StreamInfo).
    FullColour(bool),
    Stats {rtt_ms:Option<u16>,mbps:f32,received_fps:f32},
    /// Latest rumble (large, small motor) the host asked for, per controller id 0..=3.
    Rumble([(u8,u8);4]),
    /// Whether the host can create virtual controllers. Sent once, only when the host listed its abilities.
    PadSupport(bool),
    /// A refusal that leaves the session open, such as a message the grant does not allow.
    Notice(String),
    /// What the host allows this connection to do, from a friend's `welcome`.
    Caps(Vec<String>),
    /// The stream runs in the bundled component on the managed engine, so this
    /// window keeps its normal page instead of showing a picture of its own.
    Managed(bool),
}
/// Errors that must not end a session: the host refused one message and carries on.
pub fn soft_error(v:&Value)->Option<&'static str> {
    match (v["code"].as_str().unwrap_or(""),v["of"].as_str().unwrap_or("")) {
        ("forbidden",_)=>Some("The computer does not allow that."),
        ("pad_limit",_)=>Some("This computer allows no more controllers."),
        ("unsupported","pad")=>Some("Controllers are not available on this computer."),
        _=>None,
    }
}
#[derive(Clone)]
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub max_kbps: u32,
    pub discover_public: bool,
    pub audio: bool,
    /// Optional nonblocking audio delivery independent of UI event consumption.
    pub audio_sink: Option<Arc<dyn Fn(Vec<u8>)+Send+Sync>>,
    /// Optional nonblocking decoder delivery independent of UI event consumption.
    pub video_sink: Option<Arc<dyn Fn(VideoFrame)+Send+Sync>>,
    pub clipboard: bool,
    /// The display to stream (an id from `stream.displays`); empty for the one set on the computer.
    pub display: String,
    /// Ask for full colour (4:4:4 packed into a double-height frame).
    pub chroma444: bool,
    /// Relay credentials from the account, for a friend's computer: there is no relay key to
    /// ask with, and without a relay a friend behind a strict NAT gets no video at all.
    pub turn: Option<GuestTurn>,
}
/// A TURN server (`host:port`, UDP) with its username and password.
#[derive(Clone, Debug, PartialEq)]
pub struct GuestTurn {
    pub server: String,
    pub user: String,
    pub pass: String,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 60,
            max_kbps: 30_000,
            discover_public: true,
            audio: true,
            audio_sink: None,
            video_sink: None,
            clipboard: true,
            display: String::new(),
            chroma444: false,
            turn: None,
        }
    }
}

struct Packet {
    data: Vec<u8>,
    from: SocketAddr,
    relayed: bool,
}
struct Peer {
    socket: Arc<UdpSocket>,
    turn: Arc<Mutex<Option<Arc<Turn>>>>,
    targets: Vec<SocketAddr>,
    confirmed: Option<(SocketAddr, bool)>,
    live: Arc<AtomicBool>,
    reader: Option<std::thread::JoinHandle<()>>,
    /// Where relayed packets that arrive over TLS join the UDP ones.
    packets: mpsc::Sender<Packet>,
}
impl Peer {
    fn new(socket: UdpSocket) -> (Self, mpsc::Receiver<Packet>) {
        let socket = Arc::new(socket);
        let turn = Arc::new(Mutex::new(None::<Arc<Turn>>));
        let live = Arc::new(AtomicBool::new(true));
        let (s, t, l) = (socket.clone(), turn.clone(), live.clone());
        let (tx, rx) = mpsc::channel(1024);
        let packets = tx.clone();
        let reader = std::thread::spawn(move || {
            let _ = s.set_read_timeout(Some(Duration::from_millis(30)));
            let mut b = [0; 2048];
            while l.load(Ordering::Relaxed) {
                let Ok((n, from)) = s.recv_from(&mut b) else {
                    continue;
                };
                let relay = t.lock().unwrap().clone();
                let packet = if let Some(t) = relay.filter(|t| t.server == from) {
                    t.on_packet(&b[..n]).map(|(peer, data)| Packet {
                        data,
                        from: peer.into(),
                        relayed: true,
                    })
                } else if n > 0 && b[0] >> 4 == 1 {
                    Some(Packet {
                        data: b[..n].to_vec(),
                        from,
                        relayed: false,
                    })
                } else {
                    None
                };
                if let Some(p) = packet {
                    let _ = tx.try_send(p);
                }
            }
        });
        (
            Self {
                socket,
                turn,
                targets: vec![],
                confirmed: None,
                live,
                reader: Some(reader),
                packets,
            },
            rx,
        )
    }
    fn send(&self, b: &[u8]) {
        let turn = self.turn.lock().unwrap().clone();
        let targets = self
            .confirmed
            .map(|p| vec![p])
            .unwrap_or_else(|| self.targets.iter().map(|p| (*p, false)).collect());
        for (addr, via) in targets {
            if !via {
                let _ = self.socket.send_to(b, addr);
            }
            if via || self.confirmed.is_none() {
                if let (Some(t), SocketAddr::V4(to)) = (&turn, addr) {
                    t.send_raw(&self.socket, &t.wrap(to, b));
                }
            }
        }
    }
    /// One packet on one route: `via` true goes through our relay.
    fn send_on(&self, b: &[u8], addr: SocketAddr, via: bool) {
        if !via {
            let _ = self.socket.send_to(b, addr);
        } else if let (Some(t), SocketAddr::V4(to)) = (self.turn.lock().unwrap().clone(), addr) {
            t.send_raw(&self.socket, &t.wrap(to, b));
        }
    }
    fn permit(&self) {
        if let Some(t) = self.turn.lock().unwrap().clone() {
            let s = self.socket.clone();
            let targets: Vec<_> = self
                .targets
                .iter()
                .filter_map(|a| {
                    if let SocketAddr::V4(a) = a {
                        Some(*a)
                    } else {
                        None
                    }
                })
                .collect();
            std::thread::spawn(move || t.permit(&s, &targets));
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Relaxed);
        if let Some(t) = self.turn.lock().unwrap().take() {
            let socket = self.socket.clone();
            std::thread::spawn(move || t.refresh(&socket, 0));
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
/// The relay's address over UDP, and over TLS for networks that block UDP.
struct TurnCreds {
    udp: Option<SocketAddr>,
    tls: Option<(String, u16)>,
    user: String,
    pass: String,
}

/// The account's relay credentials as the stream uses them: UDP to the server, and the same
/// server over TLS on 443 for networks that block UDP.
async fn guest_credentials(t: &GuestTurn) -> Option<TurnCreds> {
    let host = t.server.rsplit_once(':').map(|(h, _)| h.to_string())?;
    let udp = tokio::net::lookup_host(t.server.as_str()).await.ok()?.find(SocketAddr::is_ipv4);
    Some(TurnCreds { udp, tls: Some((host, 443)), user: t.user.clone(), pass: t.pass.clone() })
}

async fn turn_credentials(computer: &Computer) -> Option<TurnCreds> {
    let r = computer.relay.as_ref()?;
    let url = r
        .signed_url(&computer.id, now_s(), "turn")
        .ok()?
        .replacen("wss://", "https://", 1);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .ok()?;
    let body: Value = client
        .post(url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    let ice = &body["iceServers"];
    let servers: Vec<_> = ice
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_else(|| vec![ice]);
    for server in servers {
        let (Some(user), Some(pass)) = (server["username"].as_str(), server["credential"].as_str())
        else {
            continue;
        };
        let urls: Vec<_> = server["urls"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_else(|| server["urls"].as_str().into_iter().collect());
        let mut creds = TurnCreds { udp: None, tls: None, user: user.into(), pass: pass.into() };
        for u in &urls {
            if let Some(rest) = u.strip_prefix("turns:") {
                let hp = rest.split('?').next().unwrap_or("");
                let (host, port) = hp.rsplit_once(':').map(|(h, p)| (h, p.parse().unwrap_or(5349))).unwrap_or((hp, 5349));
                // Port 443 gets through the most firewalls.
                if creds.tls.as_ref().is_none_or(|(_, p)| *p != 443) {
                    creds.tls = Some((host.to_string(), port));
                }
                continue;
            }
            if creds.udp.is_some() || !u.starts_with("turn:") || u.contains("transport=tcp") {
                continue;
            }
            let Some(hp) = u.trim_start_matches("turn:").split('?').next() else { continue };
            let hp = if hp.contains(':') { hp.to_string() } else { format!("{hp}:3478") };
            if let Ok(mut addrs) = tokio::net::lookup_host(hp).await {
                creds.udp = addrs.find(SocketAddr::is_ipv4);
            }
        }
        if creds.udp.is_some() || creds.tls.is_some() {
            return Some(creds);
        }
    }
    None
}

/// Allocates on `t`, permits the computer's addresses and keeps the allocation fresh.
async fn start_relay(t: Arc<Turn>, socket: Arc<UdpSocket>, peers: Vec<SocketAddrV4>, live: Arc<AtomicBool>) -> Option<SocketAddrV4> {
    let result = tokio::task::spawn_blocking({ let t = t.clone(); let socket = socket.clone(); let peers = peers.clone(); move || { let a = t.allocate(&socket); if a.is_some() { t.permit(&socket, &peers); } a } }).await.ok().flatten();
    if result.is_some() {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(200)).await;
                if !live.load(Ordering::Relaxed) { break; }
                let (t, s, peers) = (t.clone(), socket.clone(), peers.clone());
                let _ = tokio::task::spawn_blocking(move || { t.refresh(&s, 600); t.permit(&s, &peers); }).await;
            }
        });
    }
    result
}
fn emit(receiver: &mut Receiver, peer: &Peer, typ: Type, body: &[u8]) -> Result<(), String> {
    let packet = receiver
        .seal(typ, body)
        .ok_or("Stream packet sequence exhausted; reconnect to rekey")?;
    peer.send(&packet);
    Ok(())
}
/// Round trips on the current path and on the other one (direct or relay), five each.
#[derive(Default)]
struct PathTimes {
    current: std::collections::VecDeque<u16>,
    other: std::collections::VecDeque<u16>,
}
impl PathTimes {
    fn push(v: &mut std::collections::VecDeque<u16>, ms: u16) {
        if v.len() == 5 {
            v.pop_front();
        }
        v.push_back(ms);
    }
    fn current(&mut self, ms: u16) {
        Self::push(&mut self.current, ms);
    }
    fn other(&mut self, ms: u16) {
        Self::push(&mut self.other, ms);
    }
    fn median(v: &std::collections::VecDeque<u16>) -> u16 {
        let mut s: Vec<u16> = v.iter().copied().collect();
        s.sort_unstable();
        s[s.len() / 2]
    }
    /// The other path is clearly quicker: five samples each, 30 % and 20 ms faster by the median.
    fn other_faster(&self) -> bool {
        if self.current.len() < 5 || self.other.len() < 5 {
            return false;
        }
        let (cur, other) = (Self::median(&self.current) as u32, Self::median(&self.other) as u32);
        other * 10 < cur * 7 && cur >= other + 20
    }
}

#[cfg(test)]
mod path_tests {
    use super::PathTimes;
    #[test]
    fn switches_only_for_a_clearly_faster_path() {
        let mut p = PathTimes::default();
        for _ in 0..5 {
            p.current(200);
            p.other(60);
        }
        assert!(p.other_faster());
        let mut q = PathTimes::default();
        for _ in 0..5 {
            q.current(40);
            q.other(25);
        }
        assert!(!q.other_faster(), "15 ms is not worth a switch");
        let mut r = PathTimes::default();
        for _ in 0..4 {
            r.current(300);
            r.other(30);
        }
        assert!(!r.other_faster(), "needs five samples each");
    }
}

/// Runs a paired control connection and its PRS stream until Stop, disconnect or UI closure.
/// UI state is bounded/coalesced; a stalled UI cannot stop network keepalives.
pub async fn run(
    mut conn: Connected,
    options: Options,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
) -> Result<(), String> {
    let mut events=UiEvents::new(events);
    // Windows: keeps the Wi-Fi card on its channel for the whole session (no-op elsewhere).
    let _wlan = secpar_network::wlan::hold(None);
    events.emit(Event::Paired(conn.computer.clone()))?;
    if !conn.caps.is_empty(){events.emit(Event::Caps(conn.caps.clone()))?;events.emit(Event::PadSupport(conn.caps.iter().any(|c|c=="pad")))?;}
    events.emit(Event::Status("Finding a stream route".into()))?;
    let direct = conn.direct_host.clone();
    let discover_public = options.discover_public;
    let (socket, cands) = tokio::task::spawn_blocking(move || {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|_| "Could not open a stream socket")?;
        let route = UdpSocket::bind("0.0.0.0:0").map_err(|_| "Could not inspect local route")?;
        let remote = direct
            .as_deref()
            .unwrap_or("1.1.1.1")
            .parse::<std::net::IpAddr>()
            .map_err(|_| "Invalid route address")?;
        let _ = route.connect(SocketAddr::new(remote, 53));
        let mut cands = vec![];
        if let Ok(local) = route.local_addr() {
            cands
                .push(SocketAddr::new(local.ip(), socket.local_addr().unwrap().port()).to_string());
        }
        if discover_public {
            if let Some(public) = stun::public_addr(&socket) {
                cands.push(public.to_string());
            }
        }
        Ok::<_, String>((socket, cands))
    })
    .await
    .map_err(|_| "Stream socket setup failed")??;
    let (peer, mut packets) = Peer::new(socket);
    let mut peer = peer;
    control::send(&mut conn.socket,json!({"t":"stream.start","w":options.width,"h":options.height,"fps":options.fps,"maxKbps":options.max_kbps,"codecs":["h264"],"audio":options.audio,"cands":cands,"display":options.display,"chroma":if options.chroma444 {"444"} else {""}})).await?;
    let mut clipboard_enabled=options.clipboard;
    if clipboard_enabled {control::send(&mut conn.socket,json!({"t":"clip.sub","on":true})).await?;}
    let mut receiver = None::<Receiver>;
    let mut session_id = None;
    let mut peer_t = 0u64;
    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(5));
    let mut queue = crate::receiver::QueueDelay::new(start);
    let (mut probe_at, mut report_at, mut ping_at, mut idr_at) = (
        Instant::now() - Duration::from_secs(1),
        Instant::now(),
        Instant::now(),
        Instant::now() - Duration::from_secs(1),
    );
    let (mut high_seq, mut report_base, mut received, mut last_frame, mut rtt) =
        (None::<u32>, 0u32, 0u32, 0u32, 0u16);
    let mut last_packet = Instant::now();
    let mut received_authenticated=false;
    let mut turn_started = false;
    // Timing the other path (direct or relay) against the current one; hosts listing `pathprobe`
    // answer test probes without moving the stream.
    let path_test = conn.caps.iter().any(|c| c == "pathprobe");
    let (mut relay_for_test, mut relay_ready) = (false, false);
    let mut paths = PathTimes::default();
    let mut test_at = Instant::now();
    // After a switch, packets still on their way on the old path must not switch back.
    let mut switched_at: Option<Instant> = None;
    let (turn_tx, mut turn_rx) = mpsc::channel::<Option<SocketAddrV4>>(1);
    let mut inputs = std::collections::VecDeque::new();
    let mut rumble=[(0u8,0u8);4];
    let mut input_id = 0u64;
    let (mut stats_at,mut stats_bytes,mut stats_frames,mut rtt_known)=(Instant::now(),0u64,0u32,false);
    let result:Result<(),String>=async {loop{
        tokio::select!{
            command=commands.recv()=>match command{
                None|Some(Command::Stop)=>break Ok(()),
                Some(Command::Display(id))=>control::send(&mut conn.socket,json!({"t":"stream.display","id":id})).await?,
                Some(Command::Size(w,h))=>control::send(&mut conn.socket,json!({"t":"stream.size","w":w,"h":h})).await?,
                Some(Command::Keyframe)=>if let Some(r)=receiver.as_mut(){emit(r,&peer,Type::Idr,&[])?;},
                Some(Command::Clipboard(text))=>if clipboard_enabled && text.len()<=1_000_000 {
                    control::send(&mut conn.socket,json!({"t":"clip","mime":"text/plain","data":text})).await?;
                },
                Some(Command::ClipboardSharing(on))=>{
                    clipboard_enabled=on;
                    control::send(&mut conn.socket,json!({"t":"clip.sub","on":on})).await?;
                },
                Some(Command::Input(mut value))=>{
                    let typ=value["t"].as_str().unwrap_or("");
                    if !matches!(typ,"key"|"mouse.move"|"mouse.button"|"mouse.scroll"|"mouse.pos"|"pad"|"pad.off"){continue;}
                    if let Some(r)=receiver.as_mut(){
                        input_id=input_id.checked_add(1).ok_or("Input sequence exhausted")?;
                        let object=value.as_object_mut().ok_or("Invalid input event")?;object.insert("i".into(),input_id.into());
                        inputs.push_back(value);while inputs.len()>3{inputs.pop_front();}
                        let bytes=serde_json::to_vec(&inputs).map_err(|_|"Invalid input event")?;
                        emit(r,&peer,Type::Input,&bytes)?;
                    }
                }
            },
            message=control::next(&mut conn.socket)=>{
                let v=message?;
                match v["t"].as_str().unwrap_or(""){
                    "stream.ready"=>{
                        if v["codec"]!="h264"{return Err("Host did not negotiate H.264".into());}
                        events.emit(Event::FullColour(v["chroma"]=="444"))?;
                        events.emit(Event::StreamInfo {
                            width:v["w"].as_u64().unwrap_or(options.width as u64).clamp(1,8192) as u32,
                            height:v["h"].as_u64().unwrap_or(options.height as u64).clamp(1,8192) as u32,
                            fps:v["fps"].as_u64().unwrap_or(options.fps as u64).clamp(1,240) as u32,
                        })?;
                        let id=v["session"].as_u64().and_then(|v|u32::try_from(v).ok()).ok_or("Invalid stream session")?;
                        if session_id!=Some(id){
                            let key: [u8;32]=base64::engine::general_purpose::STANDARD.decode(v["key"].as_str().ok_or("Missing stream key")?).map_err(|_|"Invalid stream key")?.try_into().map_err(|_|"Invalid stream key length")?;
                            receiver=Some(Receiver::new(&key,id));session_id=Some(id);high_seq=None;received=0;inputs.clear();input_id=0;
                        }
                        if let Some(cands)=v["cands"].as_array(){peer.targets=cands.iter().filter_map(Value::as_str).filter_map(|s|s.parse().ok()).filter(SocketAddr::is_ipv4).take(32).collect();}
                        if let (Some(host),Some(port))=(&conn.direct_host,v["port"].as_u64().and_then(|p|u16::try_from(p).ok())){if let Ok(ip)=host.parse(){let a=SocketAddr::new(ip,port);if !peer.targets.contains(&a){peer.targets.push(a);}}}
                        peer.permit();
                    }
                    "stream.cand"=>if let Some(c)=v["c"].as_str().and_then(|s|s.parse::<SocketAddr>().ok()).filter(SocketAddr::is_ipv4){if peer.targets.len()<32 && !peer.targets.contains(&c){crate::connlog::note(&crate::connlog::short_label(&conn.computer.id),"turn","host_candidate",0);peer.targets.push(c);peer.permit();}},
                    "stream.displays"=>events.emit(Event::Displays(v))?,
                    "rumble"=>if let Some(slot)=v["id"].as_u64().and_then(|i|rumble.get_mut(i as usize)){
                        *slot=(v["lo"].as_u64().unwrap_or(0).min(255) as u8,v["hi"].as_u64().unwrap_or(0).min(255) as u8);
                        events.emit(Event::Rumble(rumble))?;
                    },
                    "cursor.shape"=>{
                        if let(Some(encoded),Some(hx),Some(hy))=(v["png"].as_str().filter(|s|s.len()<=350_000),v["hx"].as_i64().and_then(|n|i32::try_from(n).ok()),v["hy"].as_i64().and_then(|n|i32::try_from(n).ok())) {
                            if let Ok(png)=base64::engine::general_purpose::STANDARD.decode(encoded){events.emit(Event::CursorShape{png,hotspot:[hx,hy]})?;}
                        }
                    },
                    "clip"=>if clipboard_enabled && v["mime"]=="text/plain" {
                        if let Some(text)=v["data"].as_str().filter(|s|s.len()<=1_000_000){events.emit(Event::Clipboard(text.into()))?;}
                    },
                    "error" if soft_error(&v).is_some()=>events.emit(Event::Notice(soft_error(&v).unwrap_or_default().into()))?,
                    "error" if v["code"]=="audio_unavailable"=>events.emit(Event::AudioUnavailable(v["msg"].as_str().unwrap_or("Host audio is unavailable").into()))?,
                    "error" if matches!(v["of"].as_str(),Some("clip"|"clip.sub"))=>{
                        clipboard_enabled=false;
                        events.emit(Event::ClipboardUnavailable(v["msg"].as_str().unwrap_or("Clipboard sharing is unavailable on the host").into()))?;
                    },
                    "stream.error"|"error"=>return Err(v["msg"].as_str().unwrap_or("Host refused the stream").to_string()),
                    _=>{},
                }
            },
            Some(packet)=packets.recv()=>{
                let Some(r)=receiver.as_mut()else{continue;};let Some((h,plain))=r.open(&packet.data)else{continue;};
                stats_bytes=stats_bytes.saturating_add(packet.data.len() as u64);
                let route=Some((packet.from,packet.relayed));
                let test_reply=h.typ==Type::Probe && h.flags & secpar_stream::header::PATH_TEST!=0;
                if !test_reply && peer.confirmed!=route && switched_at.is_none_or(|t|t.elapsed()>Duration::from_secs(2)) {peer.confirmed=route;crate::connlog::note(&crate::connlog::short_label(&conn.computer.id),"stream",if packet.relayed{"relay"}else{"direct"},start.elapsed().as_millis() as u64);events.emit(Event::Status(if packet.relayed{"Relay"}else{"Direct"}.into()))?;}
                last_packet=Instant::now();received_authenticated=true;
                if high_seq.is_none() && matches!(h.typ,Type::Video|Type::Fec){report_base=h.seq;high_seq=Some(h.seq);}
                if let Some(high)=high_seq.as_mut(){*high=(*high).max(h.seq);received=received.saturating_add(1);}
                match h.typ{
                    Type::Video|Type::Fec=>{
                        let now=Instant::now();
                        // One packet can complete several frames: a resent one releases those waiting behind it.
                        let first=r.assembler.push(h,plain,now);
                        let mut frames:Vec<_>=first.into_iter().collect();
                        while let Some(f)=r.assembler.pop(){frames.push(f);}
                        for frame in frames {last_frame=frame.id;stats_frames+=1;secpar_stream::timing::mark(secpar_stream::timing::Stage::Arrive);if let Some(sink)=&options.video_sink {sink(frame);}else{events.emit(Event::Video(frame))?;}}
                        for (id,frags) in r.assembler.nacks(now){emit(r,&peer,Type::Nack,&secpar_stream::nack::encode(id,&frags))?;}
                    },
                    Type::Audio if plain.len()>8 => {
                        let packet=plain[8..].to_vec();
                        if let Some(sink)=&options.audio_sink{sink(packet);}
                        else{events.emit(Event::Audio{packet,received_at:Instant::now()})?;}
                    },
                    Type::Probe if plain.len()>=16 && test_reply=>{let echo=u64::from_be_bytes(plain[8..16].try_into().unwrap());let now=start.elapsed().as_micros() as u64;if echo>0 && now>=echo{paths.other(((now-echo)/1000).min(u16::MAX as u64) as u16);}},
                    Type::Probe if plain.len()>=16=>{peer_t=u64::from_be_bytes(plain[..8].try_into().unwrap());let echo=u64::from_be_bytes(plain[8..16].try_into().unwrap());let now=start.elapsed().as_micros() as u64;if echo>0 && now>=echo{rtt=((now-echo)/1000).min(u16::MAX as u64) as u16;rtt_known=true;r.assembler.rtt=Duration::from_millis(rtt as u64);paths.current(rtt);}},
                    Type::Cursor if plain.len()>=5=>events.emit(Event::Cursor{x:i16::from_be_bytes(plain[..2].try_into().unwrap()),y:i16::from_be_bytes(plain[2..4].try_into().unwrap()),visible:plain[4]!=0})?,
                    Type::Bye=>return Err("Host ended the stream".into()),
                    _=>{},
                }
            },
            Some(relayed)=turn_rx.recv()=>if relay_for_test {relay_ready=relayed.is_some();} else if let Some(c)=relayed{control::send(&mut conn.socket,json!({"t":"stream.cand","c":c.to_string()})).await?;},
            _=tick.tick()=>{
                events.flush()?;
                if stats_at.elapsed()>=Duration::from_millis(500) {
                    let seconds=stats_at.elapsed().as_secs_f32();
                    events.emit(Event::Stats{rtt_ms:rtt_known.then_some(rtt),mbps:stats_bytes as f32*8./seconds/1_000_000.,received_fps:stats_frames as f32/seconds})?;
                    stats_at=Instant::now();stats_bytes=0;stats_frames=0;
                }
                if start.elapsed()>Duration::from_secs(20) && last_packet.elapsed()>Duration::from_secs(15){
                    return Err(if received_authenticated{"Stream packets stopped arriving for 15 seconds"}else{"Host control connected, but no authenticated stream packets arrived"}.into());
                }
                if peer.confirmed.is_some() && last_packet.elapsed()>Duration::from_secs(2) {
                    peer.confirmed=None;probe_at=Instant::now()-Duration::from_secs(1);
                    events.emit(Event::Status("Reconnecting stream route".into()))?;
                }
                if let Some(r)=receiver.as_mut(){
                    let now=Instant::now();
                    r.assembler.tick(now);
                    while let Some(frame)=r.assembler.pop(){last_frame=frame.id;stats_frames+=1;if let Some(sink)=&options.video_sink {sink(frame);}else{events.emit(Event::Video(frame))?;}}
                    for (id,frags) in r.assembler.nacks(now){emit(r,&peer,Type::Nack,&secpar_stream::nack::encode(id,&frags))?;}
                    for (capture_us,arrived) in r.assembler.arrivals.drain(..){queue.add(capture_us,arrived);}
                    if probe_at.elapsed()>Duration::from_millis(if peer.confirmed.is_some(){500}else{200}){probe_at=Instant::now();let mut body=(start.elapsed().as_micros() as u64).to_be_bytes().to_vec();body.extend(peer_t.to_be_bytes());emit(r,&peer,Type::Probe,&body)?;}
                    if report_at.elapsed()>Duration::from_millis(100){report_at=Instant::now();let expected=high_seq.map(|h|h.saturating_sub(report_base)).unwrap_or(0);let loss=if expected==0{0}else{expected.saturating_sub(received).saturating_mul(1000)/expected};let mut body=[0;16];body[..4].copy_from_slice(&last_frame.to_be_bytes());body[4..6].copy_from_slice(&(loss.min(1000) as u16).to_be_bytes());body[10..12].copy_from_slice(&rtt.to_be_bytes());body[12..14].copy_from_slice(&queue.take_ms().to_be_bytes());emit(r,&peer,Type::Report,&body)?;report_base=high_seq.unwrap_or(0);received=0;}
                    if (r.assembler.needs_keyframe() || events.needs_keyframe()) && idr_at.elapsed()>Duration::from_millis(250){idr_at=Instant::now();emit(r,&peer,Type::Idr,&[])?;}
                }
                if let (Some(r),Some((addr,via)))=(receiver.as_mut(),peer.confirmed) {
                    // the other path: our relay when direct; direct when relayed through ours
                    let other_ok=if via {true} else {relay_ready};
                    if path_test && other_ok && test_at.elapsed()>Duration::from_secs(1) && switched_at.is_none_or(|t|t.elapsed()>Duration::from_secs(10)) {
                        test_at=Instant::now();
                        let mut body=(start.elapsed().as_micros() as u64).to_be_bytes().to_vec();body.extend(peer_t.to_be_bytes());
                        if let Some(pkt)=r.seal_flags(Type::Probe,secpar_stream::header::PATH_TEST,&body){peer.send_on(&pkt,addr,!via);}
                        if paths.other_faster() {
                            peer.confirmed=Some((addr,!via));switched_at=Some(Instant::now());paths=PathTimes::default();
                            crate::connlog::note(&crate::connlog::short_label(&conn.computer.id),"stream",if via{"switched_direct"}else{"switched_relay"},start.elapsed().as_millis() as u64);
                            events.emit(Event::Status(if via{"Direct"}else{"Relay"}.into()))?;
                            // a normal probe on the new path moves the host's stream there
                            let mut body=(start.elapsed().as_micros() as u64).to_be_bytes().to_vec();body.extend(peer_t.to_be_bytes());emit(r,&peer,Type::Probe,&body)?;
                        }
                    }
                }
                if ping_at.elapsed()>Duration::from_secs(10){ping_at=Instant::now();control::send(&mut conn.socket,json!({"t":"ping"})).await?;}
                let test_relay=path_test && peer.confirmed.is_some_and(|c|!c.1) && start.elapsed()>Duration::from_secs(8);
                if !turn_started && (peer.confirmed.is_none() || test_relay) && start.elapsed()>Duration::from_secs(3) && (conn.computer.relay.is_some() || options.turn.is_some()){
                    turn_started=true;relay_for_test=peer.confirmed.is_some();let computer=conn.computer.clone();let guest_turn=options.turn.clone();let socket=peer.socket.clone();let slot=peer.turn.clone();let peers:Vec<_>=peer.targets.iter().filter_map(|a|if let SocketAddr::V4(a)=a{Some(*a)}else{None}).collect();let tx=turn_tx.clone();let live=peer.live.clone();
                    let packets=peer.packets.clone();
                    tokio::spawn(async move{
                        let mut allocated=None;
                        // Each relay step goes in the log: a friend's network can stop it anywhere.
                        let log=crate::connlog::Conn::new(&computer.id);
                        let creds=match &guest_turn { Some(t)=>guest_credentials(t).await, None=>turn_credentials(&computer).await };
                        log.note("turn",if creds.is_some(){"credentials"}else{"no_credentials"},0);
                        if let Some(c)=creds{
                            if let Some(server)=c.udp{
                                let t=Arc::new(Turn::new(server,c.user.clone(),c.pass.clone()));*slot.lock().unwrap()=Some(t.clone());
                                allocated=start_relay(t,socket.clone(),peers.clone(),live.clone()).await;
                            }
                            // UDP to the relay blocked (many offices): the same relay over TLS on 443.
                            if let (None,Some((host,port)))=(allocated,c.tls.clone()){
                                let (frames,to)=(slot.clone(),packets);
                                let delivered=move|b:&[u8]|{if let Some(t)=frames.lock().unwrap().clone(){if let Some((peer,data))=t.on_packet(b){let _=to.try_send(Packet{data,from:peer.into(),relayed:true});}}};
                                if let Ok((link,addr))=crate::turn_tls::connect(&host,port,delivered).await{
                                    let t=Arc::new(Turn::new(addr,c.user,c.pass).with_link(link));*slot.lock().unwrap()=Some(t.clone());
                                    allocated=start_relay(t,socket,peers,live).await;
                                }
                            }
                        }
                        log.note("turn",if allocated.is_some(){"allocated"}else{"not_allocated"},log.began().elapsed().as_millis() as u64);
                        let _=tx.send(allocated).await;
                    });
                }
            }
        }
    }}.await;
    if let Some(r) = receiver.as_mut() {
        let _ = emit(r, &peer, Type::Bye, &[]);
    }
    let _ = control::send(&mut conn.socket, json!({"t":"stream.stop"})).await;
    let _ = conn.socket.close(None).await;
    result
}

#[cfg(test)]
mod guest_tests {
    use super::*;
    #[test]
    fn refusals_of_one_message_do_not_end_the_session() {
        assert!(soft_error(&json!({"t":"error","code":"forbidden"})).is_some());
        assert!(soft_error(&json!({"t":"error","code":"pad_limit"})).is_some());
        assert!(soft_error(&json!({"t":"error","code":"unsupported","of":"pad"})).is_some());
        for end in [json!({"code":"guest_ended"}),json!({"code":"busy"}),json!({"code":"locked"}),json!({"code":"unsupported","of":"clip"}),json!({})] {
            assert!(soft_error(&end).is_none(),"{end} must end the session");
        }
    }
}
