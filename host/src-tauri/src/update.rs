//! Silent updates, run by the service: the latest GitHub release's installer, verified against the
//! minisign key built into this exe (`updater.pub`, made with `tauri signer generate`), then run with
//! `/S /UPDATE`. The installer stops the service, replaces the files and starts it again.

use serde_json::Value;
use std::path::Path;

const LATEST: &str = "https://api.github.com/repos/modnite/Secpar/releases/latest";
/// base64 of the minisign public key file.
const PUBKEY: &str = include_str!("../updater.pub");

pub async fn latest_release() -> Result<Option<Value>, String> {
    stable_release_from(LATEST).await
}

async fn stable_release_from(url: &str) -> Result<Option<Value>, String> {
    let mut result = reqwest::Client::builder().timeout(std::time::Duration::from_secs(12))
        .build().map_err(|_| "Could not start update check")?
        .get(url).header("User-Agent", "secpar-host")
        .header("Accept", "application/vnd.github+json").send().await
        .map_err(|_| "Could not reach GitHub")?;
    // GitHub's stable endpoint deliberately excludes development prereleases.
    // No stable release is a valid state, not a connectivity failure.
    if result.status() == reqwest::StatusCode::NOT_FOUND { return Ok(None); }
    if !result.status().is_success() { return Err("GitHub update check failed".into()); }
    let mut bytes = Vec::new();
    while let Some(chunk) = result.chunk().await.map_err(|_| "Incomplete update response")? {
        if bytes.len() + chunk.len() > 256 * 1024 { return Err("Invalid update response".into()); }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|_| "Invalid update response".into())
}

/// The release's version and the download URLs of its `…-setup.exe` and `…-setup.exe.sig`.
pub fn pick_assets(release: &Value) -> Option<(String, String, String)> {
    let version = release["tag_name"].as_str()?.trim_start_matches('v').to_string();
    let assets = release["assets"].as_array()?;
    let url = |suffix: &str| {
        assets.iter().find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(suffix)))?["browser_download_url"].as_str().map(String::from)
    };
    Some((version, url("-setup.exe")?, url("-setup.exe.sig")?))
}

/// `sig` is the `.sig` file Tauri writes (base64 of a minisign signature).
pub fn verify(data: &[u8], sig: &str, pubkey_b64: &str) -> bool {
    use base64::Engine;
    let b64 = |s: &str| base64::engine::general_purpose::STANDARD.decode(s.trim()).ok().and_then(|b| String::from_utf8(b).ok());
    let (Some(pk), Some(sig)) = (b64(pubkey_b64), b64(sig)) else { return false };
    let (Ok(pk), Ok(sig)) = (minisign_verify::PublicKey::decode(&pk), minisign_verify::Signature::decode(&sig)) else { return false };
    pk.verify(data, &sig, false).is_ok()
}

