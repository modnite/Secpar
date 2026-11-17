//! The bundled Sunshine engine on the existing 3.x control connection
//! (docs/gamestream-engine.md): the supervisor, the tunnel in front of it and
//! the per-device tunnel grants.
//!
//! Nothing here runs in front of the host: `start` launches the supervisor and
//! the tunnel in the background, and the host advertises `gamestream` only once
//! both are up. The engine stays on loopback and never listens on the network.

use crate::agent::Outbox;
use secpar_gamestream_host::{Capture, ClientIdentity, Config, Encoder};
use secpar_host_core::agent::engine::{Broker, ClientTunnel, EngineBackend, EngineInfo};
use secpar_host_core::agent::{BoxFut, Session};
use secpar_tunnel::{Allowed, Ports};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The engine's base port, as docs/gamestream-engine.md fixes it.
pub const BASE_PORT: u16 = 46989;
/// Development-only override for the engine executable; never from a message.
const ENGINE_PATH: &str = "SECPAR_ENGINE_PATH";
/// The bundled engine folder, beside the app executable and in the data folder.
const ENGINE_DIR: &str = "engine";
/// A ceiling for the one-time copy, so a wrong folder cannot fill the disk.
const MAX_ENGINE_BYTES: u64 = 1 << 30;
/// Rounds of small packets towards the client's endpoints, so both NATs open.
const PUNCH_ROUNDS: u32 = 6;
const PUNCH_INTERVAL: Duration = Duration::from_millis(300);
/// Grants kept on disk; a device needs one per Moonlight client.
const MAX_GRANTS: usize = 512;
/// `gamestream/tunnel-grants.json` in the host data directory.
const GRANTS: &str = "tunnel-grants.json";

/// Where a device may reach this host's tunnel, LAN first. Read on every info
/// request, so a changed address or a new router mapping shows up at once.
pub type Addresses = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// The engine and tunnel, shared by the host handlers and the settings window.
#[derive(Clone)]
pub struct Managed {
    inner: Arc<Inner>,
}

struct Inner {
    /// The host data folder that holds `engine/` and `gamestream/`.
    data: PathBuf,
    executable: PathBuf,
    /// `gamestream/` in the host data directory.
    state: PathBuf,
    addresses: Addresses,
    /// The owner's encoder, preset and display, as they were when hosting started.
    picture: Picture,
    /// Set once the supervisor and the tunnel are both ready.
    parts: Mutex<Option<Parts>>,
    grants: Mutex<Vec<Grant>>,
}

struct Parts {
    engine: Arc<secpar_gamestream_host::Host>,
    tunnel: Arc<secpar_tunnel::Host>,
    info: EngineInfo,
}

/// A device allowed through the tunnel. Public keys and ids only, no secrets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Grant {
    owner: String,
    device: String,
    fingerprint: String,
}

/// The owner's picture settings handed to the engine.
#[derive(Clone, Default)]
pub struct Picture {
    pub encoder: String,
    pub preset: String,
    pub display: String,
}

/// Starts the engine in the background. A failure leaves the host as it was,
/// with PRS untouched and no `gamestream` capability.
pub fn start(data_dir: &Path, addresses: Addresses, picture: Picture) -> Managed {
    let state = data_dir.join("gamestream");
    let managed = Managed {
        inner: Arc::new(Inner {
            data: data_dir.to_path_buf(),
            executable: engine_path(data_dir),
            grants: Mutex::new(read_grants(&state.join(GRANTS))),
            state,
            addresses,
            picture,
            parts: Mutex::new(None),
        }),
    };
    // Secpar's own stream engine serves every device; the bundled Sunshine engine is no longer
    // shipped. SECPAR_WINDOWS_ENGINE=1 starts one placed in the data folder, for testing.
    if std::env::var_os("SECPAR_WINDOWS_ENGINE").is_some() {
        let launch = managed.clone();
        tokio::spawn(async move { launch.run().await });
    }
    managed
}

/// The bundled engine, copied from beside the app on first run. The installer
/// ships `engine/` next to the executable; the host runs it from its own data
/// folder, so the copy happens once and later starts use what is already there.
fn stage_engine(data: &Path) -> std::io::Result<bool> {
    let Some(app) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return Ok(false);
    };
    stage_engine_from(&app, data)
}

