//! Starting the stream component once the tunnel to a computer is open.
//!
//! The component only ever talks to the tunnel's local end. It keeps a separate
//! profile per hosting computer, in Secpar's own data folder, with its own
//! identity towards that computer's engine. The first time, it pairs: the
//! component starts the pairing and the hosting computer completes it when asked
//! through the tunnel, so nobody types a PIN.
use base64::{engine::general_purpose::STANDARD, Engine};
use secpar_service::channel;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{process::Child, sync::Mutex};

pub struct Settings {
    /// The stream component's executable, from Secpar's own installation.
    pub component: PathBuf,
    /// Secpar's data folder for this person.
    pub data: PathBuf,
}
/// The running stream, if any.
pub type Running = Arc<Mutex<Option<Child>>>;

fn profile(settings: &Settings, host: &str) -> PathBuf {
    settings.data.join("stream").join(host)
}
fn command(settings: &Settings, folder: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(&settings.component);
    // `portable.dat` in the working folder makes the component keep everything there.
    command.current_dir(folder).stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // no console window
    command
}
fn log(folder: &Path, name: &str) -> (Stdio, Stdio) {
    match std::fs::File::create(folder.join(name)).and_then(|f| Ok((f.try_clone()?, f))) {
        Ok((out, err)) => (out.into(), err.into()),
        Err(_) => (Stdio::null(), Stdio::null()),
    }
}

/// The component's settings live in its own folder. Qt writes `.ini` for the
/// explicit INI format the portable marker selects; on Linux an older build used
/// the native `.conf` suffix. Both are read so a profile survives either.
fn settings_text(folder: &Path) -> Option<String> {
    ["SecparStream.ini", "SecparStream.conf"].iter().find_map(|name| std::fs::read_to_string(folder.join("Secpar").join(name)).ok())
}

/// The identity the component made for itself, read from its own settings.
fn identity(folder: &Path) -> Option<(String, String)> {
    let text = settings_text(folder)?;
    let value = |key: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_owned));
    let id = value("uniqueid")?;
    let pem = value("certificate")?.replace("\\n", "\n");
    let body: String = pem.split("-----").nth(2)?.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')).collect();
    let id = id.trim_matches('"').trim_start_matches("@ByteArray(").trim_end_matches(')').to_string();
    let certificate = STANDARD.decode(body).ok()?;
    let digest: String = ring::digest::digest(&ring::digest::SHA256, &certificate).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    Some((id, digest))
}

enum Paired {
    Yes,
    /// The hosting computer took this profile's identity away earlier.
    Revoked,
    No(&'static str),
}
async fn pair(settings: &Settings, folder: &Path, address: &str, background: &str) -> Paired {
    // The PIN only travels between the component and the tunnel on this computer.
    let pin = format!("{:04}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() % 10_000).unwrap_or(0));
    let (out, err) = log(folder, "pair.log");
    let Ok(mut child) = command(settings, folder).args(["pair", address, "--pin", &pin]).env("SECPAR_MANAGED_PAIRING", "1").stdout(out).stderr(err).spawn() else {
        return Paired::No("stream_component_missing");
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    let (client_id, certificate) = loop {
        if let Some(found) = identity(folder) {
            break found;
        }
        if Instant::now() > deadline || child.try_wait().map(|s| s.is_some()).unwrap_or(true) {
            return Paired::No("stream_component_failed");
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    };
    // The host refuses anything that is not the exact identity it will see.
    if client_id.is_empty() || client_id.len() > 128 || client_id.chars().any(char::is_control)
        || certificate.len() != 64 || !certificate.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Paired::No("stream_component_failed");
    }
    let request = json!({"do": "pair", "pairing": {"client_id": client_id, "certificate_sha256": certificate, "pin": pin}});
    match channel::ask(background, &request).await.map(|a| a["answer"].clone()) {
        Some(answer) if answer == "paired" => {}
        Some(answer) if answer == "revoked" => return Paired::Revoked,
        _ => return Paired::No("pairing_refused"),
    }
    match tokio::time::timeout(Duration::from_secs(45), child.wait()).await {
        Ok(Ok(status)) if status.success() => Paired::Yes,
        _ => Paired::No("pairing_failed"),
    }
}

/// Pairs if this computer has not paired with `host` before, then starts the
/// stream. `address` is the tunnel's local end and `background` the background
/// part's channel.
pub async fn start(settings: &Settings, host: &str, address: &str, background: &str, running: &Running) -> Result<(), &'static str> {
    if !settings.component.is_file() || address.is_empty() {
        return Err("stream_component_missing");
    }
    // One stream per window: a new Connect replaces the old stream.
    stop(running).await;
    let mut folder = profile(settings, host);
    let marker = |folder: &Path| folder.join("paired");
    for attempt in 0..2 {
        std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(folder.join("portable.dat"), b"")).map_err(|_| "data_folder_unusable")?;
        if marker(&folder).exists() {
            break;
        }
        match pair(settings, &folder, address, background).await {
            Paired::Yes => {
                let _ = std::fs::write(marker(&folder), b"");
                break;
            }
            // Start over once with a new identity.
            Paired::Revoked if attempt == 0 => {
                let _ = std::fs::remove_dir_all(&folder);
                folder = profile(settings, host);
            }
            Paired::Revoked => return Err("pairing_refused"),
            Paired::No(reason) => return Err(reason),
        }
    }
    let (out, err) = log(&folder, "stream.log");
    let child = command(settings, &folder)
        .args(["stream", address, "Desktop", "--packet-size", "1024", "--display-mode", "windowed", "--absolute-mouse", "--keep-awake", "--frame-pacing", "--capture-system-keys", "fullscreen", "--no-mute-on-focus-loss"])
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|_| "stream_component_missing")?;
    *running.lock().await = Some(child);
    Ok(())
}

