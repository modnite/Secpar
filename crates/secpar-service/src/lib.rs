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
use secpar_hosting::{request_pairing, Engine, Hosting, Pairing, Still, ENGINE_BASE, PREVIEW_TTL};
use secpar_tunnel::{Fingerprint, Identity, Ports};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub mod channel;
pub mod clipboard;
pub mod files;
pub mod preview;

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
    /// Ask the computer this one is connected to for a different stream size.
    Resolution { width: u32, height: u32 },
    /// Share this computer's clipboard text with the computer it is connected to.
    Clipboard { on: bool },
    /// Send the file at this path to the computer it is connected to.
    SendFile { path: String },
    /// How far the files this computer sends have got, and the last one that
    /// arrived here.
    Transfers,
    /// A still of one of the account's computers, for its tile. `host` is that
    /// computer's certificate fingerprint and `addresses` where it can be
    /// reached. An empty list means the account lists it as offline, and an
    /// offline computer is never asked.
    Preview {
        host: String,
        max_width: u32,
        #[serde(default)]
        addresses: Vec<String>,
    },
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
/// How long one peek may take before it is given up, so a computer that does not
/// answer cannot hold the queue behind it.
const PEEK_WAIT: std::time::Duration = std::time::Duration::from_secs(8);
fn failed(code: &str) -> Value {
    json!({"ok": false, "error": code})
}