async fn download(url: &str) -> Result<Vec<u8>, String> {
    let r = reqwest::Client::new().get(url).header("User-Agent", "secpar-host").send().await.and_then(|r| r.error_for_status());
    Ok(r.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

/// Installs a newer release if there is one; returns its version once the installer started.
pub async fn check_and_install(dir: &Path) -> Result<Option<String>, String> {
    let Some(release) = latest_release().await? else { return Ok(None) };
    let Some((version, exe_url, sig_url)) = pick_assets(&release) else { return Ok(None) };
    if !crate::commands::is_newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let (exe, sig) = (download(&exe_url).await?, download(&sig_url).await?);
    if !verify(&exe, &String::from_utf8_lossy(&sig), PUBKEY) {
        return Err(format!("{version}: bad signature, not installing"));
    }
    let dir = dir.join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("PC-Remote-Host-{version}-setup.exe"));
    std::fs::write(&path, &exe).map_err(|e| e.to_string())?;
    use std::os::windows::process::CommandExt;
    const DETACHED: u32 = 0x0000_0008 | 0x0000_0200 | 0x0100_0000; // detached, new group, break away from job
    // silent; /UPDATE keeps shortcuts as the user left them
    std::process::Command::new(&path).args(["/S", "/UPDATE"]).creation_flags(DETACHED).spawn().map_err(|e| e.to_string())?;
    Ok(Some(version))
}

const FEED: &str = "https://api.github.com/repos/modnite/Secpar/releases?per_page=100";
const INSTALLER: &str = "Secpar-Windows-x64-setup.exe";
const MAX_INSTALLER: u64 = 100_000_000;

/// The installer a release offers, after every check this process insists on before it runs
/// the file as SYSTEM.
#[derive(Debug, PartialEq)]
pub struct Installer {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// The app asks for a release by tag. The tag is only a name: the feed is fetched here, and the
/// release must be a published, stable, newer one with an installer at the exact download address
/// and a SHA-256 digest from GitHub.
pub fn vet(releases: &Value, tag: &str, current: &str) -> Result<Installer, String> {
    let release = releases
        .as_array()
        .and_then(|all| all.iter().find(|r| r["tag_name"].as_str() == Some(tag)))
        .ok_or("That release is not in the feed")?;
    if release["draft"].as_bool().unwrap_or(false) || release["prerelease"].as_bool().unwrap_or(false) {
        return Err("That release is not a stable one".into());
    }
    if !crate::commands::is_newer(tag, current) {
        return Err("That release is not newer than this one".into());
    }
    let asset = release["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["name"].as_str() == Some(INSTALLER)))
        .ok_or("That release has no Windows installer")?;
    let url = format!("https://github.com/modnite/Secpar/releases/download/{tag}/{INSTALLER}");
    if asset["browser_download_url"].as_str() != Some(url.as_str()) {
        return Err("The installer address is not the expected one".into());
    }
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or("GitHub gave no checksum for the installer")?
        .to_ascii_lowercase();
    let size = asset["size"].as_u64().filter(|s| (1..=MAX_INSTALLER).contains(s)).ok_or("The installer size is not plausible")?;
    Ok(Installer { url, sha256, size })
}

/// GitHub answers a share of anonymous requests with 429 or 5xx in bad spells; four tries with a
/// growing pause ride those out.
async fn fetch(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build().map_err(|_| "Could not start the download")?;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let outcome = client.get(url).header("User-Agent", "secpar-host").header("Accept", "application/vnd.github+json").send().await;
        let retry = match &outcome {
            Ok(r) => r.status().is_server_error() || r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS,
            Err(_) => true,
        };
        if retry && attempt < 4 {
            tokio::time::sleep(std::time::Duration::from_secs(2 * u64::from(attempt))).await;
            continue;
        }
        let mut response = outcome.and_then(|r| r.error_for_status()).map_err(|_| "Could not download the update")?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "The download was interrupted")? {
            if bytes.len() as u64 + chunk.len() as u64 > limit {
                return Err("The download is larger than expected".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
}

static INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Downloads release `tag`, checks it against the feed's digest and starts its installer silently.
/// The installer stops this service and the app, replaces the files and starts the service again.
/// Returns once the installer is running.
pub async fn install_tag(dir: &Path, tag: &str) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("An update is already being installed".into());
    }
    let result = install_inner(dir, tag).await;
    if result.is_err() {
        INSTALLING.store(false, Ordering::SeqCst);
    }
    result
}

async fn install_inner(dir: &Path, tag: &str) -> Result<String, String> {
    let feed: Value = serde_json::from_slice(&fetch(FEED, 4_000_000).await?).map_err(|_| "Invalid update response")?;
    let wanted = vet(&feed, tag, env!("CARGO_PKG_VERSION"))?;
    let exe = fetch(&wanted.url, wanted.size).await?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &exe);
    let hex: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    if exe.len() as u64 != wanted.size || hex != wanted.sha256 {
        return Err("The download did not match its checksum. Nothing was installed.".into());
    }
    let dir = dir.join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(INSTALLER);
    std::fs::write(&path, &exe).map_err(|e| e.to_string())?;
    use std::os::windows::process::CommandExt;
    const DETACHED: u32 = 0x0000_0008 | 0x0000_0200 | 0x0100_0000; // detached, new group, break away from job
    // silent; /UPDATE keeps shortcuts as the user left them
    std::process::Command::new(&path).args(["/S", "/UPDATE"]).creation_flags(DETACHED).spawn().map_err(|e| e.to_string())?;
    Ok(tag.trim_start_matches('v').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn no_stable_release_is_not_a_network_failure() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({"message":"Not Found"}))).mount(&server).await;
        assert!(stable_release_from(&server.uri()).await.unwrap().is_none());
    }


    fn feed(tag: &str) -> Value {
        json!([{"tag_name": tag, "draft": false, "prerelease": false, "assets": [
            {"name": INSTALLER, "browser_download_url": format!("https://github.com/modnite/Secpar/releases/download/{tag}/{INSTALLER}"),
             "digest": format!("sha256:{}", "ab".repeat(32)), "size": 3_500_000}
        ]}])
    }

    #[test]
    fn vets_a_newer_stable_release() {
        let got = vet(&feed("v3.16.1"), "v3.16.1", "3.16.0").unwrap();
        assert_eq!(got.sha256, "ab".repeat(32));
        assert_eq!(got.url, "https://github.com/modnite/Secpar/releases/download/v3.16.1/Secpar-Windows-x64-setup.exe");
    }

    #[test]
    fn refuses_what_it_should_not_run_as_system() {
        assert!(vet(&feed("v3.16.0"), "v3.16.0", "3.16.0").is_err(), "same version");
        assert!(vet(&feed("v3.15.0"), "v3.15.0", "3.16.0").is_err(), "downgrade");
        assert!(vet(&feed("v3.16.1"), "v3.17.0", "3.16.0").is_err(), "tag not in the feed");
        let mut draft = feed("v3.16.1");
        draft[0]["draft"] = json!(true);
        assert!(vet(&draft, "v3.16.1", "3.16.0").is_err());
        let mut pre = feed("v3.16.1");
        pre[0]["prerelease"] = json!(true);
        assert!(vet(&pre, "v3.16.1", "3.16.0").is_err());
        let mut elsewhere = feed("v3.16.1");
        elsewhere[0]["assets"][0]["browser_download_url"] = json!("https://example.com/Secpar-Windows-x64-setup.exe");
        assert!(vet(&elsewhere, "v3.16.1", "3.16.0").is_err());
        let mut nosum = feed("v3.16.1");
        nosum[0]["assets"][0]["digest"] = json!(null);
        assert!(vet(&nosum, "v3.16.1", "3.16.0").is_err());
        let mut huge = feed("v3.16.1");
        huge[0]["assets"][0]["size"] = json!(900_000_000u64);
        assert!(vet(&huge, "v3.16.1", "3.16.0").is_err());
    }

    #[test]
    fn picks_installer_and_signature() {
        let r = json!({"tag_name": "v3.1.0", "assets": [
            {"name": "PC-Remote-3.1.0.apk", "browser_download_url": "https://x/apk"},
            {"name": "PC-Remote-Host-3.1.0-setup.exe.sig", "browser_download_url": "https://x/sig"},
            {"name": "PC-Remote-Host-3.1.0-setup.exe", "browser_download_url": "https://x/exe"},
        ]});
        assert_eq!(pick_assets(&r), Some(("3.1.0".into(), "https://x/exe".into(), "https://x/sig".into())));
        assert_eq!(pick_assets(&json!({"tag_name": "v1", "assets": []})), None);
    }

    #[test]
    fn signature_must_match_the_built_in_key() {
        // A real signature over b"hello" made with a throwaway key (tauri signer sign)
        const PK: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYzRkZDMTQ1Q0I5QjY2NTcKUldSWFpwdkxSY0gvWTREOHhpUjN3aWFKS1R6VDZpUUNZakJ6WXV2OTJqNTBYK3FkTTJ0dWV2T1oK";
        const SIG: &str = include_str!("../tests/hello.sig");
        assert!(verify(b"hello", SIG, PK));
        assert!(!verify(b"hellO", SIG, PK));
        assert!(!verify(b"hello", SIG, PUBKEY), "a different key must not verify");
        assert!(!verify(b"hello", "garbage", PK));
    }
}

#[cfg(test)]
mod release_check {
    /// `PCR_CHECK_EXE=… cargo test release_signature -- --ignored`: the built installer verifies with the built-in key.
    #[test]
    #[ignore]
    fn release_signature() {
        let exe = std::env::var("PCR_CHECK_EXE").unwrap();
        let data = std::fs::read(&exe).unwrap();
        let sig = std::fs::read_to_string(format!("{exe}.sig")).unwrap();
        assert!(super::verify(&data, &sig, super::PUBKEY));
    }
}
