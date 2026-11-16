//! Finding a newer release and installing it over this copy.
//!
//! The feed is the releases list on GitHub. An asset is used only when its name
//! is the one this operating system ships, its address is the release's own
//! download address, and GitHub gives its size and SHA-256. The download is
//! hashed as it is written and is refused unless both match.
//!
//! Windows: the verified installer is started silently and replaces this copy.
//! Linux: the AppImage this window was started from is replaced, byte for byte,
//! by renaming the new file over it, and the new copy is started.
use serde_json::Value;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub const CURRENT: &str = env!("SECPAR_VERSION");
const ORIGIN: &str = "https://github.com/modnite/Secpar";
const MAX_ASSET: u64 = 500_000_000;
const MAX_FEED: usize = 4_000_000;

/// One release this operating system can install.
#[derive(Clone, Debug)]
pub struct Release {
    /// The release's tag, for example `v4.0.1`.
    pub tag: String,
    /// The tag without its `v`.
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

/// The asset name for this operating system. These names are the ones the
/// published releases use, so the older app's updater keeps finding them.
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("Secpar-Windows-x64-setup.exe"),
        ("linux", "x86_64") => Some("Secpar-Linux-x64.AppImage"),
        _ => None,
    }
}

/// Where releases are read from and downloaded from.
///
/// A test build can point this at a feed on this computer only: any other
/// address is ignored, so the variable cannot be used to serve updates from
/// somewhere else.
pub fn origin() -> String {
    match std::env::var("SECPAR_UPDATE_ORIGIN") {
        Ok(value) if value.starts_with("http://127.0.0.1:") || value.starts_with("http://[::1]:") => value,
        _ => ORIGIN.into(),
    }
}

/// `v4.0.1` as numbers, and nothing else: a development or preview suffix does
/// not describe a release this app installs.
pub fn version(tag: &str) -> Option<[u32; 3]> {
    tag.strip_prefix('v')?;
    numbers(tag)
}

/// A version that may be written with or without its `v`, as the app's own
/// version is.
fn numbers(text: &str) -> Option<[u32; 3]> {
    let parts: Vec<&str> = text.strip_prefix('v').unwrap_or(text).split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut found = [0u32; 3];
    for (at, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        found[at] = part.parse().ok()?;
    }
    Some(found)
}

/// The newest usable release in a feed, if it is newer than `current`.
pub fn select(body: &Value, name: &str, current: &str, origin: &str) -> Option<Release> {
    let current = numbers(current)?;
    body.as_array()?
        .iter()
        .filter_map(|release| {
            if release["draft"].as_bool().unwrap_or(false) || release["prerelease"].as_bool().unwrap_or(false) {
                return None;
            }
            let tag = release["tag_name"].as_str()?;
            let newer = version(tag)?;
            if newer <= current {
                return None;
            }
            let asset = release["assets"]
                .as_array()?
                .iter()
                .find(|asset| asset["name"].as_str() == Some(name))?;
            let url = asset["browser_download_url"].as_str()?;
            // The file must come from the release's own download address.
            if url != format!("{origin}/releases/download/{tag}/{name}") {
                return None;
            }
            let sha256 = asset["digest"].as_str()?.strip_prefix("sha256:")?;
            if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let size = asset["size"].as_u64()?;
            if size == 0 || size > MAX_ASSET {
                return None;
            }
            Some(Release {
                tag: tag.into(),
                version: tag.trim_start_matches('v').to_string(),
                url: url.into(),
                sha256: sha256.to_ascii_lowercase(),
                size,
            })
        })
        .max_by(|a, b| version(&a.tag).cmp(&version(&b.tag)))
}

fn client() -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Secpar")
        .build()
        .map_err(|_| "unreachable".to_string())
}

