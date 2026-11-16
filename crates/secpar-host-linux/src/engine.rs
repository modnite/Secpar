//! Managed Sunshine engine and the loopback tunnel in front of it.
//!
//! The engine is the bundled executable, started and supervised by
//! `secpar-gamestream-host`, and it listens on this computer only. A device
//! reaches it through a `secpar-tunnel` host on an ephemeral UDP port that
//! forwards to `127.0.0.1:46989`. Nothing from the network chooses the engine
//! path or the port it uses (`docs/gamestream-engine.md`).
//!
//! The package ships `engine/` (the executable and its assets) beside the app
//! executable. Because Sunshine expects a writable working folder, the first
//! run copies that whole folder into `<data>/engine` and runs it from there.
//!
//! This is the platform half of [`EngineBackend`]: the authenticated 3.x agent
//! owns who may ask (`crates/secpar-host-core/src/agent/gamestream.rs`), and the
//! approval below binds the engine certificate and the client's tunnel
//! certificate to that authenticated device.

use crate::store;
use secpar_gamestream_host::{Capture, ClientIdentity, Config, Encoder, Host};
use secpar_host_core::agent::{
    engine::{Broker, ClientTunnel, EngineBackend, EngineInfo},
    BoxFut, Outbox, Session,
};
use secpar_tunnel::{Allowed, Fingerprint, Identity, Ports};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

/// The engine's base port on every Secpar host.
pub const BASE_PORT: u16 = 46989;
/// The engine listens here only; a device arrives through the tunnel.
const ENGINE_IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// How many of a client's endpoints are punched, and for how long.
const CANDIDATES: usize = 16;
const PUNCHES: u32 = 25;
const PUNCH_INTERVAL: Duration = Duration::from_millis(200);

const NO_IDENTITY: &str = "The Secpar tunnel identity could not be read";
const BAD_TUNNEL: &str = "The client's tunnel identity is invalid";
const NO_ENGINE: &str = "The bundled streaming engine is not installed";
const BAD_PACKAGE: &str = "The packaged streaming engine could not be read";
const BROKEN_PACKAGE: &str = "The packaged streaming engine is incomplete";
const SAVE_ENGINE: &str = "The bundled streaming engine could not be installed";

/// What a package's `engine/` folder may hold. A real one is a handful of files,
/// so anything beyond this is not the package.
const MAX_FILES: usize = 4096;
const MAX_BYTES: u64 = 1 << 30;
const MAX_DEPTH: u8 = 16;

/// The bundled engine, `<data>/engine/sunshine`. `SECPAR_ENGINE_PATH` is a
/// development override only; a path from a network message is never accepted.
pub fn executable(data: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SECPAR_ENGINE_PATH") {
        let path = PathBuf::from(path);
        return (path.is_absolute() && path.is_file()).then_some(path);
    }
    let bundled = data.join("engine").join("sunshine");
    bundled.is_file().then_some(bundled)
}

/// The `engine/` folder the package shipped, next to the app executable.
fn packaged() -> Option<PathBuf> {
    let directory = std::env::current_exe().ok()?.parent()?.join("engine");
    (directory.join("sunshine").is_file()).then_some(directory)
}

/// Whether hosting has an engine to run here: already installed, in the
/// package waiting for its first copy, or given by the development override.
pub fn available(data: &Path) -> bool {
    executable(data).is_some() || packaged().is_some()
}

/// First run: copies the package's whole `engine/` folder, assets included, into
/// `<data>/engine`, which is where the supervisor runs it from. A folder that is
/// already there is left alone, and an incomplete copy never becomes the real
/// one. Files are copied as they are; symbolic links are never followed.
fn install(data: &Path) -> Result<(), &'static str> {
    let engine = data.join("engine");
    if engine.join("sunshine").is_file() {
        return Ok(());
    }
    match packaged() {
        Some(from) => install_from(&from, data),
        None => Ok(()),
    }
}

