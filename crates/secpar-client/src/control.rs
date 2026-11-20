use crate::{
    connlog,
    pairing::Pairing,
    route::{self, Attempt, Timing},
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::net::TcpStream;
use tokio_rustls::rustls::{
    self,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use tokio_tungstenite::{
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

// These contain credentials. Callers must use an OS credential store, not ordinary UI persistence.
#[derive(Clone, Serialize, Deserialize)]
pub struct Relay {
    pub url: String,
    pub dev: String,
    pub ticket: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Computer {
    pub id: String,
    pub name: String,
    pub hosts: Vec<String>,
    pub port: u16,
    pub fingerprint: [u8; 32],
    pub token: String,
    pub relay: Option<Relay>,
}
pub struct Connected {
    pub socket: Socket,
    pub computer: Computer,
    pub direct_host: Option<String>,
    /// Capabilities the host listed in `welcome`. Empty after a first pairing.
    pub caps: Vec<String>,
    /// Every route that was tried, with times. Addresses only.
    pub route_log: Vec<route::Trace>,
}

impl Relay {
    pub fn signed_url(&self, pc: &str, now: u64, path: &str) -> Result<String, String> {
        if !matches!(path, "phone" | "turn") {
            return Err("Unsupported relay operation".into());
        }
        let mut url = url::Url::parse(&self.url).map_err(|_| "Invalid relay address")?;
        if url.scheme() != "wss"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Invalid relay address".into());
        }
        let key = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&self.ticket)
            .map_err(|_| "Invalid relay ticket")?;
        if key.len() != 32 {
            return Err("Invalid relay ticket".into());
        }
        let tag = ring::hmac::sign(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key),
            format!("phone:{pc}:{}:{now}", self.dev).as_bytes(),
        );
        let sig: String = tag.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        url.set_path(&format!("/v1/{path}"));
        url.set_query(None);
        url.set_fragment(None);
        url.query_pairs_mut()
            .append_pair("pc", pc)
            .append_pair("dev", &self.dev)
            .append_pair("ts", &now.to_string())
            .append_pair("sig", &sig);
        Ok(url.to_string())
    }
}

#[derive(Debug)]
struct Pinned {
    fingerprint: [u8; 32],
    provider: Arc<rustls::crypto::CryptoProvider>,
}
impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let hash = ring::digest::digest(&ring::digest::SHA256, end.as_ref());
        if hash.as_ref() != self.fingerprint {
            return Err(rustls::Error::General(
                "Host certificate does not match the pairing code".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            m,
            c,
            s,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        s: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            m,
            c,
            s,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Connect with the limits for the kind of route: a pinned LAN address or the relay.
#[cfg(test)]
async fn open(url: &str, pin: Option<[u8; 32]>) -> Result<Socket, String> {
    let t = Timing::default();
    match pin {
        Some(_) => open_with(url, pin, t.direct_tcp, t.direct_open, None).await,
        None => open_with(url, pin, t.relay_open, t.relay_open, None).await,
    }
}

/// A TCP connection that gives up after `limit`. The OS waits about 21 s on Windows for a dead
/// address. A name is resolved first and its IPv6 and IPv4 addresses race, so a broken IPv6 route
/// does not hold up the working IPv4 one.
async fn connect_tcp(url: &url::Url, port: u16, limit: Duration) -> Result<TcpStream, String> {
    use url::Host;
    let addrs: Vec<SocketAddr> = match url.host() {
        Some(Host::Ipv4(ip)) => vec![(ip, port).into()],
        Some(Host::Ipv6(ip)) => vec![(ip, port).into()],
        Some(Host::Domain(name)) => {
            let found = tokio::time::timeout(limit, tokio::net::lookup_host((name.to_string(), port)))
                .await
                .map_err(|_| "Connection timed out")?
                .map_err(|_| "Could not establish a secure connection")?;
            let (v6, v4): (Vec<_>, Vec<_>) = found.partition(|a| a.is_ipv6());
            let (mut v6, mut v4) = (v6.into_iter(), v4.into_iter());
            let mut mixed = vec![];
            loop {
                let (a, b) = (v6.next(), v4.next());
                if a.is_none() && b.is_none() {
                    break;
                }
                mixed.extend(a);
                mixed.extend(b);
            }
            mixed
        }
        None => return Err("Invalid address".into()),
    };
    let starts = (0..addrs.len()).map(|i| Duration::from_millis(250) * i as u32).collect();
    let labels = addrs.iter().map(|a| a.to_string()).collect();
    let (won, _) = route::race(labels, starts, |i| {
        let addr = addrs[i];
        async move {
            match tokio::time::timeout(limit, TcpStream::connect(addr)).await {
                Ok(Ok(s)) => Attempt::Done(s),
                Ok(Err(_)) => Attempt::Retry("Could not establish a secure connection".into()),
                Err(_) => Attempt::Retry("Connection timed out".into()),
            }
        }
    })
    .await;
    won.map(|(_, s)| s)
}

/// Names a route in the connection log: its kind and last address part, never a URL.
#[derive(Clone)]
struct Probe {
    log: connlog::Conn,
    route: String,
}
impl Probe {
    fn step(&self, phase: &str, outcome: &str, started: std::time::Instant) {
        self.log.step(phase, &format!("{} {outcome}", self.route), started);
    }
}
/// A short word for why a step failed, safe for the log.
fn why(error: &str) -> &'static str {
    let e = error.to_ascii_lowercase();
    if e.contains("timed out") {
        "timeout"
    } else if e.contains("does not match") || e.contains("certificate") {
        "pin_mismatch"
    } else if e.contains("closed") {
        "closed"
    } else if e.contains("secure connection") {
        "failed"
    } else {
        "error"
    }
}

/// TCP, TLS and the WebSocket upgrade. `tcp` limits the first, `total` all three.
async fn open_with(url: &str, pin: Option<[u8; 32]>, tcp: Duration, total: Duration, probe: Option<&Probe>) -> Result<Socket, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let tls_config = pin.map(|fingerprint| {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ClientConfig::builder_with_provider(provider.clone())
                .with_safe_default_protocol_versions()
                .expect("TLS versions")
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(Pinned {
                    fingerprint,
                    provider,
                }))
                .with_no_client_auth(),
        )
    });
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let parsed = url::Url::parse(url).map_err(|_| "Invalid address")?;
    let port = parsed.port_or_known_default().ok_or("Invalid address")?;
    let work = async {
        let t0 = std::time::Instant::now();
        let stream = match connect_tcp(&parsed, port, tcp).await {
            Ok(s) => s,
            Err(e) => {
                if let Some(p) = probe {
                    p.step("tcp", why(&e), t0);
                }
                return Err(e);
            }
        };
        if let Some(p) = probe {
            p.step("tcp", "ok", t0);
        }
        let _ = stream.set_nodelay(true);
        match tls_config {
            // A pinned computer: TLS and the upgrade are separate steps, so the log can tell them apart.
            Some(cfg) => {
                let name = rustls::pki_types::ServerName::try_from(parsed.host_str().unwrap_or("host").trim_matches(|c| c == '[' || c == ']').to_string())
                    .map_err(|_| "Invalid address".to_string())?;
                let t1 = std::time::Instant::now();
                let tls = match tokio_rustls::TlsConnector::from(cfg).connect(name, stream).await {
                    Ok(tls) => tls,
                    Err(e) => {
                        let pinned = e.to_string().to_ascii_lowercase().contains("does not match");
                        if let Some(p) = probe {
                            p.step("tls", if pinned { "pin_mismatch" } else { "failed" }, t1);
                        }
                        // A different certificate than the one this computer was paired with: say so,
                        // so the app can ask for a new credential instead of reporting "unreachable".
                        return Err(if pinned { "Host certificate does not match the pairing code" } else { "Could not establish a secure connection" }.to_string());
                    }
                };
                if let Some(p) = probe {
                    p.step("tls", "ok", t1);
                }
                let t2 = std::time::Instant::now();
                let ws = tokio_tungstenite::client_async_with_config(url, MaybeTlsStream::Rustls(tls), Some(config)).await;
                if let Some(p) = probe {
                    p.step("ws_upgrade", if ws.is_ok() { "ok" } else { "failed" }, t2);
                }
                ws.map(|(s, _)| s).map_err(|_| "Could not establish a secure connection".to_string())
            }
            // The relay: TLS with the usual roots, together with the upgrade.
            None => {
                let t1 = std::time::Instant::now();
                let ws = tokio_tungstenite::client_async_tls_with_config(url, stream, Some(config), None).await;
                if let Some(p) = probe {
                    p.step("tls_ws", if ws.is_ok() { "ok" } else { "failed" }, t1);
                }
                ws.map(|(s, _)| s).map_err(|_| "Could not establish a secure connection".to_string())
            }
        }
    };
    tokio::time::timeout(total, work)
        .await
        .map_err(|_| "Connection timed out")?
}
pub async fn send(socket: &mut Socket, value: Value) -> Result<(), String> {
    socket
        .send(Message::text(value.to_string()))
        .await
        .map_err(|_| "Connection closed".into())
}
pub async fn next(socket: &mut Socket) -> Result<Value, String> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(t))) => {
                return serde_json::from_str(&t).map_err(|_| "Host sent an invalid response".into())
            }
            Some(Ok(Message::Ping(p))) => socket
                .send(Message::Pong(p))
                .await
                .map_err(|_| "Connection closed")?,
            Some(Ok(Message::Pong(_))) | Some(Ok(Message::Binary(_))) => {}
            _ => return Err("Connection closed".into()),
        }
    }
}
/// One message from the host: JSON text or a binary frame.
pub enum Incoming {
    Text(Value),
    Binary(Vec<u8>),
}
/// Like `next`, but binary frames are returned instead of skipped.
pub async fn next_incoming(socket: &mut Socket) -> Result<Incoming, String> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(t))) => {
                return serde_json::from_str(&t)
                    .map(Incoming::Text)
                    .map_err(|_| "Host sent an invalid response".into())
            }
            Some(Ok(Message::Binary(b))) => return Ok(Incoming::Binary(b.to_vec())),
            Some(Ok(Message::Ping(p))) => socket
                .send(Message::Pong(p))
                .await
                .map_err(|_| "Connection closed")?,
            Some(Ok(Message::Pong(_))) => {}
            _ => return Err("Connection closed".into()),
        }
    }
}
/// The JPEG inside a thumbnail frame (`[0x02][JPEG]`), or None for any other frame.
pub fn thumb_jpeg(frame: &[u8]) -> Option<&[u8]> {
    match frame {
        [2, rest @ ..] if rest.starts_with(&[0xFF, 0xD8]) => Some(rest),
        _ => None,
    }
}
fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn remote(v: &Value) -> Option<Relay> {
    if v["t"] != "remote" {
        return None;
    }
    let r = Relay {
        url: v["url"].as_str()?.into(),
        dev: v["dev"].as_str()?.into(),
        ticket: v["ticket"].as_str()?.into(),
    };
    r.signed_url("validation", 0, "phone").ok()?;
    Some(r)
}