/// GitHub answers a share of anonymous requests with 429 or 5xx in bad spells;
/// four tries with a growing pause ride those out.
async fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let client = client()?;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let outcome = client.get(url).send().await;
        let retry = match &outcome {
            Ok(response) => response.status().is_server_error() || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS,
            Err(_) => true,
        };
        if retry && attempt < 4 {
            tokio::time::sleep(Duration::from_secs(u64::from(attempt) * 2)).await;
            continue;
        }
        let mut response = outcome.and_then(|response| response.error_for_status()).map_err(|_| "unreachable".to_string())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "unreachable".to_string())? {
            if bytes.len() + chunk.len() > limit {
                return Err("unreachable".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
}

/// The newest release newer than this copy, read from `origin`.
pub async fn check_at(origin: &str) -> Result<Option<Release>, String> {
    let Some(name) = asset_name() else { return Ok(None) };
    let body = get(&format!("{origin}/releases?per_page=100"), MAX_FEED).await?;
    let feed: Value = serde_json::from_slice(&body).map_err(|_| "bad_answer".to_string())?;
    Ok(select(&feed, name, CURRENT, origin))
}

pub async fn check() -> Result<Option<Release>, String> {
    check_at(&origin()).await
}

/// Where a downloaded update waits until it is installed.
pub fn updates_folder() -> PathBuf {
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(not(target_os = "linux"))]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Secpar").join("updates")
}

fn prepare_folder(folder: &Path) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|_| "not_writable".to_string())?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::symlink_metadata(folder).map_err(|_| "not_writable".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("not_writable".into());
        }
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).map_err(|_| "not_writable".to_string())?;
    }
    Ok(())
}

/// Downloads, hashes and checks a release into `folder`. Nothing is left behind
/// unless the bytes match the size and the SHA-256 from the feed.
pub async fn download_at(folder: &Path, release: &Release) -> Result<PathBuf, String> {
    prepare_folder(folder)?;
    let name = asset_name().ok_or("not_supported")?;
    let path = folder.join(format!("Secpar-{}-{}", release.version, name));
    let _ = std::fs::remove_file(&path);
    let result = async {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "not_writable".to_string())?;
        let client = client()?;
        let mut response = client
            .get(&release.url)
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|_| "download_failed".to_string())?;
        let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
        let mut count = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|_| "download_failed".to_string())? {
            count += chunk.len() as u64;
            if count > release.size {
                return Err("checksum_mismatch".into());
            }
            hash.update(&chunk);
            file.write_all(&chunk).map_err(|_| "not_writable".to_string())?;
        }
        file.sync_all().map_err(|_| "not_writable".to_string())?;
        let digest: String = hash.finish().as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
        if count != release.size || digest != release.sha256 {
            return Err("checksum_mismatch".into());
        }
        Ok(path.clone())
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

pub async fn download(release: &Release) -> Result<PathBuf, String> {
    download_at(&updates_folder(), release).await
}

/// Replaces this copy with `file`. Windows starts the installer, which stops
/// this app's own background part, writes the files and starts the window
/// again. Nothing else on the computer is touched.
#[cfg(windows)]
fn apply(file: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const DETACHED: u32 = 0x0800_0000 | 0x0000_0200 | 0x0100_0000;
    // This window has to close before the installer can replace its files, so
    // the installer is started a moment later by a helper of its own.
    const SCRIPT: &str = "Start-Sleep -Seconds 2; Start-Process -FilePath $env:SECPAR_UPDATE -ArgumentList '/S','/RESTART'";
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", SCRIPT])
        .env("SECPAR_UPDATE", file)
        .creation_flags(DETACHED)
        .spawn()
        .map_err(|_| "install_failed".to_string())?;
    Ok(())
}