fn install_from(from: &Path, data: &Path) -> Result<(), &'static str> {
    let engine = data.join("engine");
    let temporary = data.join(format!("engine-{}", store::hex(&store::random::<8>())));
    let _ = std::fs::remove_dir_all(&temporary);
    let result = (|| {
        let mut budget = Budget { files: 0, bytes: 0 };
        copy_directory(from, &temporary, &mut budget, 0)?;
        if !temporary.join("sunshine").is_file() {
            return Err(BROKEN_PACKAGE);
        }
        // A folder without the engine is a copy that stopped part-way, not
        // something the owner keeps: the engine owns no state outside it.
        if engine.exists() {
            std::fs::remove_dir_all(&engine).map_err(|_| SAVE_ENGINE)?;
        }
        std::fs::rename(&temporary, &engine).map_err(|_| SAVE_ENGINE)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&temporary);
    }
    result
}

struct Budget {
    files: usize,
    bytes: u64,
}

fn copy_directory(from: &Path, to: &Path, budget: &mut Budget, depth: u8) -> Result<(), &'static str> {
    if depth > MAX_DEPTH {
        return Err(BAD_PACKAGE);
    }
    std::fs::create_dir_all(to).map_err(|_| SAVE_ENGINE)?;
    let entries = std::fs::read_dir(from).map_err(|_| BAD_PACKAGE)?;
    for entry in entries {
        let entry = entry.map_err(|_| BAD_PACKAGE)?;
        let kind = entry.file_type().map_err(|_| BAD_PACKAGE)?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_directory(&entry.path(), &target, budget, depth + 1)?;
        } else if kind.is_file() {
            let size = entry.metadata().map_err(|_| BAD_PACKAGE)?.len();
            budget.files += 1;
            budget.bytes = budget.bytes.saturating_add(size);
            if budget.files > MAX_FILES || budget.bytes > MAX_BYTES {
                return Err(BAD_PACKAGE);
            }
            std::fs::copy(entry.path(), &target).map_err(|_| SAVE_ENGINE)?;
        }
        // Symbolic links and special files are not part of the package, so they
        // are left out rather than followed somewhere else.
    }
    Ok(())
}

/// KDE Plasma on Wayland captures through KWin; everywhere else the engine
/// picks its own capture.
fn capture() -> Capture {
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.contains("KDE"));
    if kde && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        Capture::KWin
    } else {
        Capture::Automatic
    }
}