/// `stage_engine` with the app folder named, so it can be tested. Only a local
/// folder is ever copied; a network path is refused.
fn stage_engine_from(app: &Path, data: &Path) -> std::io::Result<bool> {
    if default_executable(data).is_file() {
        return Ok(false);
    }
    let source = app.join(ENGINE_DIR);
    if network_path(app) || !source.join(engine_file()).is_file() {
        return Ok(false);
    }
    let staging = data.join("engine.copy");
    if staging.is_dir() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(data)?;
    let mut budget = MAX_ENGINE_BYTES;
    copy_tree(&source, &staging, &mut budget)?;
    let target = data.join(ENGINE_DIR);
    if target.is_dir() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::rename(&staging, &target)?;
    Ok(true)
}

fn engine_file() -> &'static str {
    if cfg!(windows) {
        "sunshine.exe"
    } else {
        "sunshine"
    }
}

/// A UNC path or other network location, which this never copies from.
fn network_path(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy();
    text.starts_with(r"\\") || text.starts_with("//")
}

/// Copies regular files and folders, skipping links. `budget` is the most it
/// may write in total.
fn copy_tree(source: &Path, target: &Path, budget: &mut u64) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let to = target.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &to, budget)?;
        } else if kind.is_file() {
            let size = entry.metadata()?.len();
            if size > *budget {
                return Err(std::io::Error::other("The bundled engine folder is too large"));
            }
            *budget -= size;
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// The bundled executable, or the development override when it is a real file.
fn engine_path(data_dir: &Path) -> PathBuf {
    if let Some(value) = std::env::var_os(ENGINE_PATH) {
        let path = PathBuf::from(value);
        if path.is_absolute() && path.is_file() {
            crate::log::line("engine: SECPAR_ENGINE_PATH is in use (development only)");
            return path;
        }
        crate::log::line("engine: SECPAR_ENGINE_PATH is not an existing file; ignored");
    }
    default_executable(data_dir)
}

fn default_executable(data_dir: &Path) -> PathBuf {
    data_dir
        .join("engine")
        .join(if cfg!(windows) { "sunshine.exe" } else { "sunshine" })
}

impl Managed {
    /// True only when both parts are up; until then clients keep using PRS.
    pub fn is_ready(&self) -> bool {
        self.inner.parts.lock().unwrap().is_some()
    }

    /// The engine session for an authenticated owner device, or None while the
    /// engine is not ready.
    pub fn session(&self, device: &str, out: Outbox, owner: &str) -> Option<Box<dyn Session>> {
        if !self.is_ready() {
            return None;
        }
        // A fresh `Arc` around a clone: every `Managed` shares the same engine state.
        Some(Broker::new(Arc::new(self.clone()), owner).session(device, out))
    }

    /// Removes a device's engine grants outside the trait's owner scoping, for the
    /// host's device-removal path.
    pub async fn revoke_owner(&self, owner: &str, device: &str) -> Result<usize, &'static str> {
        EngineBackend::revoke(self, owner, device).await
    }

    async fn run(&self) {
        match self.launch().await {
            Ok((parts, port)) => {
                *self.inner.parts.lock().unwrap() = Some(parts);
                crate::log::line(&format!(
                    "engine: ready on 127.0.0.1:{BASE_PORT}, tunnel udp {port}"
                ));
            }
            Err(e) => crate::log::line(&format!("engine: not started: {e}")),
        }
    }

    async fn launch(&self) -> Result<(Parts, u16), String> {
        let inner = &self.inner;
        // A development override is used as it is; only the bundled engine is staged.
        if inner.executable == default_executable(&inner.data) {
            match stage_engine(&inner.data) {
                Ok(true) => crate::log::line("engine: copied the bundled engine into the data folder"),
                Ok(false) => {}
                Err(e) => crate::log::line(&format!("engine: the bundled engine could not be copied: {e}")),
            }
        }
        if !inner.executable.is_file() {
            return Err(format!("{} is missing", inner.executable.display()));
        }
        let state = inner.state.join("engine");
        std::fs::create_dir_all(&state).map_err(|e| format!("state folder: {e}"))?;
        let engine = secpar_gamestream_host::Host::start(Config {
            executable: inner.executable.clone(),
            state,
            base_port: BASE_PORT,
            capture: Capture::Automatic,
            encoder: Encoder::from_choice(&inner.picture.encoder),
            audio_sink: None,
            preset: secpar_gamestream_host::Preset::from_choice(&inner.picture.preset),
            output: Some(inner.picture.display.clone()).filter(|d| !d.is_empty()),
        })
        .await
        .map_err(|e| e.to_string())?;
        let identity = secpar_tunnel::Identity::generate().map_err(|e| e.to_string())?;
        let tunnel = secpar_tunnel::serve(
            SocketAddr::from(([0, 0, 0, 0], 0)),
            &identity,
            Allowed::new([]),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Ports::engine(BASE_PORT),
        )
        .await
        .map_err(|e| e.to_string())?;
        let port = tunnel.local_addr().map_err(|e| e.to_string())?.port();
        // Devices granted before a restart may reconnect without pairing again.
        tunnel.set_allowed(inner.fingerprints());
        let info = EngineInfo {
            http_port: engine.info.http_port,
            https_port: engine.info.https_port,
            certificate_sha256: engine.info.certificate_sha256.clone(),
            tunnel_port: port,
            tunnel_certificate_sha256: hex(&identity.fingerprint()),
            tunnel_addresses: Vec::new(),
        };
        Ok((
            Parts {
                engine: Arc::new(engine),
                tunnel: Arc::new(tunnel),
                info,
            },
            port,
        ))
    }
}

