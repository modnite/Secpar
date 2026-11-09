use crate::pairing::Pairing;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
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

async fn open(url: &str, pin: Option<[u8; 32]>) -> Result<Socket, String> {
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
    tokio::time::timeout(
        Duration::from_secs(6),
        tokio_tungstenite::connect_async_tls_with_config(url, Some(config), true, connector),
    )
    .await
    .map_err(|_| "Connection timed out")?
    .map(|(s, _)| s)
    .map_err(|_| "Could not establish a secure connection".into())
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

impl Connected {
    pub async fn pair(p: Pairing, device: &str) -> Result<Self, String> {
        Self::pair_ordered(p, device, false).await
    }
    /// Account-verified computer: attempt pinned LAN addresses before its remote bootstrap URL.
    pub async fn pair_lan_first(p: Pairing, device: &str) -> Result<Self, String> {
        Self::pair_ordered(p, device, true).await
    }
    async fn pair_ordered(p: Pairing, device: &str, lan_first: bool) -> Result<Self, String> {
        let mut routes: Vec<(String, Option<String>)> =
            p.relay_url.iter().cloned().map(|r| (r, None)).collect();
        routes.extend(
            p.direct_urls()
                .into_iter()
                .zip(p.hosts.iter().cloned().map(Some)),
        );
        if lan_first && p.relay_url.is_some() { routes.rotate_left(1); }
        let mut last = "No connection address".to_string();
        for (url, direct_host) in routes {
            let mut ws = match open(&url, direct_host.as_ref().map(|_| p.fingerprint)).await {
                Ok(s) => s,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            send(
                &mut ws,
                json!({"t":"pair","otp":p.otp,"device":device,"pin":""}),
            )
            .await?;
            let v = tokio::time::timeout(Duration::from_secs(8), next(&mut ws))
                .await
                .map_err(|_| "Host did not answer pairing")??;
            if v["t"] != "paired" {
                return Err(
                    "Pairing was refused. Generate a fresh pairing link on the host.".into(),
                );
            }
            let id = v["pc"]["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("Host did not identify itself")?
                .to_string();
            if p.pc_id.as_ref().is_some_and(|expected| *expected != id) {
                let _ = ws.close(None).await;
                return Err("Host identity does not match the pairing code".into());
            }
            let token = v["token"]
                .as_str()
                .ok_or("Host did not return a device credential")?
                .to_string();
            if base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&token)
                .map(|v| v.len())
                .unwrap_or(0)
                != 32
            {
                return Err("Host returned an invalid device credential".into());
            }
            let r = tokio::time::timeout(Duration::from_secs(2), async {
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
            if direct_host.is_none() && r.is_none() {
                return Err(
                    "Host paired but did not provide remote reconnection credentials".into(),
                );
            }
            let name = v["pc"]["name"].as_str().unwrap_or(&p.name).to_string();
            let computer = Computer {
                id,
                name,
                hosts: p.hosts,
                port: p.port,
                fingerprint: p.fingerprint,
                token,
                relay: r,
            };
            return Ok(Self {
                socket: ws,
                computer,
                direct_host,
                caps: vec![],
            });
        }
        Err(last)
    }
    pub async fn reconnect(computer: Computer, device: &str) -> Result<Self, String> {
        let p = Pairing {
            name: computer.name.clone(),
            hosts: computer.hosts.clone(),
            port: computer.port,
            fingerprint: computer.fingerprint,
            otp: String::new(),
            pc_id: Some(computer.id.clone()),
            relay_url: None,
        };
        let mut routes: Vec<_> = p
            .direct_urls()
            .into_iter()
            .zip(computer.hosts.iter().cloned().map(Some))
            .collect();
        if let Some(r) = &computer.relay {
            routes.push((r.signed_url(&computer.id, now_s(), "phone")?, None));
        }
        let mut last = "Computer is offline".into();
        for (url, direct_host) in routes {
            let mut ws = match open(&url, direct_host.as_ref().map(|_| computer.fingerprint)).await
            {
                Ok(s) => s,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            send(
                &mut ws,
                json!({"t":"hello","v":1,"token":computer.token,"device":device}),
            )
            .await?;
            let v = tokio::time::timeout(Duration::from_secs(8), next(&mut ws))
                .await
                .map_err(|_| "Host did not answer")??;
            if v["t"] != "welcome" {
                return Err("Access was refused or revoked. Pair this computer again.".into());
            }
            if v["pc"]["id"].as_str() != Some(&computer.id) {
                return Err("Host identity changed".into());
            }
            let caps = v["caps"]
                .as_array()
                .map(|a| a.iter().filter_map(|c| c.as_str().map(Into::into)).collect())
                .unwrap_or_default();
            return Ok(Self {
                socket: ws,
                computer,
                direct_host,
                caps,
            });
        }
        Err(last)
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
        let ws = open(&url, None)
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
#[path = "integration.rs"]
mod integration;
