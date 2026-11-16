//! The hosting side of Secpar put together: the tunnel only lets in the devices
//! the account's access list allows, and the engine behind it only pairs with a
//! device that came in through the tunnel.
//!
//! The engine listens on this computer only. A device reaches it through the
//! tunnel, where its certificate has been checked against the list, and asks for
//! pairing over the tunnel's own channel. So the engine's pairing is always tied
//! to one device of one account, and is taken away again when that device leaves
//! the list.
use base64::Engine as _;
use secpar_core::{
    account::{Knock, Reach, Service},
    hosting::{Access, Peer, Refreshed},
    identity::{hex, unhex},
};
use secpar_tunnel::{Fingerprint, Identity};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

mod engine;
pub mod outside;
#[cfg(test)]
mod tests;

/// The engine's base port on every Secpar hosting computer. Chosen away from the
/// usual range of this engine family, so it runs beside an installed one.
pub const ENGINE_BASE: u16 = 46989;

/// How often the clipboard is looked at while a device is connected.
pub const CLIPBOARD_TICK: Duration = Duration::from_millis(500);

pub type Pending<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The stream component's own identity towards the engine, as it shows up in
/// the engine's pending pairings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pairing {
    pub client_id: String,
    pub certificate_sha256: String,
    pub pin: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The engine has not seen this pairing attempt yet. Worth trying again shortly.
    NotPending,
    /// This identity was taken away before. The device must make a new one.
    Revoked,
    Failed,
}
/// The part of the engine's supervisor that hosting needs.
pub trait Engine: Send + Sync + 'static {
    /// Completes a pairing for the device `device` of the account `owner`.
    fn approve<'a>(&'a self, owner: &'a str, device: &'a str, pairing: &'a Pairing, name: &'a str) -> Pending<'a, Result<(), Refusal>>;
    /// Takes away everything granted to this device. Returns whether it worked.
    fn revoke<'a>(&'a self, owner: &'a str, device: &'a str) -> Pending<'a, bool>;
    /// The `(owner, device)` pairs that hold a grant now.
    fn granted(&self) -> Pending<'_, Vec<(String, String)>>;
    /// Changes the size of the stream this computer is hosting. `Ok(false)`
    /// means no stream was running. A stand-in engine has no stream to change.
    fn resolution(&self, _width: u32, _height: u32) -> Pending<'_, Result<bool, Refusal>> {
        Box::pin(async { Err(Refusal::Failed) })
    }
}

#[derive(Deserialize)]
enum Request {
    #[serde(rename = "pair")]
    Pair(Pairing),
    /// Anything on the app channel: the clipboard now, files and monitors later.
    #[serde(rename = "app")]
    App(App),
}
/// What a device may ask for on the app channel.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum App {
    Clipboard(ClipboardRequest),
    File(FileRequest),
    Resolution(ResolutionRequest),
    Preview(PreviewRequest),
}
/// `{"width": 1280, "height": 720}`: the size the connected device wants the
/// running stream to have. The session survives; only the picture changes size.
#[derive(Deserialize)]
struct ResolutionRequest {
    width: u32,
    height: u32,
}
/// `{"max_width": 480}`: a still of this computer's screen, for the device's
/// list of computers. The width is the most it may have; the answer keeps the
/// screen's own shape.
#[derive(Deserialize)]
struct PreviewRequest {
    #[serde(default)]
    max_width: Option<u32>,
}
/// One clipboard request: `{"get": true}` for the text this computer holds, or
/// `{"set": "..."}` for text from the other computer.
#[derive(Deserialize)]
struct ClipboardRequest {
    #[serde(default)]
    get: bool,
    #[serde(default)]
    set: Option<String>,
}
/// One piece of a file the other computer is sending:
/// `{"name": "report.pdf", "size": 700000, "offset": 0, "data": "<base64>"}`.
/// The pieces arrive in order, each with the offset it belongs at.
#[derive(Deserialize)]
struct FileRequest {
    name: String,
    size: u64,
    offset: u64,
    data: String,
}