impl EngineBackend for Managed {
    fn info(&self) -> EngineInfo {
        // A session exists only while the parts are up, so the fallback below is
        // never what a client sees; it keeps the trait infallible.
        let mut info = self
            .inner
            .parts
            .lock()
            .unwrap()
            .as_ref()
            .map(|parts| parts.info.clone())
            .unwrap_or(EngineInfo {
                http_port: BASE_PORT,
                https_port: BASE_PORT - 5,
                certificate_sha256: String::new(),
                tunnel_port: 0,
                tunnel_certificate_sha256: String::new(),
                tunnel_addresses: Vec::new(),
            });
        info.tunnel_addresses = (self.inner.addresses)();
        info
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
        let inner = self.inner.clone();
        let (owner, device) = (owner.to_string(), device.to_string());
        let identity = ClientIdentity {
            client_id: client_id.to_string(),
            certificate_sha256: certificate_sha256.to_string(),
        };
        let (pin, name) = (pin.to_string(), client_id.to_string());
        let tunnel = tunnel.clone();
        Box::pin(async move {
            let (engine, tunnel_host) = inner.ready()?;
            // 1. The client's tunnel identity first. The component's pairing
            // request can only reach the engine once the tunnel is open, so the
            // grant must exist before the engine can see a pending pairing.
            inner.add_grant(&owner, &device, &tunnel.certificate_sha256)?;
            tunnel_host.set_allowed(inner.fingerprints());
            // 2. Small packets towards the client, so both NATs let the other in.
            for _ in 0..PUNCH_ROUNDS {
                tunnel_host.punch(&tunnel.candidates);
                tokio::time::sleep(PUNCH_INTERVAL).await;
            }
            // 3. The Moonlight certificate, through the engine's own journal.
            // A first call often runs before the component has opened its
            // pairing; the grant above stays and the client retries while the
            // component pairs.
            engine
                .approve_for(&owner, &device, &identity, &pin, &name)
                .await
                .map_err(|_| "The engine did not accept the device")?;
            Ok(())
        })
    }

    fn revoke(&self, owner: &str, device: &str) -> BoxFut<Result<usize, &'static str>> {
        let inner = self.inner.clone();
        let (owner, device) = (owner.to_string(), device.to_string());
        Box::pin(async move {
            let forgotten = inner.forget_device(&device);
            let engine = inner.parts.lock().unwrap().as_ref().map(|p| p.engine.clone());
            let Some(engine) = engine else {
                return Ok(forgotten);
            };
            // A grant may be recorded under the computer id from before the account
            // was known; revoke every owner this device actually appears under.
            let mut owners = vec![owner];
            for record in engine.grants().await {
                if record.device == device && !record.revoked && !owners.contains(&record.owner) {
                    owners.push(record.owner);
                }
            }
            let mut revoked = 0;
            for owner in owners {
                revoked = revoked.max(
                    engine
                        .revoke(&owner, &device)
                        .await
                        .map_err(|_| "The engine did not revoke the device")?,
                );
            }
            Ok(revoked.max(forgotten))
        })
    }
}

