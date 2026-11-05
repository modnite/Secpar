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
}
pub enum Event {
    Paired(Computer),
    Status(String),
    Video(VideoFrame),
    Audio(Vec<u8>),
    CursorShape { png: Vec<u8>, hotspot: [i32; 2] },
    Displays(Value),
    Cursor { x: i16, y: i16, visible: bool },
    Ended(String),
}
#[derive(Clone)]
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub max_kbps: u32,
    pub discover_public: bool,
    pub audio: bool,
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
}
impl Peer {
    fn new(socket: UdpSocket) -> (Self, mpsc::Receiver<Packet>) {
        let socket = Arc::new(socket);
        let turn = Arc::new(Mutex::new(None::<Arc<Turn>>));
        let live = Arc::new(AtomicBool::new(true));
        let (s, t, l) = (socket.clone(), turn.clone(), live.clone());
        let (tx, rx) = mpsc::channel(1024);
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
                    let _ = self.socket.send_to(&t.wrap(to, b), t.server);
                }
            }
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
async fn turn_credentials(computer: &Computer) -> Option<(SocketAddr, String, String)> {
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
        for u in urls {
            if !u.starts_with("turn:") || u.contains("transport=tcp") {
                continue;
            }
            let hp = u.trim_start_matches("turn:").split('?').next()?;
            let hp = if hp.contains(':') {
                hp.to_string()
            } else {
                format!("{hp}:3478")
            };
            if let Ok(mut addrs) = tokio::net::lookup_host(hp).await {
                if let Some(addr) = addrs.find(SocketAddr::is_ipv4) {
                    return Some((addr, user.into(), pass.into()));
                }
            }
        }
    }
    None
}
fn emit(receiver: &mut Receiver, peer: &Peer, typ: Type, body: &[u8]) -> Result<(), String> {
    let packet = receiver
        .seal(typ, body)
        .ok_or("Stream packet sequence exhausted; reconnect to rekey")?;
    peer.send(&packet);
    Ok(())
}
async fn notify(events: &mpsc::Sender<Event>, event: Event) -> Result<(), String> {
    events
        .send(event)
        .await
        .map_err(|_| "Client window closed".into())
}