/// The most a file may be, in bytes. Anything larger is refused.
pub const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
/// How many of a file's bytes travel in one app channel frame.
pub const FILE_CHUNK: usize = 256 * 1024;
/// The longest base64 text one frame may carry, so a frame stays well inside
/// the channel's own limit.
pub const MAX_FILE_FRAME: usize = 512 * 1024;

/// Whether a name a sending computer announced can be written on this computer:
/// one name, no folders, no characters the file systems refuse, and nothing
/// that stands for a place rather than a file.
pub fn file_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && name.trim() == name
        && !name.ends_with('.')
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'))
}

/// What this computer holds of a file that is arriving.
pub struct Progress {
    /// The bytes stored so far, which the sending computer checks its offset against.
    pub received: u64,
    /// The name the file was saved under, once its last piece has arrived.
    pub saved: Option<String>,
}

/// Where files from another computer are written. The hosting side checks the
/// name, the size and the offsets; this decides where the bytes go.
pub trait Files: Send + Sync + 'static {
    /// Stores one piece of an arriving file from the device `from`. `None`
    /// refuses it, which the sending computer is told.
    fn receive(&self, from: &str, name: &str, total: u64, offset: u64, data: &[u8]) -> Option<Progress>;
}

/// Files that are refused, for a computer that takes none.
pub struct NoFiles;
impl Files for NoFiles {
    fn receive(&self, _: &str, _: &str, _: u64, _: u64, _: &[u8]) -> Option<Progress> {
        None
    }
}

/// This computer's clipboard, as the hosting side uses it. It is only called
/// while a device is connected and the person allows clipboard sharing, so an
/// implementation may open the OS clipboard when `attach` says a device arrived.
pub trait Clipboard: Send + Sync + 'static {
    /// The text worth sharing now: one this computer copied while a device was
    /// connected. Text that was already in the clipboard is never returned.
    fn text(&self) -> Option<String>;
    /// Puts text from another computer into this one's clipboard. False if it
    /// could not be done. Must not block: the tunnel's answer waits on it.
    fn set(&self, text: &str) -> bool;
    /// A device connected (`true`) or the last one left. Watching the OS
    /// clipboard may start or stop here.
    fn attach(&self, _connected: bool) {}
    /// Called about twice a second while a device is connected.
    fn poll(&self) {}
}

/// A clipboard that shares nothing, for a computer that does not use one.
pub struct NoClipboard;
impl Clipboard for NoClipboard {
    fn text(&self) -> Option<String> {
        None
    }
    fn set(&self, _: &str) -> bool {
        false
    }
}

/// The widest still a device may ask for. A wider request is answered at this
/// width, which keeps the answer well inside the app channel's frame.
pub const MAX_PREVIEW_WIDTH: u32 = 1024;
/// A request narrower than this is answered at this width.
pub const MIN_PREVIEW_WIDTH: u32 = 16;
/// The width answered when a device gives none.
pub const DEFAULT_PREVIEW_WIDTH: u32 = 480;
/// How long a still is kept, so repeated requests do not capture again.
pub const PREVIEW_TTL: Duration = Duration::from_secs(5);
/// The largest base64 picture an answer may carry. Four thirds of the picture
/// itself, so this leaves the JSON well under the channel's 1 MiB frame.
pub const MAX_PREVIEW_FRAME: usize = 512 * 1024;

/// The width a request is answered at: the one asked for, kept between
/// `MIN_PREVIEW_WIDTH` and `MAX_PREVIEW_WIDTH`, and `DEFAULT_PREVIEW_WIDTH` when
/// the request gives none.
pub fn preview_width(max_width: Option<u32>) -> u32 {
    max_width.filter(|width| *width > 0).unwrap_or(DEFAULT_PREVIEW_WIDTH).clamp(MIN_PREVIEW_WIDTH, MAX_PREVIEW_WIDTH)
}

/// One still of this computer's screen, as another computer's list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Still {
    /// The picture as JPEG bytes.
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// When it was taken, in seconds since the epoch.
    pub at: u64,
}