pub struct Core<E: Engine> {
    identity: Identity,
    account: account::Service,
    /// Present when this computer hosts.
    hosting: Option<Arc<Hosting<E>>>,
    /// This computer's clipboard, shared with the connected computer while
    /// there is one.
    clipboard: Arc<clipboard::Shared>,
    /// Where files from other computers are written.
    files: Arc<files::Downloads>,
    /// Files this computer sends, and how far they have got.
    transfers: Arc<files::Transfers>,
    /// The tunnel to the computer this one is connected to, if any.
    outgoing: tokio::sync::Mutex<Option<Arc<secpar_tunnel::Client>>>,
    /// One peek at a time, so a page of tiles queues here instead of opening a
    /// tunnel for each computer at once.
    peeking: tokio::sync::Mutex<()>,
    /// When each computer was last asked for a still, so the same one is not
    /// asked again while its picture is still fresh.
    peeked: std::sync::Mutex<std::collections::HashMap<Fingerprint, std::time::Instant>>,
}
impl<E: Engine> Core<E> {
    /// Starts with the device's identity. With an engine this computer hosts:
    /// the tunnel opens for whoever the saved access list still allows.
    pub async fn start(settings: Settings, identity: Identity, engine: Option<Arc<E>>) -> Result<Arc<Self>, &'static str> {
        std::fs::create_dir_all(&settings.state).map_err(|_| "The state folder cannot be used")?;
        let clipboard = clipboard::Shared::start(true);
        let files = files::Downloads::open();
        let hosting = match engine {
            None => None,
            Some(engine) => {
                let access = Arc::new(Access::open(settings.account.origin(), identity.fingerprint(), settings.state.join("access.json"), now()));
                let loopback = "127.0.0.1".parse().map_err(|_| "No loopback address")?;
                let hosting = Hosting::start(settings.bind, &identity, access, engine, clipboard.clone(), files.clone(), Arc::new(preview::Screen), loopback, Ports::engine(ENGINE_BASE)).await.map_err(|e| e.0)?;
                Some(Arc::new(hosting))
            }
        };
        Ok(Arc::new(Self {
            identity,
            account: settings.account,
            hosting,
            clipboard,
            files,
            transfers: Default::default(),
            outgoing: tokio::sync::Mutex::new(None),
            peeking: tokio::sync::Mutex::new(()),
            peeked: Default::default(),
        }))
    }
    /// The tunnel's address when this computer hosts.
    pub fn tunnel_address(&self) -> Option<SocketAddr> {
        self.hosting.as_ref().and_then(|h| h.tunnel().local_addr().ok())
    }
    /// For the log: whether this network's internet address was found and
    /// whether the router passes the tunnel's port through.
    pub async fn reach_outside(&self) -> Option<(bool, bool)> {
        let hosting = self.hosting.as_ref()?;
        let (address, opened) = hosting.reach_outside().await;
        Some((address.is_some(), opened))
    }
    /// Keeps the access list and the tunnel current. Returns at once when this
    /// computer does not host; otherwise never.
    pub async fn keep_current(&self) {
        if let Some(hosting) = &self.hosting {
            hosting.keep_current(&self.account, &self.identity).await;
        }
    }

    /// Opens a tunnel to `host` at `addresses` for one question and drops it
    /// again. No engine port is taken and no local socket is kept, so a stream's
    /// own tunnel keeps its addresses while a still is fetched.
    ///
    /// One peek runs at a time, and the same computer is not asked again while
    /// its picture is still fresh: a page of tiles queues here rather than
    /// opening a tunnel per tile at once.
    async fn peek(&self, host: Fingerprint, addresses: &[SocketAddr], max_width: u32) -> Option<Still> {
        let _one_at_a_time = self.peeking.lock().await;
        {
            let mut asked = self.peeked.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if asked.get(&host).is_some_and(|last| last.elapsed() < PREVIEW_TTL) {
                return None;
            }
            asked.insert(host, std::time::Instant::now());
        }
        let listen = "127.0.0.2".parse().ok()?;
        let asking = async {
            let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
            // No engine ports: nothing here can collide with a stream.
            let none = Ports { tcp: Vec::new(), udp: Vec::new() };
            let tunnel = secpar_tunnel::connect_on(socket, addresses, &self.identity, host, listen, none).await.ok()?;
            secpar_hosting::preview(&tunnel, max_width).await
        };
        // A computer that does not answer must not hold the queue for long.
        tokio::time::timeout(PEEK_WAIT, asking).await.ok().flatten()
    }

    /// Answers a still request with the picture from the tunnel it came on.
    fn still_answer(still: Still) -> Value {
        json!({
            "ok": true,
            "jpeg": base64::engine::general_purpose::STANDARD.encode(&still.jpeg),
            "width": still.width,
            "height": still.height,
            "at": still.at,
        })
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
                        let tunnel = Arc::new(tunnel);
                        *outgoing = Some(tunnel.clone());
                        // The clipboard follows this tunnel until it is let go.
                        tokio::spawn(clipboard::follow(self.clipboard.clone(), Arc::downgrade(&tunnel)));
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
            Request::Resolution { width, height } => {
                let outgoing = self.outgoing.lock().await;
                let Some(tunnel) = outgoing.as_ref() else { return failed("not_connected") };
                match secpar_hosting::set_resolution(tunnel, width, height).await {
                    Ok(streaming) => json!({"ok": true, "streaming": streaming}),
                    Err(code) => failed(code),
                }
            }
            Request::Clipboard { on } => {
                self.clipboard.set_sharing(on);
                json!({"ok": true, "sharing": on})
            }
            Request::SendFile { path } => {
                let outgoing = self.outgoing.lock().await.clone();
                let Some(tunnel) = outgoing else { return failed("not_connected") };
                match files::send(self.transfers.clone(), tunnel, PathBuf::from(path)) {
                    Ok(id) => {
                        let transfer = self.transfers.list().into_iter().find(|t| t.id == id);
                        json!({"ok": true, "transfer": transfer})
                    }
                    Err(code) => failed(code),
                }
            }
            Request::Transfers => json!({
                "ok": true,
                "transfers": self.transfers.list(),
                "received": self.files.last(),
            }),
            Request::Preview { host, max_width, addresses } => {
                let unavailable = json!({"ok": false, "err": "not_available"});
                let Some(host) = unhex(&host) else { return unavailable };
                // A computer this window is already connected to answers on the
                // tunnel that is open, without a second one being opened.
                let open = {
                    let outgoing = self.outgoing.lock().await;
                    outgoing.as_ref().filter(|tunnel| tunnel.host() == host && !tunnel.is_closed()).cloned()
                };
                if let Some(tunnel) = open {
                    return match secpar_hosting::preview(&tunnel, max_width).await {
                        Some(still) => Self::still_answer(still),
                        None => unavailable,
                    };
                }
                // No addresses means the account lists it as offline, and an
                // offline computer is never asked.
                let addresses: Vec<SocketAddr> = addresses.iter().filter_map(|address| address.parse().ok()).collect();
                if addresses.is_empty() {
                    return unavailable;
                }
                match self.peek(host, &addresses, max_width).await {
                    Some(still) => Self::still_answer(still),
                    None => unavailable,
                }
            }
        }
    }
}
