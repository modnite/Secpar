//! Own a private upstream Sunshine process. This does not install a service or
//! modify another Sunshine/Apollo/Secpar instance. No media implementation lives here.
mod private;
mod tls;
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    time::{timeout, Instant},
};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Default)]
pub enum Capture {
    #[default]
    Automatic,
    X11,
}
#[derive(Clone, Copy, Default)]
pub enum Encoder {
    #[default]
    Automatic,
    Software,
}
pub struct Config {
    /// Trusted bundled executable, never a path obtained from a remote message.
    pub executable: PathBuf,
    /// An engine-owned subdirectory of Secpar's private platform state directory.
    pub state: PathBuf,
    pub base_port: u16,
    pub capture: Capture,
    pub encoder: Encoder,
    pub audio_sink: Option<String>,
}
impl Config {
    fn validate(&self) -> Result<()> {
        if !self.executable.is_absolute() || !self.executable.is_file() || !self.state.is_absolute()
        {
            return Err(Error("The bundled host or private state path is invalid"));
        }
        if !(1029..=65514).contains(&self.base_port) {
            return Err(Error("Host port range is invalid"));
        }
        for path in [&self.state, &self.executable] {
            config_value(path.to_str().ok_or(Error("Host path is not valid text"))?)?;
        }
        if let Some(sink) = &self.audio_sink {
            config_value(sink)?;
        }
        Ok(())
    }
    fn text(&self) -> Result<String> {
        self.validate()?;
        let mut text = format!(
            "sunshine_name = Secpar\nport = {}\norigin_web_ui_allowed = pc\n",
            self.base_port
        );
        for (key, file) in [
            ("file_state", "clients.json"),
            ("credentials_file", "management.json"),
            ("file_apps", "apps.json"),
            ("pkey", "host-key.pem"),
            ("cert", "host-cert.pem"),
            ("log_path", "host.log"),
        ] {
            text.push_str(&format!("{key} = {}\n", self.state.join(file).display()));
        }
        if matches!(self.capture, Capture::X11) {
            text.push_str("capture = x11\n");
        }
        if matches!(self.encoder, Encoder::Software) {
            text.push_str("encoder = software\nsw_preset = ultrafast\nsw_tune = zerolatency\n");
        }
        if let Some(sink) = &self.audio_sink {
            text.push_str(&format!("audio_sink = {sink}\n"));
        }
        Ok(text)
    }
}
fn config_value(value: &str) -> Result<()> {
    if value.is_empty()
        || value
            .chars()
            .any(|c| c.is_control() || matches!(c, '#' | '[' | ']'))
    {
        Err(Error("Host setting contains unsupported characters"))
    } else {
        Ok(())
    }
}