/// This computer's screen, for the owner's own devices. It is asked only when
/// one of them asks for a still over an open tunnel, and not again within
/// `PREVIEW_TTL`, so nothing is captured while no device is connected.
pub trait Previews: Send + Sync + 'static {
    /// The primary display at most `max_width` pixels wide. `None` when there is
    /// nothing to capture now: a locked screen, no display, or a system whose
    /// capture path Secpar does not use yet.
    fn still(&self, max_width: u32) -> Option<Still>;
}

/// A screen that cannot be shown, for a computer with no capture path.
pub struct NoPreviews;
impl Previews for NoPreviews {
    fn still(&self, _max_width: u32) -> Option<Still> {
        None
    }
}

/// What a device hears back on the tunnel's own channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Paired,
    /// The device should make a new stream identity and pair again.
    Revoked,
    NotAllowed,
    Failed,
    Unknown,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

struct Inner<E> {
    access: Arc<Access>,
    engine: Arc<E>,
    clipboard: Arc<dyn Clipboard>,
    files: Arc<dyn Files>,
    previews: Arc<dyn Previews>,
    /// The last still taken, and when, so a device that asks again soon after
    /// is given the same one instead of another capture.
    preview: std::sync::Mutex<Option<Cached>>,
}
/// A still that was taken for the width a device asked for.
struct Cached {
    width: u32,
    still: Still,
    taken: std::time::Instant,
}
impl<E: Engine> Inner<E> {
    /// The still for this request: the one taken less than `PREVIEW_TTL` ago at
    /// the same width, or a new capture. `None` when there is nothing to show.
    /// Only a connected device can ask, so no device connected means no capture.
    async fn preview(&self, max_width: Option<u32>) -> Option<Still> {
        let width = preview_width(max_width);
        if let Ok(cache) = self.preview.lock() {
            if let Some(cached) = cache.as_ref() {
                if cached.width == width && cached.taken.elapsed() < PREVIEW_TTL {
                    return Some(cached.still.clone());
                }
            }
        }
        let previews = self.previews.clone();
        // Capturing blocks the thread that takes the picture, never the runner.
        let still = tokio::task::spawn_blocking(move || previews.still(width)).await.ok().flatten()?;
        if let Ok(mut cache) = self.preview.lock() {
            *cache = Some(Cached { width, still: still.clone(), taken: std::time::Instant::now() });
        }
        Some(still)
    }

    async fn answer(&self, device: Fingerprint, request: Vec<u8>) -> Answer {
        let Ok(Request::Pair(pairing)) = serde_json::from_slice(&request) else { return Answer::Unknown };
        // The tunnel let this device in, but the list may have changed since.
        let (Some(peer), Some((owner, _))) = (self.access.peer(&device, now()), self.access.listed()) else {
            return Answer::NotAllowed;
        };
        // The name and the account come from the signed list, never from the device.
        let (account, name) = match peer {
            Peer::Own { name } => (owner, name),
            Peer::Guest { name, account, handle, .. } => (account, format!("{handle}: {name}")),
        };
        let device = hex(&device);
        // The device starts pairing with the engine and asks here at the same
        // time, so the engine may need a moment to show the attempt.
        for _ in 0..40 {
            match self.engine.approve(&account, &device, &pairing, &name).await {
                Ok(()) => return Answer::Paired,
                Err(Refusal::NotPending) => tokio::time::sleep(Duration::from_millis(250)).await,
                Err(Refusal::Revoked) => return Answer::Revoked,
                Err(Refusal::Failed) => return Answer::Failed,
            }
        }
        Answer::Failed
    }

