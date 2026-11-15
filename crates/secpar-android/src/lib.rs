//! Secpar's own part of the Android app: the device identity, the account, the
//! list of computers and the tunnel. The app passes one JSON request and gets
//! one JSON answer back (`win.secpar.app.Core.call`); each call blocks, so the
//! app makes it off its main thread.
//!
//! Android connects only. The identity and the session are kept in the app's
//! private folder, which other apps cannot read.
use jni::{
    objects::{JClass, JString},
    sys::jstring,
    JNIEnv,
};
use secpar_core::{
    account::{self, Error, Platform, Service, Session},
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
    /// Opens the tunnel to one of the account's hosting computers.
    Connect { id: String },
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
fn refused(error: Error) -> Value {
    match error {
        Error::Unreachable => failed("unreachable"),
        Error::Refused { code, message, .. } => json!({"ok": false, "error": code, "message": message}),
        Error::Invalid(_) => failed("invalid_answer"),
        Error::Store(_) => failed("store_unavailable"),
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
        let (Some(reach), Some(host)) = (&device.reach, identity::unhex(&device.fingerprint)) else { return failed("offline") };
        let addresses: Vec<SocketAddr> = reach.addresses.iter().filter_map(|a| a.parse().ok()).map(|ip| SocketAddr::new(ip, reach.port)).collect();
        if addresses.is_empty() {
            return failed("offline");
        }
        let mut tunnel = self.tunnel.lock().await;
        *tunnel = None;
        // A computer that has not heard of this device yet fetches a new list
        // when it is refused, so a second and third try are worth making.
        let connected = json!({"ok": true, "address": LOCAL, "port": ENGINE_BASE, "host": device.fingerprint, "name": device.name});
        let Ok(listen) = LOCAL.parse() else { return failed("no_socket") };
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else { return failed("no_socket") };
            let direct = secpar_tunnel::connect_on(socket, &addresses, &identity, host, listen, Ports::engine(ENGINE_BASE));
            match tokio::time::timeout(Duration::from_millis(1500), direct).await {
                Ok(Ok(opened)) => {
                    *tunnel = Some(opened);
                    return connected;
                }
                // Answered and refused: worth another try. No answer at all:
                // not on the same network, so go through the relay.
                Ok(Err(_)) => continue,
                Err(_) => break,
            }
        }

        // Meet the computer across the internet: tell it where this device is
        // seen from outside, and its relay address if the service offers a relay;
        // it answers with its own. Direct first, the relay as the fallback.
        let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else { return failed("no_socket") };
        let Ok(opened) = secpar_tunnel::open(socket) else { return failed("no_socket") };
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
        if let Ok(access) = self.account.relay(&session).await {
            if let Some(server) = first(access.server.clone()).await {
                let starting = relay.clone();
                own_relay = tokio::task::spawn_blocking(move || starting.start(server, access.username, access.credential)).await.ok().flatten();
            }
        }
        if seen.is_none() && own_relay.is_none() {
            return failed("no_internet_path");
        }
        let Ok(nowhere) = "0.0.0.0:9".parse::<SocketAddr>() else { return failed("no_socket") };
        let mut here = vec![seen.unwrap_or(nowhere)];
        here.extend(own_relay);
        let there = match self.account.knock(&session, &device.id, &identity.fingerprint(), &here).await {
            Ok(there) if !there.is_empty() => there,
            _ => return failed("computer_not_answering"),
        };
        let mut candidates: Vec<SocketAddr> = there.first().copied().filter(|a| !a.ip().is_unspecified()).into_iter().collect();
        if let (Some(_), Some(host_relay)) = (own_relay, there.get(1).copied()) {
            let adding = relay.clone();
            let _ = tokio::task::spawn_blocking(move || adding.add(host_relay)).await;
            candidates.push(host_relay);
        }
        if candidates.is_empty() {
            return failed("no_internet_path");
        }
        match secpar_tunnel::connect_through(opened, &candidates, &identity, host, listen, Ports::engine(ENGINE_BASE)).await {
            Ok(opened) => {
                *tunnel = Some(opened);
                connected
            }
            Err(_) => failed("not_reachable"),
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
    // Pairing runs while the app's own pairing call is waiting, so it does not queue behind it.
    let _turn = if matches!(request, Request::Pair { .. }) { None } else { Some(TURN.lock().unwrap_or_else(|p| p.into_inner())) };
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
            Request::Connect { id } => app.connect(&id).await,
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
