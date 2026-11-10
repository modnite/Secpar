use crate::{
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
    Connector, MaybeTlsStream, WebSocketStream,
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
        Some(_) => open_with(url, pin, t.direct_tcp, t.direct_open).await,
        None => open_with(url, pin, t.relay_open, t.relay_open).await,
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

/// TCP, TLS and the WebSocket upgrade. `tcp` limits the first, `total` all three.
async fn open_with(url: &str, pin: Option<[u8; 32]>, tcp: Duration, total: Duration) -> Result<Socket, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let connector = pin.map(|fingerprint| {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Pinned {
                fingerprint,
                provider,
            }))
            .with_no_client_auth();
        Connector::Rustls(Arc::new(cfg))
    });
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let parsed = url::Url::parse(url).map_err(|_| "Invalid address")?;
    let port = parsed.port_or_known_default().ok_or("Invalid address")?;
    let work = async {
        let stream = connect_tcp(&parsed, port, tcp).await?;
        let _ = stream.set_nodelay(true);
        tokio_tungstenite::client_async_tls_with_config(url, stream, Some(config), connector)
            .await
            .map(|(s, _)| s)
            .map_err(|_| "Could not establish a secure connection".to_string())
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
/// the previews and the stream that follow one another do not repeat the search.
#[derive(Clone, PartialEq, Debug)]
enum Remembered {
    Direct(String),
    Relay,
}
fn remembered() -> &'static Mutex<HashMap<[u8; 32], Remembered>> {
    static LAST: OnceLock<Mutex<HashMap<[u8; 32], Remembered>>> = OnceLock::new();
    LAST.get_or_init(Default::default)
}
fn remember(fingerprint: [u8; 32], route: Remembered) {
    if let Ok(mut map) = remembered().lock() {
        map.insert(fingerprint, route);
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
    async fn open(&self, pin: [u8; 32], t: &Timing) -> Result<Socket, String> {
        match self.direct {
            Some(_) => open_with(&self.url, Some(pin), t.direct_tcp, t.direct_open).await,
            None => open_with(&self.url, None, t.relay_open, t.relay_open).await,
        }
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
        // One route at a time may spend the one-time code. The others wait here and are dropped
        // if that one finishes.
        let turn = Arc::new(tokio::sync::Mutex::new(()));
        let (won, trace) = route::race(labels, starts, |i| {
            let (route, turn) = (routes[i].clone(), turn.clone());
            let (pin, otp, device, expected) = (p.fingerprint, p.otp.clone(), device.to_string(), p.pc_id.clone());
            let name = p.name.clone();
            async move {
                let mut ws = match route.open(pin, t).await {
                    Ok(s) => s,
                    Err(e) => return Attempt::Retry(e),
                };
                let _turn = turn.lock().await;
                if send(&mut ws, json!({"t":"pair","otp":otp,"device":device,"pin":""})).await.is_err() {
                    return Attempt::Retry("Connection closed".into());
                }
                let v = match tokio::time::timeout(t.answer, next(&mut ws)).await {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return route.refusal(e),
                    Err(_) => return route.refusal("Host did not answer pairing"),
                };
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
                Attempt::Done(Paired { ws, id, name, token, remote })
            }
        })
        .await;
        let (i, paired) = won?;
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
        // The device credential is not single use, but the host counts one connection per device.
        // Only one route talks to it at a time, and a route that loses never says hello.
        let turn = Arc::new(tokio::sync::Mutex::new(()));
        let (won, trace) = route::race(labels, starts, |i| {
            let (route, turn) = (routes[i].clone(), turn.clone());
            let (pin, device, token, id) = (computer.fingerprint, device.to_string(), computer.token.clone(), computer.id.clone());
            async move {
                let mut ws = match route.open(pin, t).await {
                    Ok(s) => s,
                    Err(e) => return Attempt::Retry(e),
                };
                let _turn = turn.lock().await;
                if send(&mut ws, json!({"t":"hello","v":1,"token":token,"device":device})).await.is_err() {
                    return Attempt::Retry("Connection closed".into());
                }
                let v = match tokio::time::timeout(t.answer, next(&mut ws)).await {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return Attempt::Retry(e),
                    Err(_) => return Attempt::Retry("Host did not answer".into()),
                };
                if v["t"] != "welcome" {
                    return route.refusal("Access was refused or revoked. Pair this computer again.");
                }
                if v["pc"]["id"].as_str() != Some(&id) {
                    return route.refusal("Host identity changed");
                }
                let caps: Vec<String> = v["caps"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|c| c.as_str().map(Into::into)).collect())
                    .unwrap_or_default();
                Attempt::Done((ws, caps))
            }
        })
        .await;
        let (i, (socket, caps)) = won?;
        let direct_host = routes[i].direct.clone();
        remember(computer.fingerprint, direct_host.clone().map_or(Remembered::Relay, Remembered::Direct));
        Ok(Self { socket, computer, direct_host, caps, route_log: trace })
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
        assert!(open(&format!("wss://{addr}/v1"), Some([0; 32]))
            .await
            .is_err());
        server.await.unwrap();
    }
}

#[cfg(test)]
#[path = "integration.rs"]
mod integration;

#[cfg(test)]
#[path = "route_tests.rs"]
mod route_tests;