/// Runs a paired control connection and its PRS stream until Stop, disconnect or UI closure.
/// A bounded event queue applies backpressure; the datagram queue is separately bounded.
pub async fn run(
    mut conn: Connected,
    options: Options,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
) -> Result<(), String> {
    notify(&events, Event::Paired(conn.computer.clone())).await?;
    notify(&events, Event::Status("Finding a stream route".into())).await?;
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
    control::send(&mut conn.socket,json!({"t":"stream.start","w":options.width,"h":options.height,"fps":options.fps,"maxKbps":options.max_kbps,"codecs":["h264"],"audio":options.audio,"cands":cands,"display":""})).await?;
    let mut receiver = None::<Receiver>;
    let mut session_id = None;
    let mut peer_t = 0u64;
    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(5));
    let (mut probe_at, mut report_at, mut ping_at, mut idr_at) = (
        Instant::now() - Duration::from_secs(1),
        Instant::now(),
        Instant::now(),
        Instant::now() - Duration::from_secs(1),
    );
    let (mut high_seq, mut report_base, mut received, mut last_frame, mut rtt) =
        (None::<u32>, 0u32, 0u32, 0u32, 0u16);
    let mut last_packet = Instant::now();
    let mut turn_started = false;
    let (turn_tx, mut turn_rx) = mpsc::channel::<Option<SocketAddrV4>>(1);
    let mut inputs = std::collections::VecDeque::new();
    let mut input_id = 0u64;
    let result:Result<(),String>=async {loop{
        tokio::select!{
            command=commands.recv()=>match command{
                None|Some(Command::Stop)=>break Ok(()),
                Some(Command::Display(id))=>control::send(&mut conn.socket,json!({"t":"stream.display","id":id})).await?,
                Some(Command::Keyframe)=>if let Some(r)=receiver.as_mut(){emit(r,&peer,Type::Idr,&[])?;},
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
                        let id=v["session"].as_u64().and_then(|v|u32::try_from(v).ok()).ok_or("Invalid stream session")?;
                        if session_id!=Some(id){
                            let key: [u8;32]=base64::engine::general_purpose::STANDARD.decode(v["key"].as_str().ok_or("Missing stream key")?).map_err(|_|"Invalid stream key")?.try_into().map_err(|_|"Invalid stream key length")?;
                            receiver=Some(Receiver::new(&key,id));session_id=Some(id);high_seq=None;received=0;inputs.clear();input_id=0;
                        }
                        if let Some(cands)=v["cands"].as_array(){peer.targets=cands.iter().filter_map(Value::as_str).filter_map(|s|s.parse().ok()).filter(SocketAddr::is_ipv4).take(32).collect();}
                        if let (Some(host),Some(port))=(&conn.direct_host,v["port"].as_u64().and_then(|p|u16::try_from(p).ok())){if let Ok(ip)=host.parse(){let a=SocketAddr::new(ip,port);if !peer.targets.contains(&a){peer.targets.push(a);}}}
                        peer.permit();
                    }
                    "stream.cand"=>if let Some(c)=v["c"].as_str().and_then(|s|s.parse::<SocketAddr>().ok()).filter(SocketAddr::is_ipv4){if peer.targets.len()<32 && !peer.targets.contains(&c){peer.targets.push(c);peer.permit();}},
                    "stream.displays"=>notify(&events,Event::Displays(v)).await?,
                    "cursor.shape"=>{
                        if let(Some(encoded),Some(hx),Some(hy))=(v["png"].as_str().filter(|s|s.len()<=350_000),v["hx"].as_i64().and_then(|n|i32::try_from(n).ok()),v["hy"].as_i64().and_then(|n|i32::try_from(n).ok())) {
                            if let Ok(png)=base64::engine::general_purpose::STANDARD.decode(encoded){notify(&events,Event::CursorShape{png,hotspot:[hx,hy]}).await?;}
                        }
                    },
                    "stream.error"|"error"=>return Err(v["msg"].as_str().unwrap_or("Host refused the stream").to_string()),
                    _=>{},
                }
            },
            Some(packet)=packets.recv()=>{
                let Some(r)=receiver.as_mut()else{continue;};let Some((h,plain))=r.open(&packet.data)else{continue;};
                let route=Some((packet.from,packet.relayed));
                if peer.confirmed!=route {peer.confirmed=route;notify(&events,Event::Status(if packet.relayed{"Relay"}else{"Direct"}.into())).await?;}
                last_packet=Instant::now();
                if high_seq.is_none() && matches!(h.typ,Type::Video|Type::Fec){report_base=h.seq;high_seq=Some(h.seq);}
                if let Some(high)=high_seq.as_mut(){*high=(*high).max(h.seq);received=received.saturating_add(1);}
                match h.typ{
                    Type::Video|Type::Fec=>if let Some(frame)=r.assembler.push(h,plain,Instant::now()){last_frame=frame.id;notify(&events,Event::Video(frame)).await?;},
                    Type::Audio if plain.len()>8 => notify(&events,Event::Audio(plain[8..].to_vec())).await?,
                    Type::Probe if plain.len()>=16=>{peer_t=u64::from_be_bytes(plain[..8].try_into().unwrap());let echo=u64::from_be_bytes(plain[8..16].try_into().unwrap());let now=start.elapsed().as_micros() as u64;if echo>0 && now>=echo{rtt=((now-echo)/1000).min(u16::MAX as u64) as u16;}},
                    Type::Cursor if plain.len()>=5=>notify(&events,Event::Cursor{x:i16::from_be_bytes(plain[..2].try_into().unwrap()),y:i16::from_be_bytes(plain[2..4].try_into().unwrap()),visible:plain[4]!=0}).await?,
                    Type::Bye=>return Err("Host ended the stream".into()),
                    _=>{},
                }
            },
            Some(relayed)=turn_rx.recv()=>if let Some(c)=relayed{control::send(&mut conn.socket,json!({"t":"stream.cand","c":c.to_string()})).await?;},
            _=tick.tick()=>{
                if start.elapsed()>Duration::from_secs(20) && last_packet.elapsed()>Duration::from_secs(15){return Err("No authenticated stream packets reached this client".into());}
                if let Some(r)=receiver.as_mut(){
                    r.assembler.tick(Instant::now());
                    if probe_at.elapsed()>Duration::from_millis(if peer.confirmed.is_some(){1000}else{200}){probe_at=Instant::now();let mut body=(start.elapsed().as_micros() as u64).to_be_bytes().to_vec();body.extend(peer_t.to_be_bytes());emit(r,&peer,Type::Probe,&body)?;}
                    if report_at.elapsed()>Duration::from_millis(100){report_at=Instant::now();let expected=high_seq.map(|h|h.saturating_sub(report_base)).unwrap_or(0);let loss=if expected==0{0}else{expected.saturating_sub(received).saturating_mul(1000)/expected};let mut body=[0;16];body[..4].copy_from_slice(&last_frame.to_be_bytes());body[4..6].copy_from_slice(&(loss.min(1000) as u16).to_be_bytes());body[10..12].copy_from_slice(&rtt.to_be_bytes());emit(r,&peer,Type::Report,&body)?;report_base=high_seq.unwrap_or(0);received=0;}
                    if r.assembler.needs_keyframe() && idr_at.elapsed()>Duration::from_millis(250){idr_at=Instant::now();emit(r,&peer,Type::Idr,&[])?;}
                }
                if ping_at.elapsed()>Duration::from_secs(10){ping_at=Instant::now();control::send(&mut conn.socket,json!({"t":"ping"})).await?;}
                if !turn_started && peer.confirmed.is_none() && start.elapsed()>Duration::from_secs(3) && conn.computer.relay.is_some(){
                    turn_started=true;let computer=conn.computer.clone();let socket=peer.socket.clone();let slot=peer.turn.clone();let peers:Vec<_>=peer.targets.iter().filter_map(|a|if let SocketAddr::V4(a)=a{Some(*a)}else{None}).collect();let tx=turn_tx.clone();let live=peer.live.clone();
                    tokio::spawn(async move{let allocated=if let Some((server,user,pass))=turn_credentials(&computer).await{let t=Arc::new(Turn::new(server,user,pass));*slot.lock().unwrap()=Some(t.clone());{
                        let result=tokio::task::spawn_blocking({let t=t.clone();let socket=socket.clone();let peers=peers.clone();move||{let a=t.allocate(&socket);if a.is_some(){t.permit(&socket,&peers);}a}}).await.ok().flatten();
                        if result.is_some(){tokio::spawn(async move{loop{tokio::time::sleep(Duration::from_secs(200)).await;if !live.load(Ordering::Relaxed){break;}let(t,s,peers)=(t.clone(),socket.clone(),peers.clone());let _=tokio::task::spawn_blocking(move||{t.refresh(&s,600);t.permit(&s,&peers);}).await;}});}
                        result
                    }}else{None};let _=tx.send(allocated).await;});
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