/// Which route worked last for a computer. The next connection tries it first, so the Files page,
/// the previews and the stream that follow one another do not repeat the search. It is also kept in
/// a small file (addresses only, no credentials), so the first connection after the app starts
/// begins with the route that worked last time.
#[derive(Clone, PartialEq, Debug)]
enum Remembered {
    Direct(String),
    Relay,
}
fn remembered() -> &'static Mutex<HashMap<[u8; 32], Remembered>> {
    static LAST: OnceLock<Mutex<HashMap<[u8; 32], Remembered>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(load_routes()))
}
fn routes_file() -> Option<std::path::PathBuf> {
    // Only when an app switched the log on, so tests never touch a user's files.
    connlog::enabled().then(|| connlog::state_dir().map(|d| d.join("routes.json"))).flatten()
}
fn hex32(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}
fn routes_from_json(text: &str) -> HashMap<[u8; 32], Remembered> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else { return HashMap::new() };
    map.into_iter()
        .take(64)
        .filter_map(|(k, v)| {
            let fp = unhex32(&k)?;
            let v = v.as_str()?;
            Some((fp, if v == "relay" { Remembered::Relay } else { Remembered::Direct(v.parse::<std::net::IpAddr>().ok()?.to_string()) }))
        })
        .collect()
}
fn routes_to_json(map: &HashMap<[u8; 32], Remembered>) -> String {
    let json: serde_json::Map<String, Value> = map
        .iter()
        .map(|(k, v)| (hex32(k), Value::String(match v { Remembered::Relay => "relay".into(), Remembered::Direct(h) => h.clone() })))
        .collect();
    Value::Object(json).to_string()
}
fn load_routes() -> HashMap<[u8; 32], Remembered> {
    routes_file().and_then(|p| std::fs::read_to_string(p).ok()).map(|text| routes_from_json(&text)).unwrap_or_default()
}
fn save_routes(map: &HashMap<[u8; 32], Remembered>) {
    let Some(path) = routes_file() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temp = path.with_extension("tmp");
    if std::fs::write(&temp, routes_to_json(map)).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    }
}
fn remember(fingerprint: [u8; 32], route: Remembered) {
    if let Ok(mut map) = remembered().lock() {
        if map.get(&fingerprint) != Some(&route) {
            map.insert(fingerprint, route);
            save_routes(&map);
        }
    }
}

