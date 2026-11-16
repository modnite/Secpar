//! Secpar's own part of the Android app: the device identity, the account, the
//! list of computers and the tunnel. The app passes one JSON request and gets
//! one JSON answer back (`win.secpar.app.Core.call`); each call blocks, so the
//! app makes it off its main thread.
//!
//! Android connects only. The identity and the session are kept in the app's
//! private folder, which other apps cannot read.
mod tunnel;

use jni::{
    objects::{JClass, JString},
    sys::jstring,
    JNIEnv,
};
use secpar_core::{
    account::{self, Device, Error, FriendChange, Platform, Service, Session},
    identity,
    store::FileStore,
};
use secpar_tunnel::{Identity, Ports};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// The engine's base port on every Secpar hosting computer (secpar_hosting::ENGINE_BASE).
const ENGINE_BASE: u16 = 46989;
/// Nothing else in the app listens here, so the tunnel's local end is plain loopback.
const LOCAL: &str = "127.0.0.1";
/// The widest still a card asks for unless it says otherwise. A card shows it at
/// about 64 dp, so this is already more than one screen needs.
const PREVIEW_WIDTH: u32 = 320;
/// How long one still may take. The direct attempt gives up quickly and the
/// meeting path is what a phone away from home needs, so this is long enough for
/// that and no longer: a card that gets nothing keeps its glyph.
const PREVIEW_WAIT: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Request {
    /// First call. `folder` is the app's private files folder.
    Configure {
        folder: String,
        #[serde(default)]
        test_account: Option<(String, String)>,
    },
    State,
    /// Signs in and adds this phone or tablet to the account as `name`.
    SignIn { email: String, password: String, name: String },
    SignOut,
    Devices,
    /// Takes another device off the account, such as an old record of this phone.
    RemoveDevice { id: String },
    /// Everyone on the account's friend list, including waiting requests.
    Friends,
    /// Asks the account named `handle` to be friends.
    AddFriend { handle: String },
    /// Accepts a request from the account `id`.
    AcceptFriend { id: String },
    /// Removes a friend, or turns down a request.
    RemoveFriend { id: String },
    /// Changes what the account `id` may do on this account's computers. Parts
    /// that are left out stay as they are; the answer carries the saved values.
    FriendSettings {
        id: String,
        #[serde(default)]
        connect: Option<String>,
        #[serde(default)]
        input: Option<String>,
        #[serde(default)]
        keyboard: Option<bool>,
        #[serde(default)]
        mouse: Option<bool>,
    },
    /// Opens the tunnel to one of the account's hosting computers.
    Connect { id: String },
    /// A still of one of the account's hosting computers for its card on the
    /// Computers tab, at most `max_width` pixels wide. It answers the hosting
    /// computer's own answer: `{"ok":true,"jpeg":…,"width":…,"height":…,"at":…}`
    /// or `{"ok":false,"err":"not_available"}`.
    Preview { id: String, #[serde(default)] max_width: Option<u32> },
    /// Has the connected computer complete the pairing the app has just started.
    Pair { client_id: String, certificate_sha256: String, pin: String },
    Disconnect,
}

struct App {
    runtime: tokio::runtime::Runtime,
    account: Service,
    store: FileStore,
    folder: PathBuf,
    tunnel: tokio::sync::Mutex<Option<secpar_tunnel::Client>>,
}
static APP: OnceLock<App> = OnceLock::new();
static TURN: Mutex<()> = Mutex::new(());

fn failed(code: &str) -> Value {
    json!({"ok": false, "error": code})
}
/// What a card is told when there is no still to show, in the same shape the
/// hosting computer itself uses.
fn no_preview() -> Value {
    json!({"ok": false, "err": "not_available"})
}
fn refused(error: Error) -> Value {
    match error {
        Error::Unreachable => failed("unreachable"),
        Error::Refused { code, message, .. } => json!({"ok": false, "error": code, "message": message}),
        Error::Invalid(_) => failed("invalid_answer"),
        Error::Store(_) => failed("store_unavailable"),
    }
}
/// A friend action's answer: a mistake caught before sending keeps its own wording.
fn rejected(error: Error) -> Value {
    match error {
        Error::Invalid(message) => json!({"ok": false, "error": "invalid", "message": message}),
        other => refused(other),
    }
}

impl App {
    /// The device identity. A lost key is reported, never replaced silently.
    fn identity(&self) -> Result<Identity, Value> {
        identity::device(&self.store, &self.folder.join("device.txt")).map_err(|_| failed("identity_unavailable"))
    }
    fn session(&self) -> Result<Session, Value> {
        match account::saved_session(&self.store) {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err(failed("signed_out")),
            Err(error) => Err(refused(error)),
        }
    }
    fn gone(&self, error: Error) -> Value {
        if matches!(error, Error::Refused { status: 401, .. }) {
            let _ = account::forget_session(&self.store);
            return failed("signed_out");
        }
        refused(error)
    }
    /// The same, for a friend action: an invalid value keeps its own wording.
    fn friend_gone(&self, error: Error) -> Value {
        if matches!(error, Error::Refused { status: 401, .. }) {
            let _ = account::forget_session(&self.store);
            return failed("signed_out");
        }
        rejected(error)
    }

    async fn state(&self) -> Value {
        let session = match account::saved_session(&self.store) {
            Ok(session) => session,
            Err(error) => return refused(error),
        };
        json!({
            "ok": true,
            "signed_in": session.is_some(),
            "handle": session.map(|s| s.user.handle),
            "connected": self.tunnel.lock().await.as_ref().is_some_and(|t| !t.is_closed()),
        })
    }

    async fn sign_in(&self, email: &str, password: &str, name: &str) -> Value {
        let identity = match self.identity() {
            Ok(identity) => identity,
            Err(answer) => return answer,
        };
        let session = match self.account.sign_in(email, password).await {
            Ok(session) => session,
            Err(error) => return refused(error),
        };
        if let Err(error) = account::save_session(&self.store, &session) {
            return refused(error);
        }
        let enrolled = self.account.enrol(&session, &identity, name, Platform::Android, false).await;
        let mut state = self.state().await;
        state["enrolled"] = json!(enrolled.is_ok());
        state
    }

    async fn sign_out(&self) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        *self.tunnel.lock().await = None;
        // Take this device off the account, then forget the session.
        if let (Ok(identity), Ok(devices)) = (self.identity(), self.account.devices(&session).await) {
            let mine = identity::hex(&identity.fingerprint());
            if let Some(device) = devices.iter().find(|d| d.fingerprint == mine) {
                let _ = self.account.remove(&session, &device.id).await;
            }
        }
        match account::forget_session(&self.store) {
            Ok(()) => json!({"ok": true}),
            Err(error) => refused(error),
        }
    }

    async fn remove_device(&self, id: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        // Never this phone itself: signing out does that.
        let mine = self.identity().ok().map(|i| identity::hex(&i.fingerprint()));
        if let (Some(mine), Ok(devices)) = (&mine, self.account.devices(&session).await) {
            if devices.iter().any(|d| d.id == id && &d.fingerprint == mine) {
                return failed("this_device");
            }
        }
        match self.account.remove(&session, id).await {
            Ok(()) => self.devices().await,
            Err(error) => self.gone(error),
        }
    }

    async fn devices(&self) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        let mine = self.identity().ok().map(|i| identity::hex(&i.fingerprint()));
        match self.account.devices(&session).await {
            Ok(devices) => json!({
                "ok": true,
                "devices": devices.iter().map(|d| json!({
                    "id": d.id, "name": d.name, "platform": d.platform, "hosts": d.hosts, "online": d.online,
                    "fingerprint": d.fingerprint, "this": mine.as_deref() == Some(d.fingerprint.as_str()),
                })).collect::<Vec<_>>(),
            }),
            Err(error) => self.gone(error),
        }
    }

    async fn friends(&self) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.friends(&session).await {
            Ok(friends) => json!({
                "ok": true,
                "friends": friends.iter().map(|f| json!({
                    "id": f.id, "handle": f.handle, "name": f.name, "state": f.state,
                    // A request this account sent waits for the other side to answer.
                    "incoming": f.state == "pending" && f.requester != session.user.id,
                    "input": f.input, "connect": f.connect,
                    "keyboard": f.keyboard, "mouse": f.mouse,
                })).collect::<Vec<_>>(),
            }),
            Err(error) => self.gone(error),
        }
    }

    async fn add_friend(&self, handle: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.request_friend(&session, handle).await {
            Ok(()) => json!({"ok": true}),
            Err(error) => self.friend_gone(error),
        }
    }

    async fn accept_friend(&self, id: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.accept_friend(&session, id).await {
            Ok(()) => json!({"ok": true}),
            Err(error) => self.friend_gone(error),
        }
    }

    async fn remove_friend(&self, id: &str) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.remove_friend(&session, id).await {
            Ok(()) => json!({"ok": true}),
            Err(error) => self.friend_gone(error),
        }
    }

    async fn save_friend_settings(&self, id: &str, change: FriendChange) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        match self.account.save_friend_settings(&session, id, &change).await {
            Ok(settings) => json!({
                "ok": true,
                "settings": {
                    "input": settings.input, "connect": settings.connect,
                    "keyboard": settings.keyboard, "mouse": settings.mouse,
                },
            }),
            Err(error) => self.friend_gone(error),
        }
    }

    /// Opens a tunnel to `device` that carries `ports`, direct first and then by
    /// meeting it across the internet. Nothing is kept: the caller decides
    /// whether the tunnel becomes the stream's or is used once and dropped.
    async fn reach(&self, session: &Session, identity: &Identity, device: &Device, ports: Ports) -> Result<secpar_tunnel::Client, &'static str> {
        let (Some(reach), Some(host)) = (&device.reach, identity::unhex(&device.fingerprint)) else { return Err("offline") };
        let addresses: Vec<SocketAddr> = reach.addresses.iter().filter_map(|a| a.parse().ok()).map(|ip| SocketAddr::new(ip, reach.port)).collect();
        if addresses.is_empty() {
            return Err("offline");
        }
        let Ok(listen) = LOCAL.parse() else { return Err("no_socket") };
        // A computer that has not heard of this device yet fetches a new list
        // when it is refused, so a second and third try are worth making.
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else { return Err("no_socket") };
            let direct = secpar_tunnel::connect_on(socket, &addresses, identity, host, listen, ports.clone());
            match tokio::time::timeout(Duration::from_millis(1500), direct).await {
                Ok(Ok(opened)) => return Ok(opened),
                // Answered and refused: worth another try. No answer at all:
                // not on the same network, so go through the relay.
                Ok(Err(_)) => continue,
                Err(_) => break,
            }
        }

        // Meet the computer across the internet: tell it where this device is
        // seen from outside, and its relay address if the service offers a relay;
        // it answers with its own. Direct first, the relay as the fallback.
        let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else { return Err("no_socket") };
        let Ok(opened) = secpar_tunnel::open(socket) else { return Err("no_socket") };
        let relay = opened.relay();
        let first = |name: String| async move { tokio::net::lookup_host(name).await.ok().and_then(|mut found| found.find(SocketAddr::is_ipv4)) };
        let seen = match first("stun.cloudflare.com:3478".into()).await {
            Some(stun) => {
                let asking = relay.clone();
                tokio::task::spawn_blocking(move || asking.seen_from_outside(stun)).await.ok().flatten()
            }
            None => None,
        };
        let mut own_relay = None;
        if let Ok(access) = self.account.relay(session).await {
            if let Some(server) = first(access.server.clone()).await {
                let starting = relay.clone();
                own_relay = tokio::task::spawn_blocking(move || starting.start(server, access.username, access.credential)).await.ok().flatten();
            }
        }
        if seen.is_none() && own_relay.is_none() {
            return Err("no_internet_path");
        }
        let Ok(nowhere) = "0.0.0.0:9".parse::<SocketAddr>() else { return Err("no_socket") };
        let mut here = vec![seen.unwrap_or(nowhere)];
        here.extend(own_relay);
        let there = match self.account.knock(session, &device.id, &identity.fingerprint(), &here).await {
            Ok(there) if !there.is_empty() => there,
            _ => return Err("computer_not_answering"),
        };
        let mut candidates: Vec<SocketAddr> = there.first().copied().filter(|a| !a.ip().is_unspecified()).into_iter().collect();
        if let (Some(_), Some(host_relay)) = (own_relay, there.get(1).copied()) {
            let adding = relay.clone();
            let _ = tokio::task::spawn_blocking(move || adding.add(host_relay)).await;
            candidates.push(host_relay);
        }
        if candidates.is_empty() {
            return Err("no_internet_path");
        }
        secpar_tunnel::connect_through(opened, &candidates, identity, host, listen, ports).await.map_err(|_| "not_reachable")
    }

    async fn connect(&self, id: &str) -> Value {
        let (session, identity) = match (self.session(), self.identity()) {
            (Ok(session), Ok(identity)) => (session, identity),
            (Err(answer), _) | (_, Err(answer)) => return answer,
        };
        let devices = match self.account.devices(&session).await {
            Ok(devices) => devices,
            Err(error) => return self.gone(error),
        };
        let Some(device) = devices.iter().find(|d| d.id == id) else { return failed("device_not_found") };
        let mut tunnel = self.tunnel.lock().await;
        *tunnel = None;
        match self.reach(&session, &identity, device, Ports::engine(ENGINE_BASE)).await {
            Ok(opened) => {
                *tunnel = Some(opened);
                json!({"ok": true, "address": LOCAL, "port": ENGINE_BASE, "host": device.fingerprint, "name": device.name})
            }
            Err(code) => failed(code),
        }
    }

    /// A still of one of the account's own computers, for its card. The request
    /// travels on the app channel of a tunnel opened for this one question, so a
    /// preview never takes the port a stream needs and never disturbs the stream's
    /// own tunnel. Whatever the hosting computer answers is passed on unchanged.
    async fn preview(&self, id: &str, max_width: Option<u32>) -> Value {
        let session = match self.session() {
            Ok(session) => session,
            Err(answer) => return answer,
        };
        let Ok(identity) = self.identity() else { return no_preview() };
        let devices = match self.account.devices(&session).await {
            Ok(devices) => devices,
            Err(error) => return self.gone(error),
        };
        let Some(device) = devices.iter().find(|d| d.id == id) else { return no_preview() };
        let width = max_width.unwrap_or(PREVIEW_WIDTH).clamp(64, 1920);
        let asking = async {
            let tunnel = self.reach(&session, &identity, device, Ports { tcp: Vec::new(), udp: Vec::new() }).await.map_err(|_| "unreachable")?;
            let request = serde_json::to_vec(&json!({"app": {"preview": {"max_width": width}}})).map_err(|_| "request")?;
            let answer = tunnel.app_request(&request).await.map_err(|_| "no_answer")?;
            serde_json::from_slice::<Value>(&answer).map_err(|_| "bad_answer")
        };
        match tokio::time::timeout(PREVIEW_WAIT, asking).await {
            Ok(Ok(answer)) if answer.get("jpeg").and_then(Value::as_str).is_some_and(|jpeg| !jpeg.is_empty()) => answer,
            // The computer answered with nothing to show, and says why.
            Ok(Ok(answer)) => json!({"ok": false, "err": answer.get("err").cloned().unwrap_or_else(|| "empty".into())}),
            Ok(Err(why)) => json!({"ok": false, "err": why}),
            Err(_) => json!({"ok": false, "err": "timed_out"}),
        }
    }

    async fn pair(&self, client_id: &str, certificate_sha256: &str, pin: &str) -> Value {
        let tunnel = self.tunnel.lock().await;
        let Some(tunnel) = tunnel.as_ref() else { return failed("not_connected") };
        let request = json!({"pair": {"client_id": client_id, "certificate_sha256": certificate_sha256, "pin": pin}});
        match tunnel.request(request.to_string().as_bytes()).await {
            // The hosting computer answers "paired", "revoked", "not_allowed" or "failed".
            Ok(answer) => json!({"ok": true, "answer": serde_json::from_slice::<Value>(&answer).unwrap_or(Value::Null)}),
            Err(_) => failed("no_answer"),
        }
    }
}