    /// Answers one request on the app channel. Clipboard text is shared with the
    /// owner's own devices and friends who may use the keyboard; files are taken
    /// from the owner's own devices only.
    async fn app(&self, device: Fingerprint, request: Vec<u8>) -> serde_json::Value {
        let refused = |code: &str| serde_json::json!({"ok": false, "error": code});
        let Ok(Request::App(app)) = serde_json::from_slice(&request) else { return refused("unknown") };
        // The tunnel let this device in, but the list may have changed since.
        let Some(peer) = self.access.peer(&device, now()) else { return refused("not_allowed") };
        let (own, keyboard) = match peer {
            Peer::Own { .. } => (true, true),
            Peer::Guest { keyboard, .. } => (false, keyboard),
        };
        match app {
            App::Clipboard(ClipboardRequest { get, set }) => {
                if !keyboard {
                    return refused("not_allowed");
                }
                if let Some(text) = set {
                    if !secpar_clipboard::valid(&text) {
                        return refused("too_large");
                    }
                    return serde_json::json!({"ok": self.clipboard.set(&text)});
                }
                if get {
                    return serde_json::json!({"ok": true, "text": self.clipboard.text()});
                }
                refused("unknown")
            }
            App::File(file) => {
                // A friend's device is not sent files in this first cut.
                if !own {
                    return refused("not_allowed");
                }
                if !file_name_ok(&file.name) {
                    return refused("bad_name");
                }
                if file.size == 0 || file.size > MAX_FILE || file.data.len() > MAX_FILE_FRAME {
                    return refused("too_large");
                }
                let Ok(data) = base64::engine::general_purpose::STANDARD.decode(&file.data) else { return refused("bad_data") };
                if data.is_empty() || data.len() > FILE_CHUNK {
                    return refused("too_large");
                }
                if file.offset.checked_add(data.len() as u64).is_none_or(|end| end > file.size) {
                    return refused("too_large");
                }
                match self.files.receive(&hex(&device), &file.name, file.size, file.offset, &data) {
                    Some(Progress { received, saved: Some(saved) }) => serde_json::json!({"ok": true, "received": received, "saved": saved}),
                    Some(Progress { received, saved: None }) => serde_json::json!({"ok": true, "received": received}),
                    None => refused("not_saved"),
                }
            }
            App::Resolution(ResolutionRequest { width, height }) => {
                // The size of the picture on this computer is the owner's to
                // change; a friend watching is not given it.
                if !own {
                    return refused("not_allowed");
                }
                match self.engine.resolution(width, height).await {
                    Ok(streaming) => serde_json::json!({"ok": true, "streaming": streaming}),
                    Err(_) => refused("not_changed"),
                }
            }
            App::Preview(PreviewRequest { max_width }) => {
                // This computer's screen is shown to the owner's own devices
                // only, like files. A friend watching is not given a still.
                if !own {
                    return serde_json::json!({"ok": false, "err": "not_allowed"});
                }
                let Some(still) = self.preview(max_width).await else {
                    return serde_json::json!({"ok": false, "err": "not_available"});
                };
                let jpeg = base64::engine::general_purpose::STANDARD.encode(&still.jpeg);
                // A picture this large would not fit the channel's frame.
                if jpeg.len() > MAX_PREVIEW_FRAME || still.width > MAX_PREVIEW_WIDTH {
                    return serde_json::json!({"ok": false, "err": "not_available"});
                }
                serde_json::json!({"ok": true, "jpeg": jpeg, "width": still.width, "height": still.height, "at": still.at})
            }
        }
    }
}

/// A running hosting side. Dropping it closes the tunnel.
pub struct Hosting<E: Engine> {
    inner: Arc<Inner<E>>,
    tunnel: secpar_tunnel::Host,
    /// This network's address on the internet, once found, and whether the
    /// router agreed to pass the tunnel's port through.
    outside: std::sync::Mutex<(Option<IpAddr>, bool)>,
    /// Devices to keep sending small packets to, and until when, so the
    /// routers in between let their packets in.
    punching: std::sync::Mutex<Vec<(tokio::time::Instant, SocketAddr)>>,
}
impl<E: Engine> Hosting<E> {
    /// Opens the tunnel on `bind` for the devices `access` allows right now, in
    /// front of an engine that listens on `engine_ip` with `ports`, with
    /// `clipboard` as the text this computer shares, `files` as where files
    /// from another computer are written, and `previews` as this computer's
    /// screen.
    #[allow(clippy::too_many_arguments)]
    pub async fn start(bind: SocketAddr, identity: &Identity, access: Arc<Access>, engine: Arc<E>, clipboard: Arc<dyn Clipboard>, files: Arc<dyn Files>, previews: Arc<dyn Previews>, engine_ip: IpAddr, ports: secpar_tunnel::Ports) -> secpar_tunnel::Result<Self> {
        let allowed = secpar_tunnel::Allowed::new(access.allowed(now()));
        let tunnel = secpar_tunnel::serve(bind, identity, allowed, engine_ip, ports).await?;
        let inner = Arc::new(Inner { access, engine, clipboard, files, previews, preview: Default::default() });
        let answering = inner.clone();
        tunnel.on_request(Arc::new(move |device, request| {
            let answering = answering.clone();
            Box::pin(async move { serde_json::to_vec(&answering.answer(device, request).await).unwrap_or_default() })
        }));
        let sharing = inner.clone();
        tunnel.on_app(Arc::new(move |device, request| {
            let sharing = sharing.clone();
            Box::pin(async move { serde_json::to_vec(&sharing.app(device, request).await).unwrap_or_default() })
        }));
        Ok(Self { inner, tunnel, outside: Default::default(), punching: Default::default() })
    }
    pub fn tunnel(&self) -> &secpar_tunnel::Host {
        &self.tunnel
    }
    pub fn access(&self) -> &Access {
        &self.inner.access
    }