/// How likely a LAN address is to be the one to use from here, lower first. The system is asked
/// which of this computer's own addresses it would send from (no packet is sent). An address in
/// the same /24 is on the same network. Other private addresses might be, public ones rarely,
/// link-local ones almost never. The order the host gave is kept inside each class.
fn locality(host: &str) -> u8 {
    use std::net::{IpAddr, Ipv4Addr, UdpSocket};
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            if ip.is_link_local() {
                return 4;
            }
            let source = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
                .and_then(|s| s.connect((ip, 9)).map(|_| s))
                .and_then(|s| s.local_addr());
            match source {
                Ok(std::net::SocketAddr::V4(mine)) if mine.ip().octets()[..3] == ip.octets()[..3] => 0,
                Ok(_) if ip.is_private() => 1,
                Ok(_) => 2,
                Err(_) => 5,
            }
        }
        Ok(IpAddr::V6(ip)) => {
            if (ip.segments()[0] & 0xffc0) == 0xfe80 {
                4
            } else {
                3
            }
        }
        Err(_) => 2,
    }
}

/// One way to reach a computer. `direct` is the LAN address, None for the relay.
#[derive(Clone)]
struct Route {
    url: String,
    direct: Option<String>,
}
impl Route {
    fn label(&self) -> String {
        self.direct.clone().unwrap_or_else(|| "relay".into())
    }
    async fn open(&self, pin: [u8; 32], t: &Timing, probe: &Probe) -> Result<Socket, String> {
        match self.direct {
            Some(_) => open_with(&self.url, Some(pin), t.direct_tcp, t.direct_open, Some(probe)).await,
            None => open_with(&self.url, None, t.relay_open, t.relay_open, Some(probe)).await,
        }
    }
    /// What the connection log calls this route: `lan4:.131`, `ll6:1` or `relay`.
    fn kind(&self) -> String {
        self.direct.as_deref().map_or_else(|| "relay".into(), connlog::addr)
    }
    /// A host that answers and says no is final on a LAN address. Through the relay it may only
    /// be that the computer is offline, so another route can still work.
    fn refusal<T>(&self, why: impl Into<String>) -> Attempt<T> {
        if self.direct.is_some() {
            Attempt::Fatal(why.into())
        } else {
            Attempt::Retry(why.into())
        }
    }
}

