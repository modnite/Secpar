//! The host's one WebSocket to the rendezvous Worker (docs/remote-protocol.md), the same for
//! every desktop host: signing, registration of the relay key, reconnect with backoff, and the
//! hand-over of each relayed phone to the agent (`agent::relay::serve`). The platform only says
//! where its relay key and PC id live (`RelaySource`).
//!
//! Nothing here ever logs a URL: the host URL carries the signature and, on first contact, the
//! relay key itself.

use crate::agent::{relay, AgentHandle, MAX_FRAME};
use futures_util::StreamExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls::{self, pki_types::ServerName};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};

const BACKOFF_S: [u64; 5] = [1, 2, 5, 10, 30];

/// The deployed Worker; `PCR_RENDEZVOUS_URL` at build time overrides it (empty turns it off).
const DEFAULT_URL: &str = "wss://pcr-rendezvous.sadikhossain747.workers.dev";

/// The Worker's base URL, or None when remote access is built out.
pub fn url() -> Option<&'static str> {
    let u = option_env!("PCR_RENDEZVOUS_URL").unwrap_or(DEFAULT_URL);
    (!u.is_empty()).then_some(u)
}

/// Where a host keeps what the Worker connection needs.
pub trait RelaySource: Send {
    fn pc_id(&self) -> String;
    /// The 32 random bytes only this host (and the Worker, after registration) knows.
    fn relay_key(&self) -> Vec<u8>;
    /// The Worker accepted the key already (later connections leave out `register`).
    fn relay_registered(&self) -> bool;
    fn set_relay_registered(&mut self) -> std::io::Result<()>;
    /// Devices removed on this host, repeated to the Worker on every connection.
    fn revoked(&self) -> Vec<String>;
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn b64url(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

fn hmac(key: &[u8], message: &str) -> [u8; 32] {
    ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key), message.as_bytes())
        .as_ref()
        .try_into()
        .expect("SHA256 length")
}

/// `sig = HMAC-SHA256(relayKey, "host:" + pc + ":" + ts)`, hex.
pub fn host_sig(relay_key: &[u8], pc: &str, ts: u64) -> String {
    hex(&hmac(relay_key, &format!("host:{pc}:{ts}")))
}

/// `HMAC-SHA256(relayKey, "ticket:" + pc + ":" + device)`: what a paired phone presents.
pub fn ticket(relay_key: &[u8], pc: &str, device: &str) -> [u8; 32] {
    hmac(relay_key, &format!("ticket:{pc}:{device}"))
}

/// `<base>/v1/host?pc=..&ts=..&sig=..[&register=..]`
pub fn host_url(base: &str, pc: &str, relay_key: &[u8], ts: u64, register: bool) -> String {
    let mut u = format!("{base}/v1/host?pc={pc}&ts={ts}&sig={}", host_sig(relay_key, pc, ts));
    if register {
        u += &format!("&register={}", b64url(relay_key));
    }
    u
}

/// A fixed phone URL for the pairing link, good for about two minutes under the Worker's
/// +/-600 s timestamp window (docs/remote-protocol.md, "Secpar remote pairing bootstrap").
/// Only a signature made with `ticket` is exported, never the ticket.
pub fn pairing_relay_url(base: &str, pc: &str, dev: &str, ticket: &[u8], now: u64) -> String {
    let ts = now.saturating_sub(480);
    let sig = hex(&hmac(ticket, &format!("phone:{pc}:{dev}:{ts}")));
    format!("{}/v1/phone?pc={pc}&dev={dev}&ts={ts}&sig={sig}", base.trim_end_matches('/'))
}

pub fn text_env(c: u32, m: &str) -> Message {
    Message::text(serde_json::json!({ "c": c, "m": m }).to_string())
}

pub fn bin_env(c: u32, b: &[u8]) -> Message {
    let mut v = Vec::with_capacity(4 + b.len());
    v.extend_from_slice(&c.to_be_bytes());
    v.extend_from_slice(b);
    Message::binary(v)
}

pub fn close_env(c: u32) -> Message {
    Message::text(serde_json::json!({ "c": c, "o": "close" }).to_string())
}

pub fn revoke_env(dev: &str) -> Message {
    crate::agent::revoke_envelope(dev)
}

#[doc(hidden)]
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
pub type Ws = tokio_tungstenite::WebSocketStream<Box<dyn Io>>;