    /// Brings the tunnel and the engine in line with the list as it is now:
    /// closes the tunnels of devices that may no longer connect, and takes the
    /// engine pairing away from devices the list no longer names at all.
    ///
    /// A friend whose acceptance ran out only loses the tunnel. Their pairing
    /// stays, because the engine cannot be reached without the tunnel and they
    /// may be accepted again.
    pub async fn apply(&self) {
        self.tunnel.set_allowed(self.inner.access.allowed(now()));
        // Without a list nothing is known about who left it, so nothing is revoked.
        let Some((_, listed)) = self.inner.access.listed() else { return };
        let listed: Vec<String> = listed.iter().map(hex).collect();
        for (owner, device) in self.inner.engine.granted().await {
            if !listed.contains(&device) {
                self.inner.engine.revoke(&owner, &device).await;
            }
        }
    }

    /// Where other devices can reach this tunnel: its port, and the address this
    /// computer uses on its network (or the one address the tunnel is bound to).
    pub fn reach(&self) -> Option<Reach> {
        let bound = self.tunnel.local_addr().ok()?;
        let address = if bound.ip().is_unspecified() {
            // No packet is sent; this only asks the system which address it would use.
            let probe = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
            probe.connect("192.0.2.1:9").ok()?;
            probe.local_addr().ok()?.ip()
        } else {
            bound.ip()
        };
        let mut addresses = vec![address.to_string()];
        // The home address first: it is tried first and is the faster way.
        if let (Some(public), true) = (self.outside.lock().unwrap().0, bound.ip().is_unspecified()) {
            addresses.push(public.to_string());
        }
        Some(Reach { port: bound.port(), addresses })
    }