/// This computer's IPv4 addresses, best first (empty elsewhere).
fn addresses() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        crate::stream::local_addresses()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// Lowercase hex SHA-256 to bytes; anything else is not a certificate pin.
fn pin_bytes(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut pin = [0u8; 32];
    for (i, pair) in text.as_bytes().chunks(2).enumerate() {
        pin[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(pin)
}

/// The endpoints worth punching: real, reachable unicast addresses only.
fn places(candidates: &[SocketAddr]) -> Vec<SocketAddr> {
    candidates
        .iter()
        .copied()
        .filter(|a| a.port() != 0 && !a.ip().is_unspecified() && !a.ip().is_multicast())
        .take(CANDIDATES)
        .collect()
}

/// Sends a small packet from the tunnel's own port towards each endpoint for a
/// few seconds, so the routers in between let that device's packets in.
fn punch(tunnel: &Arc<secpar_tunnel::Host>, candidates: &[SocketAddr]) {
    let places = places(candidates);
    if places.is_empty() {
        return;
    }
    let tunnel = tunnel.clone();
    tokio::spawn(async move {
        for _ in 0..PUNCHES {
            tunnel.punch(&places);
            tokio::time::sleep(PUNCH_INTERVAL).await;
        }
    });
}

#[derive(Serialize, Deserialize)]
struct Saved {
    certificate: Vec<u8>,
    private_key: Vec<u8>,
}

/// The host's tunnel identity. It is created once and then reused, because a
/// client pins this certificate across connections.
fn identity(data: &Path) -> Result<Identity, &'static str> {
    let path = data.join("tunnel-identity.json");
    match store::read_private(&path) {
        Ok(bytes) => {
            let saved: Saved = serde_json::from_slice(&bytes).map_err(|_| NO_IDENTITY)?;
            if saved.certificate.is_empty() || saved.private_key.is_empty() {
                return Err(NO_IDENTITY);
            }
            Ok(Identity::from_der(saved.certificate, saved.private_key))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let identity = Identity::generate().map_err(|error| error.0)?;
            let saved = Saved {
                certificate: identity.certificate_der().to_vec(),
                private_key: identity.key_der().to_vec(),
            };
            let bytes = serde_json::to_vec(&saved).map_err(|_| NO_IDENTITY)?;
            store::write_private(&path, &bytes).map_err(|_| NO_IDENTITY)?;
            Ok(identity)
        }
        Err(_) => Err(NO_IDENTITY),
    }
}

/// The tunnel in front of the engine, and the devices allowed through it.
pub struct Tunnel {
    host: Arc<secpar_tunnel::Host>,
    allowed: Arc<Allowed>,
    /// This host's tunnel certificate pin, which clients must accept.
    pin: String,
    /// Each device's tunnel certificate, so a revoked device loses its way in.
    devices: Mutex<HashMap<String, Fingerprint>>,
}

impl Tunnel {
    /// Opens an ephemeral UDP port in front of the engine on loopback.
    pub async fn open(data: &Path) -> Result<Self, &'static str> {
        let identity = identity(data)?;
        let pin = store::hex(&identity.fingerprint());
        let allowed = Allowed::new([]);
        let host = secpar_tunnel::serve(
            SocketAddr::from(([0, 0, 0, 0], 0)),
            &identity,
            allowed.clone(),
            ENGINE_IP,
            Ports::engine(BASE_PORT),
        )
        .await
        .map_err(|error| error.0)?;
        Ok(Self {
            host: Arc::new(host),
            allowed,
            pin,
            devices: Mutex::new(HashMap::new()),
        })
    }
    pub fn pin(&self) -> &str {
        &self.pin
    }
    /// The UDP port every advertised address uses.
    pub fn port(&self) -> Option<u16> {
        self.host.local_addr().ok().map(|address| address.port())
    }
    /// Where a client should try to reach this tunnel, LAN first.
    pub fn addresses(&self) -> Vec<String> {
        addresses()
    }
    /// Lets one paired client in and opens its way through NAT
    /// (`docs/gamestream-engine.md`, steps 2 and 3). The engine still refuses a
    /// client certificate that was not approved.
    pub fn admit(&self, device: &str, certificate_sha256: &str, candidates: &[SocketAddr]) -> bool {
        let Some(fingerprint) = pin_bytes(certificate_sha256) else {
            return false;
        };
        let allowed: Vec<Fingerprint> = {
            let mut devices = self.devices.lock().unwrap();
            devices.insert(device.to_string(), fingerprint);
            devices.values().copied().collect()
        };
        self.host.set_allowed(allowed);
        punch(&self.host, candidates);
        true
    }
    /// Takes this device's way in away again.
    pub fn forget(&self, device: &str) {
        let allowed: Vec<Fingerprint> = {
            let mut devices = self.devices.lock().unwrap();
            if devices.remove(device).is_none() {
                return;
            }
            devices.values().copied().collect()
        };
        self.host.set_allowed(allowed);
    }
    /// Whether this client tunnel certificate can use the tunnel right now.
    pub fn allows(&self, certificate_sha256: &str) -> bool {
        pin_bytes(certificate_sha256).is_some_and(|pin| self.allowed.contains(&pin))
    }
    #[cfg(test)]
    fn has(&self, device: &str) -> bool {
        self.devices.lock().unwrap().contains_key(device)
    }
}