/// The routes in the order they start, with their start times.
/// `relay_first`: the relay was asked for first (a pairing link, or the last connection used it).
fn schedule(
    mut urls: Vec<(String, String)>,
    relay: Option<String>,
    fingerprint: [u8; 32],
    mut relay_first: bool,
    t: &Timing,
) -> (Vec<Route>, Vec<Duration>) {
    urls.sort_by_key(|(host, _)| locality(host));
    match remembered().lock().ok().and_then(|m| m.get(&fingerprint).cloned()) {
        Some(Remembered::Direct(host)) => {
            if let Some(at) = urls.iter().position(|(h, _)| *h == host) {
                let hit = urls.remove(at);
                urls.insert(0, hit);
            }
        }
        Some(Remembered::Relay) if relay.is_some() => relay_first = true,
        _ => {}
    }
    let head = if relay.is_some() && relay_first { t.direct_after_relay } else { Duration::ZERO };
    let mut routes = vec![];
    let mut starts = vec![];
    for (k, (host, url)) in urls.into_iter().enumerate() {
        routes.push(Route { url, direct: Some(host) });
        starts.push(head + t.start_offset(k));
    }
    if let Some(url) = relay {
        routes.push(Route { url, direct: None });
        starts.push(if relay_first { Duration::ZERO } else { t.relay_delay });
    }
    (routes, starts)
}

fn direct_urls(hosts: &[String], port: u16) -> Vec<(String, String)> {
    hosts
        .iter()
        .map(|h| {
            let url = if h.contains(':') { format!("wss://[{h}]:{port}/v1") } else { format!("wss://{h}:{port}/v1") };
            (h.clone(), url)
        })
        .collect()
}

struct Paired {
    ws: Socket,
    id: String,
    name: String,
    token: String,
    remote: Option<Relay>,
}