    /// Finds this network's address on the internet and asks the router to pass
    /// the tunnel's port through. Returns the address, if found, and whether the
    /// router agreed. Does nothing when the tunnel is bound to one address only.
    pub async fn reach_outside(&self) -> (Option<IpAddr>, bool) {
        let Ok(bound) = self.tunnel.local_addr() else { return (None, false) };
        if !bound.ip().is_unspecified() {
            return (None, false);
        }
        let local = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|probe| probe.connect("192.0.2.1:9").and_then(|_| probe.local_addr()));
        let opened = match local {
            Ok(SocketAddr::V4(local)) => outside::open_on_router(*local.ip(), bound.port()).await,
            _ => false,
        };
        let found = (outside::public_address().await, opened);
        *self.outside.lock().unwrap() = found;
        found
    }

    /// Fetches a new list with the device's own key and applies it.
    pub async fn refresh(&self, service: &Service, identity: &Identity) -> Refreshed {
        let reach = self.reach();
        let result = self.inner.access.refresh(service, identity, reach.as_ref()).await;
        if result == Refreshed::Removed {
            // This computer is no longer a hosting device of the account.
            self.tunnel.set_allowed([]);
            for (owner, device) in self.inner.engine.granted().await {
                self.inner.engine.revoke(&owner, &device).await;
            }
        } else {
            self.apply().await;
        }
        result
    }

    /// The owner accepted a friend who is asked about each time, until `until`.
    pub async fn accept(&self, device: &Fingerprint, until: u64) -> bool {
        let accepted = self.inner.access.accept(device, until);
        self.apply().await;
        accepted
    }
    /// The owner ended a friend's visit.
    pub async fn dismiss(&self, device: &Fingerprint) {
        self.inner.access.dismiss(device);
        self.apply().await;
    }

    /// Keeps everything current: applies time limits every 20 seconds and asks
    /// the account service for a new list every 15 minutes. Never returns.
    ///
    /// It also asks for a new list when a device that is not allowed tries to
    /// connect, at most once in ten seconds: a computer the owner has just
    /// signed in on is not on the old list yet, and should not have to wait.
    pub async fn keep_current(&self, service: &Service, identity: &Identity) {
        tokio::join!(self.keep_list(service, identity), self.meet(service, identity), self.punch(), self.keep_clipboard());
    }

    /// Watches this computer's clipboard while a device is connected, so the
    /// clipboard can be answered with text this computer copies now. Nothing is
    /// read while no device is connected. Never returns.
    pub async fn keep_clipboard(&self) {
        let mut watching = false;
        loop {
            tokio::time::sleep(CLIPBOARD_TICK).await;
            if self.tunnel.connected().is_empty() {
                if watching {
                    watching = false;
                    self.inner.clipboard.attach(false);
                }
                continue;
            }
            watching = true;
            // Starts watching again if sharing was switched off and on meanwhile.
            self.inner.clipboard.attach(true);
            self.inner.clipboard.poll();
        }
    }

    async fn punch(&self) {
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let to: Vec<SocketAddr> = {
                let mut punching = self.punching.lock().unwrap();
                punching.retain(|(until, _)| *until > tokio::time::Instant::now());
                punching.iter().map(|(_, to)| *to).collect()
            };
            self.tunnel.punch(&to);
        }
    }

    /// Waits for devices that cannot reach this computer directly. Each says
    /// where it is on the internet and, if it has one, its relay address; this
    /// computer answers with its own two, sends packets towards the device so
    /// the routers let it in, and adds the device at the relay. Only devices on
    /// the access list get an answer, and what then arrives is the tunnel, which
    /// checks them again. The relay address is given back ten minutes after the
    /// last use. In both lists the first entry is the direct address
    /// (0.0.0.0:9 when unknown) and the second, if any, the relay address.
    async fn meet(&self, service: &Service, identity: &Identity) {
        let relay = self.tunnel.relay();
        let mut answers: Vec<Knock> = Vec::new();
        let mut used = tokio::time::Instant::now();
        let mut kept = used;
        loop {
            let knocks = match service.device_knocks(identity, &answers).await {
                Ok(knocks) => knocks,
                Err(_) => {
                    answers.clear();
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
            };
            answers.clear();
            for knock in knocks {
                let peers: Vec<SocketAddr> = knock.candidates.iter().filter_map(|c| c.parse().ok()).collect();
                if !unhex(&knock.device).is_some_and(|device| self.inner.access.allowed(now()).contains(&device)) {
                    // Perhaps a device the list does not have yet.
                    self.refresh(service, identity).await;
                    if !unhex(&knock.device).is_some_and(|device| self.inner.access.allowed(now()).contains(&device)) {
                        continue;
                    }
                }
                let Some(direct) = peers.first().copied() else { continue };
                // Direct: where this socket is seen from outside, and packets towards the device.
                let seen = match tokio::net::lookup_host(outside::STUN).await.ok().and_then(|mut found| found.find(SocketAddr::is_ipv4)) {
                    Some(stun) => {
                        let asking = relay.clone();
                        tokio::task::spawn_blocking(move || asking.seen_from_outside(stun)).await.ok().flatten()
                    }
                    None => None,
                };
                if !direct.ip().is_unspecified() {
                    self.punching.lock().unwrap().push((tokio::time::Instant::now() + Duration::from_secs(12), direct));
                }
                // The relay, when the device has a relay address and this service offers one.
                if let Some(there) = peers.get(1).copied() {
                    if relay.address().is_none() {
                        if let Ok(access) = service.device_relay(identity).await {
                            if let Some(server) = tokio::net::lookup_host(&access.server).await.ok().and_then(|mut found| found.find(SocketAddr::is_ipv4)) {
                                let starting = relay.clone();
                                let _ = tokio::task::spawn_blocking(move || starting.start(server, access.username, access.credential)).await;
                                kept = tokio::time::Instant::now();
                            }
                        }
                    }
                    if relay.address().is_some() {
                        let adding = relay.clone();
                        let _ = tokio::task::spawn_blocking(move || adding.add(there)).await;
                    }
                }
                let mut candidates = vec![seen.map_or("0.0.0.0:9".to_string(), |a| a.to_string())];
                candidates.extend(relay.address().filter(|_| peers.len() > 1).map(|a| a.to_string()));
                if seen.is_none() && candidates.len() < 2 {
                    continue;
                }
                answers.push(Knock { device: knock.device, candidates });
                used = tokio::time::Instant::now();
            }
            if relay.address().is_some() {
                if !self.tunnel.connected().is_empty() {
                    used = tokio::time::Instant::now();
                }
                if used.elapsed() > Duration::from_secs(600) {
                    relay.stop();
                } else if kept.elapsed() > Duration::from_secs(200) {
                    kept = tokio::time::Instant::now();
                    let relay = relay.clone();
                    let _ = tokio::task::spawn_blocking(move || relay.keep()).await;
                }
            }
        }
    }

    async fn keep_list(&self, service: &Service, identity: &Identity) {
        let mut tick = 0u32;
        let mut knocked = tokio::time::Instant::now() - Duration::from_secs(10);
        loop {
            if tick % 45 == 0 {
                // Every 15 minutes, so the router's hour never runs out.
                self.reach_outside().await;
                self.refresh(service, identity).await;
            } else {
                self.apply().await;
            }
            tick = tick.wrapping_add(1);
            let next = tokio::time::sleep(Duration::from_secs(20));
            tokio::pin!(next);
            loop {
                tokio::select! {
                    _ = &mut next => break,
                    _ = self.tunnel.turned_away() => {
                        if knocked.elapsed() >= Duration::from_secs(10) {
                            knocked = tokio::time::Instant::now();
                            self.refresh(service, identity).await;
                        }
                    }
                }
            }
        }
    }
}