/// The running supervisor and the tunnel in front of it. Dropping it ends both:
/// the engine child is killed with its controller, and the tunnel port closes.
pub struct Engine {
    host: Arc<Host>,
    tunnel: Arc<Tunnel>,
    /// Display names for the engine's own client list, by Secpar device.
    names: Mutex<HashMap<String, String>>,
}

impl Engine {
    /// Starts the bundled engine, then the tunnel. The first start can take up
    /// to a minute while the engine looks for capture and encoder support.
    pub async fn start(data: &Path) -> Result<Self, &'static str> {
        let executable = match executable(data) {
            Some(executable) => executable,
            None => {
                // The package's engine folder is copied into the data directory
                // once, because the engine wants a writable working folder.
                install(data)?;
                executable(data).ok_or(NO_ENGINE)?
            }
        };
        let config = Config {
            executable,
            state: data.join("gamestream"),
            base_port: BASE_PORT,
            capture: capture(),
            encoder: Encoder::Automatic,
            audio_sink: None,
        };
        let host = Host::start(config).await.map_err(|error| error.0)?;
        // The tunnel only opens once the engine is ready, so an advertised
        // `gamestream` capability always has both behind it.
        let tunnel = Tunnel::open(data).await?;
        Ok(Self {
            host: Arc::new(host),
            tunnel: Arc::new(tunnel),
            names: Mutex::new(HashMap::new()),
        })
    }
    pub fn tunnel(&self) -> &Tunnel {
        &self.tunnel
    }
    /// The engine half of the authenticated agent session for one device. The
    /// agent only calls this after it has authenticated the device as the owner's.
    pub fn agent_session(self: Arc<Self>, owner: &str, device: &str, out: Outbox) -> Box<dyn Session> {
        Broker::new(self, owner).session(device, out)
    }
    /// Ends every engine grant held by one device. The tunnel entry goes too,
    /// because revocation runs through [`EngineBackend::revoke`].
    pub async fn revoke_grants(self: Arc<Self>, owner: &str, device: &str) -> Result<usize, &'static str> {
        Broker::new(self, owner).revoke_device(device).await
    }
    /// The name this device shows under in the engine's client list.
    pub fn name_device(&self, device: &str, name: &str) {
        let name: String = name.chars().take(64).collect();
        if name.is_empty() {
            return;
        }
        self.names
            .lock()
            .unwrap()
            .insert(device.to_string(), name);
    }
    fn device_name(&self, device: &str) -> String {
        self.names
            .lock()
            .unwrap()
            .get(device)
            .cloned()
            .unwrap_or_else(|| "Secpar device".into())
    }
}