impl Inner {
    fn ready(&self) -> Result<(Arc<secpar_gamestream_host::Host>, Arc<secpar_tunnel::Host>), &'static str> {
        self.parts
            .lock()
            .unwrap()
            .as_ref()
            .map(|parts| (parts.engine.clone(), parts.tunnel.clone()))
            .ok_or("The engine is not available")
    }

    fn grants_path(&self) -> PathBuf {
        self.state.join(GRANTS)
    }

    fn fingerprints(&self) -> Vec<[u8; 32]> {
        self.grants
            .lock()
            .unwrap()
            .iter()
            .filter_map(|grant| parse_hex(&grant.fingerprint))
            .collect()
    }

    /// Records the client's tunnel certificate for this device. One certificate
    /// belongs to one device; a changed owner (the account was linked or unlinked)
    /// moves the record instead of adding a second one.
    fn add_grant(&self, owner: &str, device: &str, fingerprint: &str) -> Result<(), &'static str> {
        let valid = |v: &str| !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control);
        if !valid(owner) || !valid(device) || !hex_sha256(fingerprint) {
            return Err("The tunnel identity is invalid");
        }
        let mut grants = self.grants.lock().unwrap();
        // One certificate belongs to one device, as the engine journal also holds.
        if grants
            .iter()
            .any(|g| g.fingerprint == fingerprint && g.device != device)
        {
            return Err("This tunnel certificate belongs to another device");
        }
        let before = grants.clone();
        if let Some(existing) = grants
            .iter_mut()
            .find(|g| g.device == device && g.fingerprint == fingerprint)
        {
            if existing.owner == owner {
                return Ok(());
            }
            existing.owner = owner.into();
        } else {
            if grants.len() >= MAX_GRANTS {
                return Err("Too many tunnel grants");
            }
            grants.push(Grant {
                owner: owner.into(),
                device: device.into(),
                fingerprint: fingerprint.into(),
            });
        }
        if write_grants(&self.grants_path(), &grants).is_err() {
            *grants = before;
            return Err("Could not save the tunnel grant");
        }
        Ok(())
    }

    /// Drops every tunnel grant of this device, whatever owner they were made
    /// under; returns how many went.
    fn forget_device(&self, device: &str) -> usize {
        let removed = {
            let mut grants = self.grants.lock().unwrap();
            let before = grants.len();
            grants.retain(|g| g.device != device);
            let removed = before - grants.len();
            if removed > 0 && write_grants(&self.grants_path(), &grants).is_err() {
                crate::log::line("engine: could not save the tunnel grants after a removal");
            }
            removed
        };
        if removed > 0 {
            if let Some(parts) = self.parts.lock().unwrap().as_ref() {
                parts.tunnel.set_allowed(self.fingerprints());
            }
        }
        removed
    }
}

/// The addresses a client may try: IPs only, LAN first. The public one is added
/// only when the router reported it; nothing is invented.
pub fn addresses(lan: Vec<String>, public: Option<String>) -> Vec<String> {
    let mut list: Vec<String> = Vec::with_capacity(lan.len() + 1);
    for ip in lan.into_iter().chain(public) {
        if ip.parse::<IpAddr>().is_ok() && !list.contains(&ip) {
            list.push(ip);
        }
    }
    list
}