/// The connecting side of the same exchange: asks the host, through the tunnel,
/// to complete the pairing its stream component has just started.
pub async fn request_pairing(tunnel: &secpar_tunnel::Client, pairing: &Pairing) -> Answer {
    let Ok(request) = serde_json::to_vec(&serde_json::json!({ "pair": pairing })) else { return Answer::Failed };
    match tunnel.request(&request).await {
        Ok(answer) => serde_json::from_slice(&answer).unwrap_or(Answer::Failed),
        Err(_) => Answer::Failed,
    }
}

/// The text in the other computer's clipboard, if it has copied anything since
/// this computer connected. `None` when it has nothing to share yet, or did not
/// answer. Text that was already there before connecting is never returned.
pub async fn clipboard_text(tunnel: &secpar_tunnel::Client) -> Option<String> {
    let request = serde_json::to_vec(&serde_json::json!({ "app": { "clipboard": { "get": true } } })).ok()?;
    let answer: serde_json::Value = serde_json::from_slice(&tunnel.app_request(&request).await.ok()?).ok()?;
    // A device that may not share gets no text key at all.
    Some(answer.get("text")?.as_str()?.to_string())
}

/// Puts text in the other computer's clipboard. False when it refused, for
/// example because this device may not use the keyboard, the text is too large,
/// or the other computer could not write it.
pub async fn set_clipboard(tunnel: &secpar_tunnel::Client, text: &str) -> bool {
    let Ok(request) = serde_json::to_vec(&serde_json::json!({ "app": { "clipboard": { "set": text } } })) else { return false };
    match tunnel.app_request(&request).await {
        Ok(answer) => serde_json::from_slice::<serde_json::Value>(&answer).is_ok_and(|answer| answer["ok"] == true),
        Err(_) => false,
    }
}