/// Ends the stream when the tunnel is gone, and the tunnel when the stream
/// window was closed. A stream that ends within a few seconds of starting on an
/// old pairing means the hosting computer no longer knows it: the pairing mark
/// is removed so the next Connect pairs again.
pub async fn watch(settings: Arc<Settings>, host: String, background: String, running: Running) {
    let started = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut slot = running.lock().await;
        let Some(child) = slot.as_mut() else { return };
        if let Ok(Some(status)) = child.try_wait() {
            *slot = None;
            drop(slot);
            if !status.success() && started.elapsed() < Duration::from_secs(8) {
                let _ = std::fs::remove_file(profile(&settings, &host).join("paired"));
            }
            channel::ask(&background, &json!({"do": "disconnect"})).await;
            return;
        }
        drop(slot);
        let connected = channel::ask(&background, &json!({"do": "status"})).await.is_some_and(|s| s["connected_to_host"] == true);
        if !connected {
            if let Some(mut child) = running.lock().await.take() {
                let _ = child.kill().await;
            }
            return;
        }
    }
}

/// Stops the stream, if one is running.
pub async fn stop(running: &Running) {
    if let Some(mut child) = running.lock().await.take() {
        let _ = child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The shape Qt writes for a QByteArray in portable mode, including its literal
    // newline escapes. The DER body is arbitrary because only the bytes are hashed.
    const BODY: &str = "MIIBvTCCASagAwIBAgIBADANBgkqhkiG9w0BAQsFADAjMSEwHwYDVQQDDBhOVklESUEgR2FtZVN0cmVhbSBDbGllbnQwHhcNMjYxMTE0MjExMDUyWhcNNDYxMTA5MjExMDUyWjAjMSEwHwYDVQQDDBhOVklESUEgR2FtZVN0cmVhbSBDbGllbnQ=";
    fn settings(folder: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(folder.join("Secpar")).unwrap();
        let pem = format!("-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n");
        // QSettings stores the line breaks as the two characters backslash and n.
        let escaped = pem.replace('\n', "\\n");
        let text = format!("[General]\ncertificate=\"@ByteArray({escaped})\"\nuniqueid=0123456789ABCDEF\n");
        std::fs::write(folder.join("Secpar").join(name), text).unwrap();
    }
    fn digest() -> String {
        let bytes = STANDARD.decode(BODY).unwrap();
        ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn reads_the_identity_from_qt_ini_settings() {
        let folder = tempfile::tempdir().unwrap();
        settings(folder.path(), "SecparStream.ini", BODY);
        assert_eq!(identity(folder.path()), Some(("0123456789ABCDEF".into(), digest())));
    }

    #[test]
    fn reads_an_older_conf_settings_file() {
        let folder = tempfile::tempdir().unwrap();
        settings(folder.path(), "SecparStream.conf", BODY);
        assert_eq!(identity(folder.path()), Some(("0123456789ABCDEF".into(), digest())));
    }

    #[test]
    fn an_unreadable_or_incomplete_profile_has_no_identity() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(identity(folder.path()), None);
        settings(folder.path(), "SecparStream.ini", BODY);
        std::fs::write(folder.path().join("Secpar").join("SecparStream.ini"), "[General]\nuniqueid=1\n").unwrap();
        assert_eq!(identity(folder.path()), None);
    }

    #[test]
    fn a_missing_component_is_reported_before_any_tunnel_use() {
        let folder = tempfile::tempdir().unwrap();
        let settings = Settings { component: folder.path().join("absent"), data: folder.path().join("data") };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let running: Running = Default::default();
        let result = runtime.block_on(start(&settings, "host", "127.0.0.2:46989", "channel", &running));
        assert_eq!(result, Err("stream_component_missing"));
        assert!(!folder.path().join("data").exists());
    }
}