impl Connected {
    pub async fn pair(p: Pairing, device: &str) -> Result<Self, String> {
        Self::pair_with(p, device, false, &Timing::default()).await
    }
    /// Account-verified computer: attempt pinned LAN addresses before its remote bootstrap URL.
    pub async fn pair_lan_first(p: Pairing, device: &str) -> Result<Self, String> {
        Self::pair_with(p, device, true, &Timing::default()).await
    }
    pub async fn pair_with(p: Pairing, device: &str, lan_first: bool, t: &Timing) -> Result<Self, String> {
        let (routes, starts) = schedule(direct_urls(&p.hosts, p.port), p.relay_url.clone(), p.fingerprint, !lan_first, t);
        let labels = routes.iter().map(Route::label).collect();
        let log = connlog::Conn::new(p.pc_id.as_deref().unwrap_or("pairing"));
        // One route at a time may spend the one-time code. The others wait here and are dropped
        // if that one finishes.
        let turn = Arc::new(tokio::sync::Mutex::new(()));
        let (won, trace) = route::race(labels, starts, |i| {
            let (route, turn) = (routes[i].clone(), turn.clone());
            let (pin, otp, device, expected) = (p.fingerprint, p.otp.clone(), device.to_string(), p.pc_id.clone());
            let name = p.name.clone();
            let probe = Probe { log: log.clone(), route: route.kind() };
            async move {
                let started = std::time::Instant::now();
                let mut ws = match route.open(pin, t, &probe).await {
                    Ok(s) => s,
                    Err(e) => {
                        probe.step("route_try", why(&e), started);
                        return Attempt::Retry(e);
                    }
                };
                let _turn = turn.lock().await;
                let asked = std::time::Instant::now();
                if send(&mut ws, json!({"t":"pair","otp":otp,"device":device,"pin":""})).await.is_err() {
                    probe.step("route_try", "closed", started);
                    return Attempt::Retry("Connection closed".into());
                }
                let v = match tokio::time::timeout(t.answer, next(&mut ws)).await {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        probe.step("pair", why(&e), asked);
                        probe.step("route_try", why(&e), started);
                        return route.refusal(e);
                    }
                    Err(_) => {
                        probe.step("pair", "timeout", asked);
                        probe.step("route_try", "timeout", started);
                        return route.refusal("Host did not answer pairing");
                    }
                };
                probe.step("pair", if v["t"] == "paired" { "ok" } else { "refused" }, asked);
                if v["t"] != "paired" {
                    return Attempt::Fatal("Pairing was refused. Generate a fresh pairing link on the host.".into());
                }
                let Some(id) = v["pc"]["id"].as_str().filter(|s| !s.is_empty()).map(str::to_string) else {
                    return Attempt::Fatal("Host did not identify itself".into());
                };
                if expected.as_ref().is_some_and(|expected| *expected != id) {
                    let _ = ws.close(None).await;
                    return Attempt::Fatal("Host identity does not match the pairing code".into());
                }
                let Some(token) = v["token"].as_str().map(str::to_string) else {
                    return Attempt::Fatal("Host did not return a device credential".into());
                };
                if base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&token).map(|v| v.len()).unwrap_or(0) != 32 {
                    return Attempt::Fatal("Host returned an invalid device credential".into());
                }
                let wait = if route.direct.is_some() { t.remote_wait } else { Duration::from_secs(2) };
                let remote = tokio::time::timeout(wait, async {
                    loop {
                        let m = next(&mut ws).await?;
                        if let Some(r) = remote(&m) {
                            return Ok::<_, String>(r);
                        }
                    }
                })
                .await
                .ok()
                .and_then(Result::ok);
                if route.direct.is_none() && remote.is_none() {
                    return Attempt::Fatal("Host paired but did not provide remote reconnection credentials".into());
                }
                let name = v["pc"]["name"].as_str().unwrap_or(&name).to_string();
                probe.step("route_try", "connected", started);
                Attempt::Done(Paired { ws, id, name, token, remote })
            }
        })
        .await;
        let (i, paired) = won?;
        log.note("welcome", &format!("{} paired", routes[i].kind()), log.began().elapsed().as_millis() as u64);
        let direct_host = routes[i].direct.clone();
        remember(p.fingerprint, direct_host.clone().map_or(Remembered::Relay, Remembered::Direct));
        let computer = Computer {
            id: paired.id,
            name: paired.name,
            hosts: p.hosts,
            port: p.port,
            fingerprint: p.fingerprint,
            token: paired.token,
            relay: paired.remote,
        };
        Ok(Self { socket: paired.ws, computer, direct_host, caps: vec![], route_log: trace })
    }
    pub async fn reconnect(computer: Computer, device: &str) -> Result<Self, String> {
        Self::reconnect_with(computer, device, &Timing::default()).await
    }
    pub async fn reconnect_with(computer: Computer, device: &str, t: &Timing) -> Result<Self, String> {
        let relay = match &computer.relay {
            Some(r) => Some(r.signed_url(&computer.id, now_s(), "phone")?),
            None => None,
        };
        let (routes, starts) = schedule(direct_urls(&computer.hosts, computer.port), relay, computer.fingerprint, false, t);
        if routes.is_empty() {
            return Err("Computer is offline".into());
        }
        let labels = routes.iter().map(Route::label).collect();
        let log = connlog::Conn::new(&computer.id);
        log.note("schedule", &format!("routes={} first={}", routes.len(), routes[0].kind()), 0);
        // The device credential is not single use, but the host counts one connection per device.
        // Only one route talks to it at a time, and a route that loses never says hello.
        let turn = Arc::new(tokio::sync::Mutex::new(()));
        let (won, trace) = route::race(labels, starts, |i| {
            let (route, turn) = (routes[i].clone(), turn.clone());
            let (pin, device, token, id) = (computer.fingerprint, device.to_string(), computer.token.clone(), computer.id.clone());
            let probe = Probe { log: log.clone(), route: route.kind() };
            async move {
                let started = std::time::Instant::now();
                let mut ws = match route.open(pin, t, &probe).await {
                    Ok(s) => s,
                    Err(e) => {
                        probe.step("route_try", why(&e), started);
                        return Attempt::Retry(e);
                    }
                };
                let waited = std::time::Instant::now();
                let _turn = turn.lock().await;
                if waited.elapsed() > Duration::from_millis(5) {
                    probe.step("turn_wait", "ok", waited);
                }
                let asked = std::time::Instant::now();
                if send(&mut ws, json!({"t":"hello","v":1,"token":token,"device":device})).await.is_err() {
                    probe.step("route_try", "closed", started);
                    return Attempt::Retry("Connection closed".into());
                }
                let v = match tokio::time::timeout(t.answer, next(&mut ws)).await {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        probe.step("hello", why(&e), asked);
                        probe.step("route_try", why(&e), started);
                        return Attempt::Retry(e);
                    }
                    Err(_) => {
                        probe.step("hello", "timeout", asked);
                        probe.step("route_try", "timeout", started);
                        return Attempt::Retry("Host did not answer".into());
                    }
                };
                probe.step("hello", if v["t"] == "welcome" { "welcome" } else { "refused" }, asked);
                if v["t"] != "welcome" {
                    probe.step("route_try", "refused", started);
                    return route.refusal("Access was refused or revoked. Pair this computer again.");
                }
                if v["pc"]["id"].as_str() != Some(&id) {
                    probe.step("route_try", "identity", started);
                    return route.refusal("Host identity changed");
                }
                probe.step("route_try", "connected", started);
                let caps: Vec<String> = v["caps"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|c| c.as_str().map(Into::into)).collect())
                    .unwrap_or_default();
                Attempt::Done((ws, caps))
            }
        })
        .await;
        let (i, (socket, caps)) = match won {
            Ok(w) => w,
            Err(e) => {
                log.note("welcome", "failed", log.began().elapsed().as_millis() as u64);
                return Err(e);
            }
        };
        log.note("welcome", &routes[i].kind(), log.began().elapsed().as_millis() as u64);
        let direct_host = routes[i].direct.clone();
        remember(computer.fingerprint, direct_host.clone().map_or(Remembered::Relay, Remembered::Direct));
        Ok(Self { socket, computer, direct_host, caps, route_log: trace })
    }
}

/// The relay origin a friend's computer is reached through. It must be a plain https origin.
/// Returns the wss address of the guest route for one computer.
pub fn guest_url(relay: &str, pc: &str) -> Result<String, String> {
    let bad = || "The computer gave an invalid relay address".to_string();
    if relay.len() > 300 || relay.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(bad());
    }
    if pc.is_empty()
        || pc.len() > 64
        || !pc.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("The computer gave an invalid identifier".into());
    }
    let mut url = url::Url::parse(relay).map_err(|_| bad())?;
    if url.scheme() != "https"
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(bad());
    }
    url.set_scheme("wss").map_err(|_| bad())?;
    url.set_path("/v1/guest");
    url.query_pairs_mut().append_pair("pc", pc);
    Ok(url.to_string())
}