struct Password(String);
impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}
fn password(dir: &Path) -> Result<Password> {
    let path = dir.join("management.secret");
    if !path.exists() {
        let mut bytes = [0u8; 32];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| Error("Could not generate private host credentials"))?;
        let text = hex(&bytes);
        match private::create(&path, text.as_bytes()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(Error("Could not save private host credentials")),
        }
    }
    let bytes =
        private::read(&path, 64).map_err(|_| Error("Could not read private host credentials"))?;
    let text =
        String::from_utf8(bytes).map_err(|_| Error("Private host credentials are invalid"))?;
    if !is_hex(&text, 64) {
        return Err(Error("Private host credentials are invalid"));
    }
    Ok(Password(text))
}
fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The controller owns this child handle; stopping/dropping never searches for
/// process names or stops an installed service. State and certificates persist.
pub struct Host {
    child: Child,
    _lock: std::fs::File,
    api: Api,
    pub info: Info,
}
#[derive(Clone, Debug)]
pub struct Info {
    pub http_port: u16,
    pub https_port: u16,
    pub certificate_sha256: String,
    pub version: String,
}
impl Host {
    pub async fn start(config: Config) -> Result<Self> {
        config.validate()?;
        private::directory(&config.state)
            .map_err(|_| Error("Could not protect private host state"))?;
        for file in [
            "host.lock",
            "management.secret",
            "management.json",
            "clients.json",
            "apps.json",
            "host-key.pem",
            "host-cert.pem",
            "host.log",
        ] {
            private::ordinary_or_missing(&config.state.join(file))
                .map_err(|_| Error("Private host state contains a redirected file"))?;
        }
        let mut lock_options = std::fs::OpenOptions::new();
        lock_options
            .read(true)
            .write(true)
            .create(true)
            .truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            lock_options.mode(0o600);
        }
        let lock = lock_options
            .open(config.state.join("host.lock"))
            .map_err(|_| Error("Could not open private host lock"))?;
        lock.try_lock()
            .map_err(|_| Error("This private host state is already in use"))?;
        let secret = password(&config.state)?;
        let conf = config.state.join("sunshine.conf");
        // Generate files before spawn, not through a shell command. Each instance
        // must own a distinct state directory; the platform adapter enforces that.
        replace_private(&conf, config.text()?.as_bytes())?;
        let apps = config.state.join("apps.json");
        if !apps.exists() {
            private::create(
                &apps,
                br#"{"env":{},"apps":[{"name":"Desktop","image-path":""}]}"#,
            )
            .map_err(|_| Error("Could not create desktop stream entry"))?;
        }
        let mut setup = command(&config.executable);
        setup.arg(&conf).args(["--creds", "Secpar", &secret.0]);
        let mut setup = setup
            .spawn()
            .map_err(|_| Error("Could not initialize the bundled host"))?;
        match timeout(Duration::from_secs(10), setup.wait()).await {
            Ok(Ok(status)) if status.success() => {}
            _ => {
                let _ = setup.kill().await;
                return Err(Error("Bundled host credential initialization failed"));
            }
        }
        let mut child = command(&config.executable)
            .arg(&conf)
            .spawn()
            .map_err(|_| Error("Could not start the bundled host"))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut api = None;
        loop {
            if child
                .try_wait()
                .map_err(|_| Error("Could not read bundled host status"))?
                .is_some()
            {
                return Err(Error("Bundled host stopped before it became ready"));
            }
            if api.is_none() {
                if let Ok(pem) = private::certificate(&config.state.join("host-cert.pem")) {
                    api = Api::new(config.base_port + 1, &pem, &secret.0).ok();
                }
            }
            if let Some(api) = &api {
                if let Ok(value) = api.request(reqwest::Method::GET, "config", None).await {
                    if value["status"] == true {
                        let info = Info {
                            http_port: config.base_port,
                            https_port: config.base_port - 5,
                            certificate_sha256: api.fingerprint.clone(),
                            version: value["version"].as_str().unwrap_or("unknown").into(),
                        };
                        return Ok(Self {
                            child,
                            _lock: lock,
                            api: api.clone(),
                            info,
                        });
                    }
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill().await;
                return Err(Error("Bundled host did not become ready"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }
    pub async fn pending(&self) -> Result<Pending> {
        let value = self.api.request(reqwest::Method::GET, "pin", None).await?;
        let pending: Pending =
            serde_json::from_value(value).map_err(|_| Error("Host pairing response is invalid"))?;
        if pending.pairings.len() > 16 {
            return Err(Error("Host returned too many pending pairings"));
        }
        Ok(pending)
    }
    /// Approval requires an identity-binding-capable backend. Never fall back to
    /// selecting an untrusted device name or to feeding a global stdin PIN.
    pub async fn approve(&self, identity: &ClientIdentity, pin: &str, name: &str) -> Result<()> {
        identity.validate()?;
        if pin.len() != 4
            || !pin.bytes().all(|b| b.is_ascii_digit())
            || name.is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
        {
            return Err(Error("Pairing approval is invalid"));
        }
        let pending = self.pending().await?;
        if pending.secpar_identity_binding != 1 {
            return Err(Error("Bundled host does not support device-bound pairing"));
        }
        let matches: Vec<_> = pending
            .pairings
            .iter()
            .filter(|p| {
                p.client_id == identity.client_id
                    && p.certificate_sha256 == identity.certificate_sha256
            })
            .collect();
        if matches.len() != 1 {
            return Err(Error("The expected device has no unique pending pairing"));
        }
        let id = &matches[0].id;
        if !is_hex(id, 32) {
            return Err(Error("Host pairing identifier is invalid"));
        }
        let value=self.api.request(reqwest::Method::POST,"pin",Some(json!({"pairing_id":id,"pin":pin,"name":name,
            "expected_client_id":identity.client_id,"expected_certificate_sha256":identity.certificate_sha256}))).await?;
        if value["status"] != true {
            return Err(Error("Host rejected device pairing"));
        }
        Ok(())
    }
    pub async fn stop(mut self) -> Result<()> {
        if self
            .child
            .try_wait()
            .map_err(|_| Error("Could not read bundled host status"))?
            .is_none()
        {
            self.child
                .kill()
                .await
                .map_err(|_| Error("Could not stop the bundled host"))?;
        }
        Ok(())
    }
}
fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    command
}
fn replace_private(path: &Path, bytes: &[u8]) -> Result<()> {
    // A new, private sibling avoids following an existing symlink while writing.
    let mut nonce = [0u8; 16];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| Error("Could not prepare private host configuration"))?;
    let temp = path.with_extension(format!("{}.new", hex(&nonce)));
    private::create(&temp, bytes)
        .map_err(|_| Error("Could not prepare private host configuration"))?;
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path).map_err(|_| Error("Could not replace host configuration"))?;
    }
    std::fs::rename(temp, path).map_err(|_| Error("Could not save private host configuration"))
}

