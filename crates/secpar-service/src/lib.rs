//! The background part of the Secpar app on a desktop computer.
//!
//! It holds the device key, so it is the only part that speaks as this device:
//! it accepts tunnels when the computer hosts, and it opens tunnels to other
//! computers when the person connects somewhere. The window is the part the
//! person signs in to; it talks to the account service with their session and
//! asks this part, over a channel that never leaves the computer, to sign an
//! enrolment or to open a tunnel. The password and the session never come here.
use secpar_core::{
    account::{self, Challenge, Platform},
    hosting::{Access, Refreshed},
    identity::{hex, unhex},
};
use secpar_hosting::{request_pairing, Engine, Hosting, Pairing, ENGINE_BASE};
use secpar_tunnel::{Identity, Ports};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub mod channel;

pub struct Settings {
    /// The account service this device belongs to.
    pub account: account::Service,
    /// A private folder for this part's own files.
    pub state: PathBuf,
    /// Where the tunnel listens when this computer hosts.
    pub bind: SocketAddr,
}

/// What the window may ask for. One JSON object per line, answered by one line.
#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Request {
    Status,
    /// Sign an enrolment for the challenge the window fetched with the session.
    Enrolment { challenge: Challenge, name: String },
    /// Fetch the access list now.
    Refresh,
    /// The owner accepted a friend who is asked about each time.
    Accept { device: String, seconds: u64 },
    Dismiss { device: String },
    /// Open a tunnel to another computer, known by its certificate.
    Connect { host: String, addresses: Vec<SocketAddr> },
    Disconnect,
    /// Have the other computer complete the stream component's pairing.
    Pair { pairing: Pairing },
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
fn failed(code: &str) -> Value {
    json!({"ok": false, "error": code})
}

pub struct Core<E: Engine> {
    identity: Identity,
    account: account::Service,
    /// Present when this computer hosts.
    hosting: Option<Hosting<E>>,
    /// The tunnel to the computer this one is connected to, if any.
    outgoing: tokio::sync::Mutex<Option<secpar_tunnel::Client>>,
}
impl<E: Engine> Core<E> {
    /// Starts with the device's identity. With an engine this computer hosts:
    /// the tunnel opens for whoever the saved access list still allows.
    pub async fn start(settings: Settings, identity: Identity, engine: Option<Arc<E>>) -> Result<Arc<Self>, &'static str> {
        std::fs::create_dir_all(&settings.state).map_err(|_| "The state folder cannot be used")?;
        let hosting = match engine {
            None => None,
            Some(engine) => {
                let access = Arc::new(Access::open(settings.account.origin(), identity.fingerprint(), settings.state.join("access.json"), now()));
                let loopback = "127.0.0.1".parse().map_err(|_| "No loopback address")?;
                Some(Hosting::start(settings.bind, &identity, access, engine, loopback, Ports::engine(ENGINE_BASE)).await.map_err(|e| e.0)?)
            }
        };
        Ok(Arc::new(Self { identity, account: settings.account, hosting, outgoing: tokio::sync::Mutex::new(None) }))
    }
    /// The tunnel's address when this computer hosts.
    pub fn tunnel_address(&self) -> Option<SocketAddr> {
        self.hosting.as_ref().and_then(|h| h.tunnel().local_addr().ok())
    }
    /// Keeps the access list and the tunnel current. Returns at once when this
    /// computer does not host; otherwise never.
    pub async fn keep_current(&self) {
        if let Some(hosting) = &self.hosting {
            hosting.keep_current(&self.account, &self.identity).await;
        }
    }

    /// Answers one line from the window.
    pub async fn answer(&self, line: &str) -> Value {
        let Ok(request) = serde_json::from_str::<Request>(line) else { return failed("unknown_request") };
        match request {
            Request::Status => {
                let outgoing = self.outgoing.lock().await;
                json!({
                    "ok": true,
                    "device": hex(&self.identity.fingerprint()),
                    "hosts": self.hosting.is_some(),
                    "tunnel_port": self.tunnel_address().map(|a| a.port()),
                    "access_expires_at": self.hosting.as_ref().and_then(|h| h.access().expires_at()),
                    "allowed": self.hosting.as_ref().map(|h| h.access().allowed(now()).len()),
                    "connected_devices": self.hosting.as_ref().map(|h| h.tunnel().connected().iter().map(hex).collect::<Vec<_>>()),
                    "connected_to_host": outgoing.as_ref().is_some_and(|t| !t.is_closed()),
                })
            }
            Request::Enrolment { challenge, name } => {
                let Some(platform) = Platform::current() else { return failed("unsupported_platform") };
                match account::enrolment(&self.identity, &challenge, self.account.origin(), &name, platform, self.hosting.is_some()) {
                    Ok(proof) => json!({"ok": true, "proof": proof}),
                    Err(_) => failed("invalid_challenge"),
                }
            }
            Request::Refresh => {
                let Some(hosting) = &self.hosting else { return failed("not_hosting") };
                match hosting.refresh(&self.account, &self.identity).await {
                    Refreshed::Updated => json!({"ok": true, "result": "updated"}),
                    Refreshed::Removed => json!({"ok": true, "result": "removed"}),
                    Refreshed::Kept(account::Error::Unreachable) => json!({"ok": true, "result": "kept", "reason": "unreachable"}),
                    Refreshed::Kept(_) => json!({"ok": true, "result": "kept", "reason": "refused"}),
                }
            }
            Request::Accept { device, seconds } => {
                let (Some(hosting), Some(device)) = (&self.hosting, unhex(&device)) else { return failed("not_possible") };
                // At most a day at a time; the window asks again for longer visits.
                json!({"ok": hosting.accept(&device, now() + seconds.min(86_400)).await})
            }
            Request::Dismiss { device } => {
                let (Some(hosting), Some(device)) = (&self.hosting, unhex(&device)) else { return failed("not_possible") };
                hosting.dismiss(&device).await;
                json!({"ok": true})
            }
            Request::Connect { host, addresses } => {
                let Some(host) = unhex(&host) else { return failed("invalid_host") };
                if addresses.is_empty() || addresses.len() > 8 {
                    return failed("invalid_addresses");
                }
                let mut outgoing = self.outgoing.lock().await;
                // One computer at a time; the old tunnel's local ports must be free first.
                *outgoing = None;
                let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else { return failed("no_socket") };
                let Ok(listen) = "127.0.0.2".parse() else { return failed("no_socket") };
                match secpar_tunnel::connect_on(socket, &addresses, &self.identity, host, listen, Ports::engine(ENGINE_BASE)).await {
                    Ok(tunnel) => {
                        *outgoing = Some(tunnel);
                        json!({"ok": true, "stream_address": format!("127.0.0.2:{ENGINE_BASE}")})
                    }
                    Err(_) => failed("not_connected"),
                }
            }
            Request::Disconnect => {
                *self.outgoing.lock().await = None;
                json!({"ok": true})
            }
            Request::Pair { pairing } => {
                let outgoing = self.outgoing.lock().await;
                let Some(tunnel) = outgoing.as_ref() else { return failed("not_connected") };
                json!({"ok": true, "answer": request_pairing(tunnel, &pairing).await})
            }
        }
    }
}