/// Plain words for an `error` frame that ends a friend connection. The host's own text is kept
/// where it says more than the code does.
pub fn guest_error(v: &Value) -> String {
    let msg: String = v["msg"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    let msg = msg.trim();
    match v["code"].as_str().unwrap_or("") {
        "guest_refused" => "Access was removed, or this computer stopped sharing with friends.".into(),
        "busy" if !msg.is_empty() => format!("This computer is busy. {msg}"),
        "busy" => "This computer is busy. Try again later.".into(),
        "locked" => "This computer is locked.".into(),
        "forbidden" => "The computer does not allow that.".into(),
        "guest_ended" if !msg.is_empty() => msg.into(),
        "guest_ended" => "The owner ended the session.".into(),
        _ if !msg.is_empty() => msg.into(),
        _ => "The computer refused the connection.".into(),
    }
}

impl Connected {
    /// Connect to a friend's computer with a grant from the account service. The relay is a
    /// public service, so its certificate is checked normally and nothing is pinned. A friend has
    /// no relay ticket, so the returned computer has no TURN route of its own.
    pub async fn guest(relay: &str, pc: &str, grant: &str, fallback_name: &str) -> Result<Self, String> {
        let url = guest_url(relay, pc)?;
        let t = Timing::default();
        let ws = open_with(&url, None, t.relay_open, t.relay_open, None)
            .await
            .map_err(|_| "Could not reach the computer through the relay".to_string())?;
        Self::guest_over(ws, pc, grant, fallback_name).await
    }
    async fn guest_over(mut ws: Socket, pc: &str, grant: &str, fallback_name: &str) -> Result<Self, String> {
        if grant.is_empty() || grant.len() > 4096 {
            return Err("The connection grant is invalid".into());
        }
        send(&mut ws, json!({"t":"guest","grant":grant})).await?;
        let welcome = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                match ws.next().await {
                    Some(Ok(Message::Text(t))) => {
                        let v: Value = serde_json::from_str(&t)
                            .map_err(|_| "Host sent an invalid response".to_string())?;
                        match v["t"].as_str() {
                            Some("welcome") => return Ok(v),
                            Some("error") => return Err(guest_error(&v)),
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Ping(p))) => ws
                        .send(Message::Pong(p))
                        .await
                        .map_err(|_| "Connection closed".to_string())?,
                    Some(Ok(Message::Close(Some(frame)))) => {
                        return Err(if u16::from(frame.code) == 4401 {
                            "The relay refused the connection".into()
                        } else {
                            "The computer is offline or stopped sharing".into()
                        })
                    }
                    Some(Ok(_)) => {}
                    _ => return Err("The computer is offline or stopped sharing".into()),
                }
            }
        })
        .await
        .map_err(|_| "The computer did not answer".to_string())??;
        let name = welcome["pc"]["name"]
            .as_str()
            .map(|n| n.chars().filter(|c| !c.is_control()).take(80).collect::<String>())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| fallback_name.to_string());
        let caps = welcome["caps"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).take(16).map(String::from).collect())
            .unwrap_or_default();
        Ok(Self {
            socket: ws,
            computer: Computer {
                id: pc.into(),
                name,
                hosts: Vec::new(),
                port: 0,
                fingerprint: [0; 32],
                token: String::new(),
                relay: None,
            },
            direct_host: None,
            caps,
            route_log: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    #[test]
    fn thumbnail_frames_carry_a_jpeg_after_one_marker_byte() {
        assert_eq!(thumb_jpeg(&[2, 0xFF, 0xD8, 1, 2]), Some(&[0xFF, 0xD8, 1, 2][..]));
        assert_eq!(thumb_jpeg(&[2]), None);
        assert_eq!(thumb_jpeg(&[2, 0x89, 0x50]), None);
        assert_eq!(thumb_jpeg(&[1, 0xFF, 0xD8]), None);
        assert_eq!(thumb_jpeg(&[]), None);
    }
    #[tokio::test]
    async fn a_name_is_resolved_and_connected_quickly_even_if_one_family_is_dead() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = url::Url::parse(&format!("wss://localhost:{port}/v1")).unwrap();
        let started = std::time::Instant::now();
        assert!(connect_tcp(&url, port, Duration::from_millis(500)).await.is_ok());
        assert!(started.elapsed() < Duration::from_millis(900));
    }
    use tokio_rustls::{rustls::pki_types::PrivatePkcs8KeyDer, TlsAcceptor};
    #[tokio::test]
    async fn pinned_tls_pairs_reconnects_and_rejects_other_certificate() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin: [u8; 32] = ring::digest::digest(&ring::digest::SHA256, cert.der())
            .as_ref()
            .try_into()
            .unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
            )
            .unwrap();
        let accept = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7; 32]);
        let server_token = token.clone();
        let server = tokio::spawn(async move {
            for mode in ["pair", "pair", "hello", "bad-certificate"] {
                let (s, _) = listener.accept().await.unwrap();
                let tls = accept.accept(s).await;
                if mode == "bad-certificate" {
                    assert!(tls.is_err());
                    continue;
                }
                let mut ws = tokio_tungstenite::accept_async(tls.unwrap()).await.unwrap();
                let m = ws.next().await.unwrap().unwrap().into_text().unwrap();
                let m: Value = serde_json::from_str(&m).unwrap();
                assert_eq!(m["t"], mode);
                if mode == "hello" {
                    assert_eq!(m["token"], server_token);
                }
                ws.send(Message::text(json!({"t":if mode=="pair"{"paired"}else{"welcome"},"pc":{"id":"pc1","name":"Test PC"},"token":server_token}).to_string())).await.unwrap();
                if mode == "pair" {
                    ws.send(Message::text(json!({"t":"remote","url":"wss://relay.example","dev":"desktop1","ticket":server_token}).to_string())).await.unwrap();
                }
                while let Some(Ok(m)) = ws.next().await {
                    if m.is_close() {
                        break;
                    }
                }
            }
        });
        let p = Pairing {
            name: "Test".into(),
            hosts: vec!["127.0.0.1".into()],
            port: addr.port(),
            fingerprint: pin,
            otp: "once".into(),
            relay_url: None,
            pc_id: Some("pc1".into()),
        };
        let mut manual = Connected::pair(p.clone(), "Desktop").await.unwrap();
        manual.socket.close(None).await.unwrap();
        drop(manual);
        let unused_relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut owned = p;
        owned.relay_url = Some(format!("wss://{}/v1/phone?pc=pc1", unused_relay.local_addr().unwrap()));
        let mut c = Connected::pair_lan_first(owned, "Desktop").await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(20), unused_relay.accept()).await.is_err(), "A working LAN route must not contact the relay");
        assert!(c.computer.relay.is_some());
        assert_eq!(c.computer.token, token);
        let computer = c.computer.clone();
        c.socket.close(None).await.unwrap();
        drop(c);
        let mut c = Connected::reconnect(computer, "Desktop").await.unwrap();
        c.socket.close(None).await.unwrap();
        drop(c);
        assert_eq!(
            open(&format!("wss://{addr}/v1"), Some([0; 32])).await.err().as_deref(),
            Some("Host certificate does not match the pairing code"),
            "a computer that was set up again says so, which the app answers with a new credential"
        );
        server.await.unwrap();
    }

    #[test]
    fn guest_url_turns_an_https_origin_into_the_guest_route() {
        assert_eq!(guest_url("https://relay.example.com", "abc-123_X").unwrap(), "wss://relay.example.com/v1/guest?pc=abc-123_X");
        assert_eq!(guest_url("https://relay.example.com/", "pc1").unwrap(), "wss://relay.example.com/v1/guest?pc=pc1");
        assert_eq!(guest_url("https://relay.example.com:8443", "pc1").unwrap(), "wss://relay.example.com:8443/v1/guest?pc=pc1");
    }
    #[test]
    fn guest_url_refuses_bad_relays() {
        for relay in [
            "http://relay.example.com",
            "ws://relay.example.com",
            "wss://relay.example.com",
            "ftp://relay.example.com",
            "relay.example.com",
            "",
            "https://",
            "https://user:pw@relay.example.com",
            "https://user@relay.example.com",
            "https://relay.example.com/v1/phone",
            "https://relay.example.com/?x=1",
            "https://relay.example.com/#frag",
            "https://relay.example.com\n",
            "https://relay .example.com",
        ] {
            assert!(guest_url(relay, "pc1").is_err(), "{relay:?} must be refused");
        }
    }
    #[test]
    fn guest_url_refuses_identifiers_that_could_inject_a_path_or_query() {
        for pc in ["", "a/b", "a?b=1", "a&dev=x", "a#b", "..", "a b", "a%2fb", "pc\n", &"x".repeat(65)] {
            assert!(guest_url("https://relay.example.com", pc).is_err(), "{pc:?} must be refused");
        }
    }
    #[test]
    fn guest_errors_are_put_in_plain_words() {
        let e = |v: Value| guest_error(&v);
        assert!(e(json!({"t":"error","code":"guest_refused"})).contains("Access was removed"));
        assert_eq!(e(json!({"t":"error","code":"locked","msg":"x"})), "This computer is locked.");
        assert_eq!(e(json!({"t":"error","code":"busy"})), "This computer is busy. Try again later.");
        assert_eq!(e(json!({"t":"error","code":"busy","msg":"The owner is using the PC."})), "This computer is busy. The owner is using the PC.");
        assert_eq!(e(json!({"t":"error","code":"guest_ended","msg":"Owner started a stream"})), "Owner started a stream");
        assert_eq!(e(json!({"t":"error","code":"guest_ended"})), "The owner ended the session.");
        assert_eq!(e(json!({"t":"error","code":"mystery","msg":"Host says no\u{7}"})), "Host says no");
        assert_eq!(e(json!({"t":"error"})), "The computer refused the connection.");
    }
    async fn guest_server(replies: Vec<Value>) -> (String, tokio::task::JoinHandle<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            let first = ws.next().await.unwrap().unwrap().into_text().unwrap();
            for r in replies {
                ws.send(Message::text(r.to_string())).await.unwrap();
            }
            serde_json::from_str(&first).unwrap()
        });
        (format!("ws://{addr}/v1/guest?pc=pc1"), task)
    }
    #[tokio::test]
    async fn guest_sends_the_grant_first_and_reads_the_welcome() {
        let (url, server) = guest_server(vec![json!({"t":"welcome","v":1,"pc":{"id":"pc1","name":"Friend PC"},"caps":["stream","input","pad"]})]).await;
        let ws = open(&url, None).await.unwrap();
        let c = Connected::guest_over(ws, "pc1", "the-grant", "Fallback").await.unwrap();
        assert_eq!(server.await.unwrap(), json!({"t":"guest","grant":"the-grant"}));
        assert_eq!(c.computer.name, "Friend PC");
        assert_eq!(c.computer.id, "pc1");
        assert!(c.computer.relay.is_none() && c.direct_host.is_none() && c.computer.token.is_empty());
        assert_eq!(c.caps, ["stream", "input", "pad"]);
    }
    #[tokio::test]
    async fn guest_surfaces_a_host_refusal_and_skips_unrelated_frames() {
        let (url, server) = guest_server(vec![json!({"t":"pong"}), json!({"t":"error","code":"locked","msg":"Locked"})]).await;
        let ws = open(&url, None).await.unwrap();
        let e = Connected::guest_over(ws, "pc1", "g", "Fallback").await.err().unwrap();
        assert_eq!(e, "This computer is locked.");
        server.await.unwrap();
        let (url, server) = guest_server(vec![]).await;
        let ws = open(&url, None).await.unwrap();
        let e = Connected::guest_over(ws, "pc1", &"x".repeat(4097), "Fallback").await.err().unwrap();
        assert_eq!(e, "The connection grant is invalid");
        server.abort();
    }
    #[tokio::test]
    async fn guest_to_a_bad_relay_never_opens_a_socket() {
        let e = Connected::guest("http://127.0.0.1:9", "pc1", "g", "F").await.err().unwrap();
        assert_eq!(e, "The computer gave an invalid relay address");
    }
}