fn hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex(value: &str) -> Option<[u8; 32]> {
    if !hex_sha256(value) {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(value.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn read_grants(path: &Path) -> Vec<Grant> {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Vec<Grant>>(&bytes) {
            Ok(grants) if grants.len() <= MAX_GRANTS => grants
                .into_iter()
                .filter(|g| hex_sha256(&g.fingerprint))
                .collect(),
            Ok(_) => {
                crate::log::line("engine: the tunnel grants are too large; starting empty");
                Vec::new()
            }
            Err(_) => {
                crate::log::line("engine: the tunnel grants are unreadable; starting empty");
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => {
            crate::log::line("engine: could not read the tunnel grants");
            Vec::new()
        }
    }
}

fn write_grants(path: &Path, grants: &[Grant]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec(grants).map_err(std::io::Error::other)?;
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn managed(state: &Path) -> Managed {
        Managed {
            inner: Arc::new(Inner {
                data: state.to_path_buf(),
                executable: default_executable(state),
                grants: Mutex::new(read_grants(&state.join(GRANTS))),
                state: state.to_path_buf(),
                addresses: Arc::new(Vec::new),
                picture: Picture::default(),
                parts: Mutex::new(None),
            }),
        }
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "secpar-engine-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn grants_are_per_device_and_durable() {
        let dir = temp();
        let engine = managed(&dir);
        assert!(engine.inner.add_grant("acct", "phone", A).is_ok());
        assert!(engine.inner.add_grant("acct", "phone", A).is_ok(), "the same grant may repeat");
        assert_eq!(engine.inner.fingerprints(), vec![parse_hex(A).unwrap()]);
        // A certificate may not be moved to another device.
        assert!(engine.inner.add_grant("acct", "tablet", A).is_err());
        assert!(engine.inner.add_grant("other", "phone", B).is_ok());
        assert!(engine.inner.add_grant("acct", "phone", "short").is_err());

        let reopened = read_grants(&dir.join(GRANTS));
        assert_eq!(reopened.len(), 2, "a restart keeps the devices in");
        assert_eq!(managed(&dir).inner.fingerprints().len(), 2);

        // The account was linked after this device paired: the record moves to the
        // new owner instead of adding a second one.
        assert!(engine.inner.add_grant("new-account", "phone", A).is_ok());
        let moved = read_grants(&dir.join(GRANTS));
        assert_eq!(moved.len(), 2);
        assert_eq!(
            moved.iter().filter(|g| g.fingerprint == A).map(|g| g.owner.as_str()).collect::<Vec<_>>(),
            vec!["new-account"]
        );

        assert_eq!(engine.inner.forget_device("phone"), 2, "every owner's record goes");
        assert!(engine.inner.fingerprints().is_empty());
        assert!(read_grants(&dir.join(GRANTS)).is_empty(), "a removal is saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_grant_file_starts_empty_instead_of_failing_the_host() {
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(GRANTS), b"not json").unwrap();
        assert!(read_grants(&dir.join(GRANTS)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tunnel_addresses_are_ips_lan_first_and_never_invented() {
        assert_eq!(
            addresses(
                vec!["192.168.1.20".into(), "10.0.0.5".into()],
                Some("203.0.113.7".into())
            ),
            vec!["192.168.1.20", "10.0.0.5", "203.0.113.7"]
        );
        assert!(addresses(Vec::new(), None).is_empty(), "no address is better than a wrong one");
        assert_eq!(
            addresses(vec!["192.168.1.20".into()], Some("192.168.1.20".into())),
            vec!["192.168.1.20"],
            "the same address is listed once"
        );
        assert!(addresses(vec!["not-an-ip".into(), "".into()], Some("x".into())).is_empty());
    }

    #[test]
    fn the_bundled_engine_sits_under_the_host_data_folder() {
        let path = default_executable(Path::new(r"C:\ProgramData\Secpar"));
        assert_eq!(path, Path::new(r"C:\ProgramData\Secpar\engine\sunshine.exe"));
        assert_eq!(hex(&[0, 255]), "00ff");
    }

    #[test]
    fn the_engines_folder_is_copied_from_beside_the_app_on_first_run() {
        let base = temp();
        let app = base.join("app");
        let data = base.join("data");
        std::fs::create_dir_all(app.join("engine").join("assets")).unwrap();
        std::fs::write(app.join("engine").join(engine_file()), b"exe").unwrap();
        std::fs::write(app.join("engine").join("assets").join("theme.bin"), b"theme").unwrap();
        std::fs::write(app.join("engine").join("assets").join("runtime.dll"), b"dll").unwrap();

        assert!(stage_engine_from(&app, &data).unwrap(), "the first run copies it");
        assert_eq!(std::fs::read(default_executable(&data)).unwrap(), b"exe");
        assert_eq!(
            std::fs::read(data.join("engine").join("assets").join("theme.bin")).unwrap(),
            b"theme"
        );
        assert_eq!(
            std::fs::read(data.join("engine").join("assets").join("runtime.dll")).unwrap(),
            b"dll"
        );
        assert!(!stage_engine_from(&app, &data).unwrap(), "the second run leaves it alone");
        assert!(!data.join("engine.copy").exists(), "no half-written folder stays behind");

        // Nothing to copy, or an app folder that is a network path.
        assert!(!stage_engine_from(&base.join("missing"), &base.join("other")).unwrap());
        #[cfg(windows)]
        assert!(!stage_engine_from(Path::new(r"\\server\share"), &base.join("third")).unwrap());
        let _ = std::fs::remove_dir_all(&base);
    }
}