/// A WebSocket to the Worker (`wss://` checks the certificate against the system's roots).
pub async fn connect(url: &str) -> Result<Ws, tungstenite::Error> {
    let req = url.into_client_request()?;
    let uri = req.uri();
    let host = uri.host().unwrap_or_default().to_string();
    let tls = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    let tcp = tokio::net::TcpStream::connect((host.as_str(), port)).await?;
    let _ = tcp.set_nodelay(true);
    let io: Box<dyn Io> = if tls {
        use rustls_platform_verifier::BuilderVerifierExt;
        let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .and_then(|b| b.with_platform_verifier())
            .map_err(|e| tungstenite::Error::Io(std::io::Error::other(e)))?
            .with_no_client_auth();
        let name = ServerName::try_from(host).map_err(|e| tungstenite::Error::Io(std::io::Error::other(e)))?;
        Box::new(tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(name, tcp).await?)
    } else {
        Box::new(tcp)
    };
    let cfg = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME + 64))
        .max_frame_size(Some(MAX_FRAME + 64));
    Ok(tokio_tungstenite::client_async_with_config(req, io, Some(cfg)).await?.0)
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn url_for(base: &str, source: &Mutex<dyn RelaySource>, register: bool) -> String {
    let s = source.lock().unwrap();
    host_url(base, &s.pc_id(), &s.relay_key(), unix_now(), register)
}

/// Connects, registering the relay key when the Worker doesn't know it yet (or forgot it: a 401
/// for a key we believed registered is retried once with `register`).
async fn open(base: &str, source: &Mutex<dyn RelaySource>) -> Result<Ws, tungstenite::Error> {
    let registered = source.lock().unwrap().relay_registered();
    let ws = match connect(&url_for(base, source, !registered)).await {
        Err(tungstenite::Error::Http(r)) if registered && r.status() == 401 => connect(&url_for(base, source, true)).await?,
        r => r?,
    };
    if !registered {
        let _ = source.lock().unwrap().set_relay_registered();
    }
    Ok(ws)
}

/// Keeps the Worker connection up for the life of the process. `log` gets short messages that
/// never contain a URL or a key.
pub async fn run(agent: AgentHandle, source: Arc<Mutex<dyn RelaySource>>, base: String, log: impl Fn(&str) + Send + Sync) {
    let mut fails = 0;
    loop {
        match open(&base, &source).await {
            Ok(ws) => {
                fails = 0;
                log("rendezvous: connected");
                let revoked = source.lock().unwrap().revoked();
                let (writer, reader) = ws.split();
                relay::serve(agent.clone(), reader, writer, revoked).await;
                log("rendezvous: disconnected");
            }
            Err(e) => log(&format!("rendezvous: {e}")),
        }
        tokio::time::sleep(Duration::from_secs(BACKOFF_S[fails.min(BACKOFF_S.len() - 1)])).await;
        fails += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_match_the_worker_vectors() {
        // Expected values from Node's crypto.createHmac, independent of ring.
        let key = [7u8; 32];
        assert_eq!(host_sig(&key, "pc1", 1_700_000_000), "9e9f4441bcfd13a524fd922878e5e207cc90974f6f56c46410c2b866a1cc63c1");
        assert_eq!(b64url(&ticket(&key, "pc1", "dev1")), "Apu-dBssItiqztSY2R7lwB90YggnWttqG85Fei6VQzs");
        assert_ne!(ticket(&key, "pc1", "dev1"), ticket(&key, "pc2", "dev1"));
    }

    #[test]
    fn host_url_signs_and_registers() {
        let key = [7u8; 32];
        let u = host_url("wss://w.example", "pc1", &key, 1_700_000_000, false);
        assert_eq!(u, format!("wss://w.example/v1/host?pc=pc1&ts=1700000000&sig={}", host_sig(&key, "pc1", 1_700_000_000)));
        assert!(!u.contains("register"));
        assert!(host_url("wss://w.example", "pc1", &key, 1, true).ends_with(&format!("&register={}", b64url(&key))));
    }

    #[test]
    fn pairing_url_is_a_signature_that_expires_with_the_code() {
        let t = [3u8; 32];
        let u = pairing_relay_url("wss://w.example/", "pc1", "pair_x", &t, 1_000);
        assert!(u.starts_with("wss://w.example/v1/phone?pc=pc1&dev=pair_x&ts=520&sig="));
        assert!(!u.contains(&b64url(&t)) && !u.contains(&hex(&t)), "the ticket itself is not exported");
        assert!(pairing_relay_url("wss://w.example", "pc1", "pair_x", &t, 100).contains("ts=0&"));
    }

    #[test]
    fn envelopes_have_the_documented_shape() {
        let json = |m: Message| serde_json::from_str::<serde_json::Value>(m.to_text().unwrap()).unwrap();
        assert_eq!(json(text_env(3, r#"{"t":"pong"}"#)), serde_json::json!({"c":3,"m":"{\"t\":\"pong\"}"}));
        assert_eq!(bin_env(0x01020304, &[5]).into_data().to_vec(), vec![1, 2, 3, 4, 5]);
        assert_eq!(json(close_env(3)), serde_json::json!({"c":3,"o":"close"}));
        assert_eq!(json(revoke_env("ab")), serde_json::json!({"o":"revoke","dev":"ab"}));
    }
}