impl EngineBackend for Engine {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            http_port: self.host.info.http_port,
            https_port: self.host.info.https_port,
            certificate_sha256: self.host.info.certificate_sha256.clone(),
            tunnel_port: self.tunnel.port().unwrap_or(0),
            tunnel_certificate_sha256: self.tunnel.pin().to_string(),
            tunnel_addresses: self.tunnel.addresses(),
        }
    }
    fn approve(
        &self,
        owner: &str,
        device: &str,
        client_id: &str,
        certificate_sha256: &str,
        pin: &str,
        tunnel: &ClientTunnel,
    ) -> BoxFut<Result<(), &'static str>> {
        if pin_bytes(&tunnel.certificate_sha256).is_none() {
            return Box::pin(async { Err(BAD_TUNNEL) });
        }
        let host = self.host.clone();
        let way = self.tunnel.clone();
        let identity = ClientIdentity {
            client_id: client_id.to_string(),
            certificate_sha256: certificate_sha256.to_string(),
        };
        let client = tunnel.clone();
        let (owner, device, pin, name) = (
            owner.to_string(),
            device.to_string(),
            pin.to_string(),
            self.device_name(device),
        );
        Box::pin(async move {
            // The client's tunnel first: the component's pairing request can
            // only reach the engine through it. A first call often runs before
            // the component has opened its pairing; the tunnel entry stays and
            // the client retries while the component pairs.
            if !way.admit(&device, &client.certificate_sha256, &client.candidates) {
                return Err(BAD_TUNNEL);
            }
            // The engine records the certificate against this authenticated
            // device.
            host.approve_for(&owner, &device, &identity, &pin, &name)
                .await
                .map_err(|error| error.0)?;
            Ok(())
        })
    }
    fn revoke(&self, owner: &str, device: &str) -> BoxFut<Result<usize, &'static str>> {
        let host = self.host.clone();
        let way = self.tunnel.clone();
        let (owner, device) = (owner.to_string(), device.to_string());
        Box::pin(async move {
            let removed = host.revoke(&owner, &device).await.map_err(|error| error.0)?;
            way.forget(&device);
            Ok(removed)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SECPAR_ENGINE_PATH` is process-wide, so the tests that touch it take turns.
    static ENV: Mutex<()> = Mutex::new(());

    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "secpar-engine-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// An `engine/` folder as the package lays it out: the binary and its assets.
    fn package(name: &str) -> PathBuf {
        let engine = directory(name).join("engine");
        std::fs::create_dir_all(engine.join("assets").join("shaders")).unwrap();
        std::fs::write(engine.join("sunshine"), b"engine binary").unwrap();
        std::fs::write(engine.join("assets").join("apps.json"), b"{}").unwrap();
        std::fs::write(
            engine.join("assets").join("shaders").join("convert.frag"),
            b"shader",
        )
        .unwrap();
        engine
    }

    fn leftovers(data: &Path) -> Vec<String> {
        std::fs::read_dir(data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("engine-"))
            .collect()
    }

    #[test]
    fn the_packages_engine_folder_is_copied_once_with_all_of_its_assets() {
        let data = directory("install");
        let from = package("install-package");
        install_from(&from, &data).unwrap();
        let engine = data.join("engine");
        assert_eq!(std::fs::read(engine.join("sunshine")).unwrap(), b"engine binary");
        assert_eq!(std::fs::read(engine.join("assets").join("apps.json")).unwrap(), b"{}");
        assert_eq!(
            std::fs::read(engine.join("assets").join("shaders").join("convert.frag")).unwrap(),
            b"shader",
            "assets are part of the copy"
        );
        assert!(executable(&data).is_some(), "the supervisor runs the copy");
        assert!(leftovers(&data).is_empty(), "no temporary folder is left");
        // A second start keeps what is already installed.
        let marker = engine.join("keep.txt");
        std::fs::write(&marker, b"installed").unwrap();
        install(&data).unwrap();
        assert_eq!(std::fs::read(&marker).unwrap(), b"installed");
    }

    #[test]
    fn an_incomplete_package_never_becomes_the_engine() {
        let data = directory("broken");
        let from = directory("broken-package").join("engine");
        std::fs::create_dir_all(from.join("assets")).unwrap();
        std::fs::write(from.join("assets").join("apps.json"), b"{}").unwrap();
        assert_eq!(install_from(&from, &data), Err(BROKEN_PACKAGE));
        assert!(!data.join("engine").exists(), "half a copy is not kept");
        assert!(leftovers(&data).is_empty());
        // A copy that stopped part-way earlier is replaced by a complete one.
        std::fs::create_dir_all(data.join("engine").join("assets")).unwrap();
        std::fs::write(data.join("engine").join("assets").join("old"), b"old").unwrap();
        install_from(&package("broken-good"), &data).unwrap();
        assert_eq!(std::fs::read(data.join("engine").join("sunshine")).unwrap(), b"engine binary");
        assert!(!data.join("engine").join("assets").join("old").exists());
        assert!(leftovers(&data).is_empty());
    }

    #[tokio::test]
    async fn the_tunnel_identity_is_created_once_and_then_pinned() {
        let data = directory("identity");
        let first = Tunnel::open(&data).await.unwrap();
        let pin = first.pin().to_string();
        assert_eq!(pin.len(), 64);
        assert!(pin.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        let port = first.port().expect("an ephemeral port");
        assert_ne!(port, 0);
        assert_ne!(port, BASE_PORT, "the tunnel never takes the engine's port");
        drop(first);
        let again = Tunnel::open(&data).await.unwrap();
        assert_eq!(again.pin(), pin, "a client's pin must survive a restart");
        assert_ne!(Tunnel::open(&directory("other")).await.unwrap().pin(), pin);
    }

    #[tokio::test]
    async fn only_a_real_certificate_pin_opens_the_tunnel_and_revoking_closes_it() {
        let tunnel = Tunnel::open(&directory("admit")).await.unwrap();
        let pin = "ab".repeat(32);
        let bad = [
            String::new(),
            "AB".repeat(32),
            "ab".repeat(31),
            "zz".repeat(32),
        ];
        for bad in &bad {
            assert!(!tunnel.admit("phone", bad, &[]), "{bad:?} is not a pin");
        }
        assert!(!tunnel.allows(&pin));
        assert!(tunnel.admit("phone", &pin, &[]));
        assert!(tunnel.allows(&pin));
        // A second device is added without losing the first.
        assert!(tunnel.admit("laptop", &"cd".repeat(32), &[]));
        assert!(tunnel.allows(&pin) && tunnel.allows(&"cd".repeat(32)));
        tunnel.forget("phone");
        assert!(!tunnel.allows(&pin), "the revoked device keeps no way in");
        assert!(!tunnel.has("phone"));
        assert!(tunnel.allows(&"cd".repeat(32)));
    }

    #[tokio::test]
    async fn only_reachable_endpoints_are_punched() {
        let places = places(&[
            "198.51.100.8:54321".parse().unwrap(),
            "0.0.0.0:54321".parse().unwrap(),
            "192.168.1.30:0".parse().unwrap(),
            "239.1.1.1:9".parse().unwrap(),
            "192.168.1.30:54321".parse().unwrap(),
        ]);
        assert_eq!(
            places,
            vec![
                "198.51.100.8:54321".parse().unwrap(),
                "192.168.1.30:54321".parse().unwrap(),
            ]
        );
        // Admitting a device with no usable endpoint is still allowed.
        let tunnel = Tunnel::open(&directory("punch")).await.unwrap();
        assert!(tunnel.admit("phone", &"ab".repeat(32), &places));
    }

    #[test]
    fn the_engine_is_the_bundled_executable_unless_development_says_otherwise() {
        let _env = ENV.lock().unwrap();
        let data = directory("executable");
        assert_eq!(executable(&data), None, "nothing bundled in an empty folder");
        std::fs::create_dir_all(data.join("engine")).unwrap();
        let bundled = data.join("engine").join("sunshine");
        std::fs::write(&bundled, b"#!/bin/sh\n").unwrap();
        assert_eq!(executable(&data), Some(bundled.clone()));
        // The development override must be an absolute existing file.
        std::env::set_var("SECPAR_ENGINE_PATH", &bundled);
        assert_eq!(executable(&data), Some(bundled.clone()));
        std::env::set_var("SECPAR_ENGINE_PATH", "relative/sunshine");
        assert_eq!(executable(&data), None);
        std::env::remove_var("SECPAR_ENGINE_PATH");
        assert_eq!(executable(&data), Some(bundled));
    }

    #[tokio::test]
    async fn a_missing_engine_is_a_plain_refusal() {
        let _env = ENV.lock().unwrap();
        let data = directory("missing");
        assert_eq!(
            Engine::start(&data).await.err(),
            Some("The bundled streaming engine is not installed")
        );
    }
}