/// Asks the computer this one is connected to for a different stream size. The
/// stream keeps running; the picture changes size and the decoder reconfigures
/// from the next sequence header. `Ok(true)` means it is changing, `Ok(false)`
/// that no stream is running there, and `Err` why it refused.
pub async fn set_resolution(tunnel: &secpar_tunnel::Client, width: u32, height: u32) -> Result<bool, &'static str> {
    let request = serde_json::json!({ "app": { "resolution": { "width": width, "height": height } } });
    let request = serde_json::to_vec(&request).map_err(|_| "unknown")?;
    let answer = tunnel.app_request(&request).await.map_err(|_| "no_answer")?;
    let answer: serde_json::Value = serde_json::from_slice(&answer).map_err(|_| "unknown")?;
    if answer["ok"] != true {
        return Err(match answer["error"].as_str() {
            Some("not_allowed") => "not_allowed",
            Some("not_changed") => "not_changed",
            _ => "unknown",
        });
    }
    Ok(answer["streaming"] == true)
}

/// The still of the other computer's screen, for a tile in this computer's list
/// of computers. `None` when it has none to give now (a locked screen, no
/// capture path there, no answer) or when this device may not see it.
pub async fn preview(tunnel: &secpar_tunnel::Client, max_width: u32) -> Option<Still> {
    let request = serde_json::json!({ "app": { "preview": { "max_width": max_width } } });
    let request = serde_json::to_vec(&request).ok()?;
    let answer = tunnel.app_request(&request).await.ok()?;
    let answer: serde_json::Value = serde_json::from_slice(&answer).ok()?;
    if answer["ok"] != true {
        return None;
    }
    let jpeg = base64::engine::general_purpose::STANDARD.decode(answer["jpeg"].as_str()?).ok()?;
    if jpeg.len() < 4 || jpeg[..2] != [0xFF, 0xD8] || jpeg[jpeg.len() - 2..] != [0xFF, 0xD9] {
        return None;
    }
    Some(Still {
        jpeg,
        width: u32::try_from(answer["width"].as_u64()?).ok()?,
        height: u32::try_from(answer["height"].as_u64()?).ok()?,
        at: answer["at"].as_u64().unwrap_or(0),
    })
}

/// What the other computer answered about one piece of a file.
pub struct Sent {
    /// The bytes it holds of the file after this piece, or `None` when it
    /// refused the piece. The refusal code says why.
    pub received: Option<u64>,
    /// The name it saved the file under, once the last piece arrived.
    pub saved: Option<String>,
    /// Why the piece was refused, in the words the hosting side uses.
    pub error: Option<String>,
}
impl Sent {
    fn refused(code: &str) -> Self {
        Self { received: None, saved: None, error: Some(code.into()) }
    }
}

/// Sends one piece of a file to the other computer, which answers with how much
/// of the file it holds (`{"ok": true, "received": N}`). Only the owner's own
/// devices are given files; a friend's device answers `not_allowed`.
pub async fn send_file_chunk(tunnel: &secpar_tunnel::Client, name: &str, size: u64, offset: u64, data: &[u8]) -> Sent {
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    let request = serde_json::json!({ "app": { "file": { "name": name, "size": size, "offset": offset, "data": encoded } } });
    let Ok(request) = serde_json::to_vec(&request) else { return Sent::refused("unknown") };
    match tunnel.app_request(&request).await {
        Ok(answer) => match serde_json::from_slice::<serde_json::Value>(&answer) {
            Ok(answer) if answer["ok"] == true => match answer["received"].as_u64() {
                Some(received) => Sent { received: Some(received), saved: answer["saved"].as_str().map(str::to_owned), error: None },
                None => Sent::refused("unknown"),
            },
            Ok(answer) => Sent::refused(answer["error"].as_str().unwrap_or("unknown")),
            Err(_) => Sent::refused("unknown"),
        },
        Err(_) => Sent::refused("no_answer"),
    }
}