#[derive(Clone, Debug, Deserialize)]
pub struct Pending {
    #[serde(default)]
    pub secpar_identity_binding: u32,
    pub pairings: Vec<PendingClient>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct PendingClient {
    pub id: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub certificate_sha256: String,
}
#[derive(Clone)]
pub struct ClientIdentity {
    pub client_id: String,
    pub certificate_sha256: String,
}
impl ClientIdentity {
    fn validate(&self) -> Result<()> {
        if self.client_id.is_empty()
            || self.client_id.len() > 128
            || self.client_id.chars().any(char::is_control)
            || !is_hex(&self.certificate_sha256, 64)
        {
            Err(Error("Client certificate identity is invalid"))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone)]
struct Api {
    client: reqwest::Client,
    origin: String,
    password: String,
    fingerprint: String,
}
impl Api {
    fn new(port: u16, pem: &[u8], password: &str) -> Result<Self> {
        let text = std::str::from_utf8(pem).map_err(|_| Error("Host certificate is invalid"))?;
        let encoded: String = text
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .map(str::trim)
            .collect();
        let der = STANDARD
            .decode(encoded)
            .map_err(|_| Error("Host certificate is invalid"))?;
        let fingerprint: [u8; 32] = digest::digest(&digest::SHA256, &der)
            .as_ref()
            .try_into()
            .unwrap();
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(tls::config(fingerprint)?)
            .no_proxy()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| Error("Could not create private host control connection"))?;
        Ok(Self {
            client,
            origin: format!("https://127.0.0.1:{port}/api/"),
            password: password.into(),
            fingerprint: hex(&fingerprint),
        })
    }
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        if !matches!(path, "config" | "pin") {
            return Err(Error("Unknown private host operation"));
        }
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.origin))
            .basic_auth("Secpar", Some(&self.password));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| Error("Private host control connection failed"))?;
        if !response.status().is_success() {
            return Err(Error("Private host rejected the control request"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error("Private host response failed"))?
        {
            if bytes.len().saturating_add(chunk.len()) > 1024 * 1024 {
                return Err(Error("Private host response exceeds its limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error("Private host response is not valid JSON"))
    }
}

#[cfg(test)]
mod tests;
