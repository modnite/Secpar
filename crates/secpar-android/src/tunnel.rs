//! The managed engine's tunnel for the 3.x app (docs/gamestream-engine.md). The app has
//! already authenticated the computer on its own control connection and the computer has
//! allow-listed this device's tunnel certificate by the time `connect` is called.
//!
//! One QUIC connection carries the engine's TCP and UDP ports. On the phone the tunnel
//! listens on the loopback engine base only, so Moonlight talks to `127.0.0.1:46989` as if
//! the engine were local and the engine is never opened on the network.
use jni::{
    objects::{JClass, JString},
    sys::jstring,
    JNIEnv,
};
use secpar_tunnel::{Client, Identity, Opened, Ports};
use serde_json::{json, Value};
use std::{
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// The engine's base port on every Secpar hosting computer (secpar_hosting::ENGINE_BASE).
const ENGINE_BASE: u16 = 46989;
const LISTEN: &str = "127.0.0.1";
/// Used to learn which address the computer must punch towards.
const STUN: &str = "stun.cloudflare.com:3478";
/// One tunnel attempt can try several addresses; the tunnel itself gives each 8 seconds.
const OPEN_TIMEOUT: Duration = Duration::from_secs(20);

struct Tunnel {
    runtime: tokio::runtime::Runtime,
    /// Kept for the life of the process, so a retry reuses the certificate the computer
    /// was told about during pairing.
    identity: Identity,
    /// A socket opened but not connected yet: its UDP port is the one to punch towards.
    opened: Option<Opened>,
    client: Option<Client>,
}

static TUNNEL: OnceLock<Option<Mutex<Tunnel>>> = OnceLock::new();

fn failed(code: &str) -> Value {
    json!({"ok": false, "error": code})
}

fn state() -> Option<&'static Mutex<Tunnel>> {
    TUNNEL
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().ok()?;
            let identity = Identity::generate().ok()?;
            Some(Mutex::new(Tunnel { runtime, identity, opened: None, client: None }))
        })
        .as_ref()
}

fn stun() -> Option<SocketAddr> {
    STUN.to_socket_addrs().ok().and_then(|mut found| found.find(SocketAddr::is_ipv4))
}

fn addresses(text: &str) -> Vec<std::net::IpAddr> {
    serde_json::from_str::<Vec<String>>(text)
        .unwrap_or_default()
        .iter()
        .filter_map(|value| value.trim().parse().ok())
        .collect()
}

/// A tunnel socket with a fresh certificate pin and UDP port for this attempt.
fn start() -> Value {
    let Some(state) = state() else { return failed("not_available") };
    let mut tunnel = state.lock().unwrap_or_else(|poison| poison.into_inner());
    // A new attempt replaces whatever was left from the last one.
    tunnel.client = None;
    tunnel.opened = None;
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else { return failed("no_socket") };
    let Ok(local) = socket.local_addr() else { return failed("no_socket") };
    let opened = match tunnel.runtime.block_on(async { secpar_tunnel::open(socket) }) {
        Ok(opened) => opened,
        Err(_) => return failed("no_socket"),
    };
    let seen = match stun() {
        Some(stun) => {
            let relay = opened.relay();
            tunnel
                .runtime
                .block_on(async { tokio::task::spawn_blocking(move || relay.seen_from_outside(stun)).await.ok().flatten() })
        }
        None => None,
    };
    let pin = secpar_core::identity::hex(&tunnel.identity.fingerprint());
    tunnel.opened = Some(opened);
    json!({"ok": true, "certificate_sha256": pin, "port": local.port(), "seen": seen.map(|a| a.to_string())})
}

/// Connects to the computer's tunnel, which must present `host_pin`, and offers the
/// engine's ports on the loopback.
fn connect(text: &str, port: u16, host_pin: &str) -> Value {
    let Some(state) = state() else { return failed("not_available") };
    let mut tunnel = state.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(opened) = tunnel.opened.take() else { return failed("not_started") };
    let Some(pin) = secpar_core::identity::unhex(host_pin) else { return failed("bad_pin") };
    let candidates: Vec<SocketAddr> = addresses(text).into_iter().map(|ip| SocketAddr::new(ip, port)).collect();
    if candidates.is_empty() {
        return failed("no_addresses");
    }
    let Ok(listen) = LISTEN.parse() else { return failed("no_socket") };
    let identity = tunnel.identity.clone();
    let attempt = async {
        secpar_tunnel::connect_through(opened, &candidates, &identity, pin, listen, Ports::engine(ENGINE_BASE)).await
    };
    match tunnel.runtime.block_on(async { tokio::time::timeout(OPEN_TIMEOUT, attempt).await }) {
        Ok(Ok(client)) => {
            tunnel.client = Some(client);
            json!({"ok": true})
        }
        // The socket is spent either way; the caller may start a new attempt with the
        // certificate the computer already knows. The crate's own sentence is passed on.
        Ok(Err(error)) => json!({"ok": false, "error": "not_reachable", "msg": error.0}),
        Err(_) => json!({"ok": false, "error": "not_reachable", "msg": "The computer did not answer in time."}),
    }
}

fn close() {
    if let Some(state) = state() {
        let mut tunnel = state.lock().unwrap_or_else(|poison| poison.into_inner());
        tunnel.client = None;
        tunnel.opened = None;
    }
}

fn answer(env: &mut JNIEnv, value: Value) -> jstring {
    env.new_string(value.to_string()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn text(env: &mut JNIEnv, value: &JString) -> String {
    env.get_string(value).map(String::from).unwrap_or_default()
}

/// `String com.pcremote.engine.NativeTunnel.start()`
#[no_mangle]
pub extern "system" fn Java_com_pcremote_engine_NativeTunnel_start(mut env: JNIEnv, _: JClass) -> jstring {
    answer(&mut env, std::panic::catch_unwind(start).unwrap_or_else(|_| failed("internal_error")))
}

/// `String com.pcremote.engine.NativeTunnel.connect(String addresses, int port, String hostPin)`
#[no_mangle]
pub extern "system" fn Java_com_pcremote_engine_NativeTunnel_connect(
    mut env: JNIEnv,
    _: JClass,
    addresses: JString,
    port: jni::sys::jint,
    host_pin: JString,
) -> jstring {
    let addresses = text(&mut env, &addresses);
    let host_pin = text(&mut env, &host_pin);
    answer(
        &mut env,
        std::panic::catch_unwind(|| connect(&addresses, port as u16, &host_pin)).unwrap_or_else(|_| failed("internal_error")),
    )
}

/// `void com.pcremote.engine.NativeTunnel.close()`
#[no_mangle]
pub extern "system" fn Java_com_pcremote_engine_NativeTunnel_close(_: JNIEnv, _: JClass) {
    let _ = std::panic::catch_unwind(close);
}