/// Replaces the AppImage this window was started from, keeping its permissions,
/// and starts the new copy once this one has ended.
#[cfg(target_os = "linux")]
fn apply(file: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let target = std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|path| path.is_absolute()).ok_or("not_appimage")?;
    let bytes = std::fs::read(file).map_err(|_| "install_failed".to_string())?;
    if bytes.len() < 8 || &bytes[..4] != b"\x7fELF" {
        return Err("install_failed".into());
    }
    let folder = target.parent().ok_or("not_writable")?;
    let beside = folder.join(format!(".{}.new-{}", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Secpar.AppImage".into()), std::process::id()));
    let write = || -> std::io::Result<()> {
        std::fs::write(&beside, &bytes)?;
        std::fs::set_permissions(&beside, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&beside, &target)
    };
    if write().is_err() {
        let _ = std::fs::remove_file(&beside);
        return Err("not_writable".into());
    }
    // The running AppImage stays mapped, so starting the new file is safe; it
    // waits a moment so this window is gone before it appears.
    std::process::Command::new("/bin/sh")
        .args(["-c", "sleep 1; exec \"$1\"", "secpar-update", &target.to_string_lossy()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "install_failed".to_string())?;
    Ok(())
}

/// Checks the feed again, downloads the newest release and installs it.
/// Returns the version that is now in place.
pub async fn install() -> Result<String, String> {
    let release = check().await?.ok_or("no_update")?;
    let file = download(&release).await?;
    apply(&file)?;
    Ok(release.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, name: &str, origin: &str) -> Value {
        json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": false,
            "assets": [{
                "name": name,
                "browser_download_url": format!("{origin}/releases/download/{tag}/{name}"),
                "digest": format!("sha256:{}", "ab".repeat(32)),
                "size": 3_500_000u64,
            }],
        })
    }

    #[test]
    fn takes_the_newest_release_with_the_asset_for_this_system() {
        let name = asset_name().unwrap();
        let origin = ORIGIN;
        let feed = json!([
            release("v4.0.1", name, origin),
            release("v4.1.0", name, origin),
            release("v4.2.0", "Secpar-4.2.0-Windows.zip", origin),
        ]);
        assert_eq!(select(&feed, name, "4.0.0", origin).unwrap().version, "4.1.0");
        // The same version, or an older one, is not an update.
        assert!(select(&feed, name, "4.1.0", origin).is_none());
        assert!(select(&feed, name, "4.2.0", origin).is_none());
    }

    #[test]
    fn refuses_releases_and_files_it_cannot_vouch_for() {
        let name = asset_name().unwrap();
        let origin = ORIGIN;
        let mut draft = release("v4.0.1", name, origin);
        draft["draft"] = json!(true);
        assert!(select(&json!([draft]), name, "4.0.0", origin).is_none());
        let mut preview = release("v4.0.1", name, origin);
        preview["prerelease"] = json!(true);
        assert!(select(&json!([preview]), name, "4.0.0", origin).is_none());

        let mut elsewhere = release("v4.0.1", name, origin);
        elsewhere["assets"][0]["browser_download_url"] = json!(format!("https://example.com/{name}"));
        assert!(select(&json!([elsewhere]), name, "4.0.0", origin).is_none());

        let mut no_digest = release("v4.0.1", name, origin);
        no_digest["assets"][0]["digest"] = Value::Null;
        assert!(select(&json!([no_digest]), name, "4.0.0", origin).is_none());

        let mut short_digest = release("v4.0.1", name, origin);
        short_digest["assets"][0]["digest"] = json!("sha256:abc");
        assert!(select(&json!([short_digest]), name, "4.0.0", origin).is_none());

        let mut huge = release("v4.0.1", name, origin);
        huge["assets"][0]["size"] = json!(MAX_ASSET + 1);
        assert!(select(&json!([huge]), name, "4.0.0", origin).is_none());

        // A development tag is not a release this app installs.
        let mut dev = release("v4.0.1-secpar-desktop-dev.5", name, origin);
        dev["assets"][0]["browser_download_url"] = json!(format!("{origin}/releases/download/v4.0.1-secpar-desktop-dev.5/{name}"));
        assert!(select(&json!([dev]), name, "4.0.0", origin).is_none());
    }

    #[test]
    fn versions_are_three_numbers_after_a_v() {
        assert_eq!(version("v4.0.0"), Some([4, 0, 0]));
        assert_eq!(version("v10.20.30"), Some([10, 20, 30]));
        for bad in ["4.0.0", "v4.0", "v4.0.0.1", "v4.0.x", "v4.0.0-rc1", ""] {
            assert_eq!(version(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_downloads_address_is_the_github_one_unless_a_test_feed_on_this_computer_is_set() {
        let saved = std::env::var_os("SECPAR_UPDATE_ORIGIN");
        std::env::remove_var("SECPAR_UPDATE_ORIGIN");
        assert_eq!(origin(), ORIGIN);
        // Anywhere but this computer is ignored.
        std::env::set_var("SECPAR_UPDATE_ORIGIN", "https://example.com");
        assert_eq!(origin(), ORIGIN);
        std::env::set_var("SECPAR_UPDATE_ORIGIN", "http://127.0.0.1:9");
        assert_eq!(origin(), "http://127.0.0.1:9");
        match saved {
            Some(value) => std::env::set_var("SECPAR_UPDATE_ORIGIN", value),
            None => std::env::remove_var("SECPAR_UPDATE_ORIGIN"),
        }
    }
}