fn configure(folder: String, test_account: Option<(String, String)>) -> Value {
    let account = match &test_account {
        Some((origin, address)) => Service::local_test(origin, address),
        None => Service::new(account::ORIGIN),
    };
    let (Ok(account), Ok(runtime)) = (account, tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()) else {
        return failed("not_configured");
    };
    let folder = PathBuf::from(folder).join("secpar");
    let app = App { runtime, account, store: FileStore(folder.clone()), folder, tunnel: tokio::sync::Mutex::new(None) };
    match APP.set(app) {
        Ok(()) => json!({"ok": true}),
        // The app's process outlives its screens; a second screen finds it configured.
        Err(_) => json!({"ok": true, "already": true}),
    }
}

fn call(request: &str) -> Value {
    let Ok(request) = serde_json::from_str::<Request>(request) else { return failed("unknown_request") };
    // These two do not touch the stream's tunnel or the signed-in session's
    // place in line: pairing runs while the app's own pairing call waits, and a
    // preview runs on its own short tunnel beside whatever else is happening.
    let alone = matches!(request, Request::Pair { .. } | Request::Preview { .. });
    let _turn = if alone { None } else { Some(TURN.lock().unwrap_or_else(|p| p.into_inner())) };
    if let Request::Configure { folder, test_account } = request {
        return configure(folder, test_account);
    }
    let Some(app) = APP.get() else { return failed("not_configured") };
    app.runtime.block_on(async {
        match request {
            Request::Configure { .. } => failed("not_configured"),
            Request::State => app.state().await,
            Request::SignIn { email, password, name } => app.sign_in(&email, &password, &name).await,
            Request::SignOut => app.sign_out().await,
            Request::Devices => app.devices().await,
            Request::RemoveDevice { id } => app.remove_device(&id).await,
            Request::Friends => app.friends().await,
            Request::AddFriend { handle } => app.add_friend(&handle).await,
            Request::AcceptFriend { id } => app.accept_friend(&id).await,
            Request::RemoveFriend { id } => app.remove_friend(&id).await,
            Request::FriendSettings { id, connect, input, keyboard, mouse } => {
                app.save_friend_settings(&id, FriendChange { input, connect, keyboard, mouse }).await
            }
            Request::Connect { id } => app.connect(&id).await,
            Request::Preview { id, max_width } => app.preview(&id, max_width).await,
            Request::Pair { client_id, certificate_sha256, pin } => app.pair(&client_id, &certificate_sha256, &pin).await,
            Request::Disconnect => {
                *app.tunnel.lock().await = None;
                json!({"ok": true})
            }
        }
    })
}

/// `String win.secpar.app.Core.call(String request)`
#[no_mangle]
pub extern "system" fn Java_win_secpar_app_Core_call<'local>(mut env: JNIEnv<'local>, _class: JClass<'local>, request: JString<'local>) -> jstring {
    let answer = match env.get_string(&request) {
        Ok(text) => {
            let text: String = text.into();
            std::panic::catch_unwind(|| call(&text)).unwrap_or_else(|_| failed("internal_error"))
        }
        Err(_) => failed("unknown_request"),
    };
    env.new_string(answer.to_string()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}