#[cfg(test)]
mod speed_tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_rustls::{rustls::pki_types::PrivatePkcs8KeyDer, TlsAcceptor};

    #[test]
    fn the_address_on_this_network_goes_first() {
        assert_eq!(locality("127.0.0.1"), 0, "same network as the address the system would send from");
        assert!(locality("169.254.10.20") > locality("127.0.0.1"));
        assert!(locality("fe80::1") > locality("2001:db8::1"));
        let urls = direct_urls(&["169.254.10.20".into(), "fe80::1".into(), "127.0.0.1".into()], 48999);
        let (routes, starts) = schedule(urls, None, [7; 32], false, &Timing::default());
        let order: Vec<_> = routes.iter().map(Route::label).collect();
        assert_eq!(order, ["127.0.0.1", "169.254.10.20", "fe80::1"], "link-local addresses wait their turn");
        assert_eq!(starts[0], Duration::ZERO);
    }

    #[test]
    fn remembered_routes_survive_a_restart_and_ignore_junk() {
        let mut map = HashMap::new();
        map.insert([1u8; 32], Remembered::Direct("192.168.100.5".into()));
        map.insert([2u8; 32], Remembered::Relay);
        let text = routes_to_json(&map);
        assert_eq!(routes_from_json(&text), map);
        assert!(!text.contains("token"), "addresses only");
        assert!(routes_from_json("not json").is_empty());
        let junk = format!("{{\"{}\":\"https://evil.example/\",\"zz\":\"relay\",\"{}\":7}}", hex32(&[3; 32]), hex32(&[4; 32]));
        assert!(routes_from_json(&junk).is_empty(), "only an IP address or the word relay is taken back");
    }

    /// A real connection with the log on: every step is there, the token is not.
    #[tokio::test]
    async fn a_connection_leaves_a_readable_log_without_secrets() {
        let _serial = crate::connlog::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = rustls::crypto::ring::default_provider().install_default();
        let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin: [u8; 32] = ring::digest::digest(&ring::digest::SHA256, cert.der()).as_ref().try_into().unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into())
            .unwrap();
        let accept = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9; 32]);
        let server_token = token.clone();
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(accept.accept(s).await.unwrap()).await.unwrap();
            let m = ws.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(m.contains(&server_token));
            ws.send(Message::text(json!({"t":"welcome","pc":{"id":"abcdef0123456789","name":"X"},"caps":["files"]}).to_string())).await.unwrap();
            while ws.next().await.is_some() {}
        });
        let dir = std::env::temp_dir().join(format!("secpar-connlog-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::connlog::set_path(Some(dir.join("connect.log")));
        let computer = Computer { id: "abcdef0123456789".into(), name: "X".into(), hosts: vec!["127.0.0.1".into()], port, fingerprint: pin, token: token.clone(), relay: None };
        let c = Connected::reconnect(computer, "Desktop").await.unwrap();
        crate::connlog::flush();
        crate::connlog::set_path(None);
        drop(c);
        let text = std::fs::read_to_string(dir.join("connect.log")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        for phase in [" schedule ", " tcp ", " tls ", " ws_upgrade ", " hello ", " route_try ", " welcome "] {
            assert!(text.contains(phase), "missing{phase}in\n{text}");
        }
        assert!(text.contains("lo4:.1 connected") && text.contains(" abcdef01 "), "{text}");
        assert!(!text.contains(&token) && !text.contains("127.0.0.1") && !text.contains(&token[..12]), "{text}");
        assert!(text.lines().all(|l| l.contains(" ms=") && l.contains(" at=")), "{text}");
    }
}

#[cfg(test)]
#[path = "integration.rs"]
mod integration;

#[cfg(test)]
#[path = "route_tests.rs"]
mod route_tests;
