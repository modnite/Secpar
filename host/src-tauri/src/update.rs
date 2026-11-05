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
