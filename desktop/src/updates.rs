//! Product-specific update discovery and user-approved installation.
use eframe::egui;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
const FEED: &str = "https://api.github.com/repos/modnite/Secpar/releases?per_page=100";
pub const CURRENT: &str = env!("SECPAR_RELEASE_TAG");
#[derive(Clone, Debug)]
struct Release {
    tag: String,
    url: String,
    hash: String,
    size: u64,
}
#[derive(Eq, PartialEq, Ord, PartialOrd, Debug)]
struct Version([u32; 3], bool, u32);
fn version(tag: &str) -> Option<Version> {
    let text = tag.strip_prefix('v')?;
    let (base, preview) = if let Some((base, n)) = text.split_once("-secpar-desktop-dev.") {
        (base, Some(n.parse::<u32>().ok()?))
    } else {
        (text, None)
    };
    let base: Vec<u32> = base
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    Some(Version(
        base.try_into().ok()?,
        preview.is_none(),
        preview.unwrap_or(0),
    ))
}
fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("Secpar-Windows-x64-setup.exe"),
        // Reserved for unified packages, never select the old client-only archives.
        ("linux", "x86_64") => Some("Secpar-Linux-x64.AppImage"),
        ("macos", "x86_64") => Some("Secpar-macOS-x64.pkg"),
        ("macos", "aarch64") => Some("Secpar-macOS-arm64.pkg"),
        _ => None,
    }
}
fn select(body: &Value, name: &str, current: &str) -> Option<Release> {
    let current = version(current)?;
    body.as_array()?
        .iter()
        .filter_map(|r| {
            if r["draft"].as_bool().unwrap_or(false) {
                return None;
            }
            let tag = r["tag_name"].as_str()?;
            let v = version(tag)?;
            if v <= current || (current.1 && !v.1) {
                return None;
            }
            let a = r["assets"]
                .as_array()?
                .iter()
                .find(|a| a["name"].as_str() == Some(name))?;
            let url = a["browser_download_url"].as_str()?;
            if url != format!("https://github.com/modnite/Secpar/releases/download/{tag}/{name}") {
                return None;
            }
            let hash = a["digest"].as_str()?.strip_prefix("sha256:")?;
            if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            let size = a["size"].as_u64()?;
            if size == 0 || size > 500_000_000 {
                return None;
            }
            Some((
                v,
                Release {
                    tag: tag.into(),
                    url: url.into(),
                    hash: hash.to_ascii_lowercase(),
                    size,
                },
            ))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r)
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .user_agent("Secpar-updater")
        .build()
        .map_err(|_| "Could not start updater".into())
}
async fn check() -> Result<Option<Release>, String> {
    let Some(name) = asset_name() else {
        return Ok(None);
    };
    let mut response = client()?
        .get(FEED)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| "Could not check for updates. Try again.")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Incomplete update response")?
    {
        if bytes.len() + chunk.len() > 4_000_000 {
            return Err("Update response too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let releases = serde_json::from_slice(&bytes).map_err(|_| "Invalid update response")?;
    Ok(select(&releases, name, CURRENT))
}
async fn download(r: Release) -> Result<PathBuf, String> {
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Secpar/updates");
    std::fs::create_dir_all(&directory).map_err(|_| "Could not create update folder")?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let suffix = if cfg!(windows) {
        "exe"
    } else if cfg!(target_os = "macos") {
        "pkg"
    } else {
        "AppImage"
    };
    let path = directory.join(format!("Secpar-{}-{nonce}.{suffix}", r.tag));
    let result = async {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "Could not save update")?;
        let mut response = client()?
            .get(&r.url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|_| "Download failed")?;
        let mut hash = Sha256::new();
        let mut count = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|_| "Download interrupted")? {
            count += chunk.len() as u64;
            if count > r.size {
                return Err("Unexpected download size".into());
            }
            hash.update(&chunk);
            file.write_all(&chunk)
                .map_err(|_| "Could not save update")?;
        }
        file.sync_all().map_err(|_| "Could not save update")?;
        if count != r.size || format!("{:x}", hash.finalize()) != r.hash {
            return Err("Download verification failed. Nothing was installed.".into());
        }
        Ok(path.clone())
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}
enum ResultEvent {
    Checked(Result<Option<Release>, String>),
    Downloaded(Result<PathBuf, String>),
}
pub struct Panel {
    tx: mpsc::UnboundedSender<ResultEvent>,
    rx: mpsc::UnboundedReceiver<ResultEvent>,
    release: Option<Release>,
    ready: Option<PathBuf>,
    busy: bool,
    message: String,
    pub open: bool,
    last_check: Option<Instant>,
}
impl Panel {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut panel = Self {
            tx,
            rx,
            release: None,
            ready: None,
            busy: false,
            message: "Automatic checks every six hours.".into(),
            open: false,
            last_check: None,
        };
        if cfg!(feature = "ui-review")
            && std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p.starts_with("updates-"))
        {
            panel.open = true;
            if std::env::var("SECPAR_REVIEW_PAGE").is_ok_and(|p| p == "updates-none-sample") {
                panel.message = "You're up to date.".into();
            } else {
                panel.message = "Development 8 is available.".into();
                panel.release = Some(Release {
                    tag: "v0.1.0-secpar-desktop-dev.8".into(),
                    url: String::new(),
                    hash: String::new(),
                    size: 100,
                });
            }
        }
        panel
    }
    fn start_check(&mut self, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.message = "Checking for updates…".into();
        self.last_check = Some(Instant::now());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "Could not start updater".into())
                .and_then(|rt| rt.block_on(check()));
            let _ = tx.send(ResultEvent::Checked(result));
            ctx.request_repaint();
        });
    }
    pub fn tick(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            self.busy = false;
            match event {
                ResultEvent::Checked(Ok(release)) => {
                    self.message = release
                        .as_ref()
                        .map(|r| format!("{} is available.", r.tag))
                        .unwrap_or_else(|| "You’re up to date.".into());
                    if self.release.as_ref().map(|r| &r.tag) != release.as_ref().map(|r| &r.tag) {
                        self.ready = None;
                    }
                    self.release = release;
                }
                ResultEvent::Downloaded(Ok(path)) => {
                    self.ready = Some(path);
                    self.message = "Download verified. Ready to install.".into();
                }
                ResultEvent::Checked(Err(e)) | ResultEvent::Downloaded(Err(e)) => self.message = e,
            }
        }
        ctx.request_repaint_after(Duration::from_secs(60));
        if !cfg!(feature = "ui-review")
            && self
                .last_check
                .is_none_or(|t| t.elapsed() > Duration::from_secs(6 * 3600))
        {
            self.start_check(ctx);
        }
    }
    pub fn button(&mut self, ui: &mut egui::Ui) {
        if ui
            .button(if self.release.is_some() {
                "Update available"
            } else {
                "Updates"
            })
            .clicked()
        {
            self.open = true;
        }
    }
    pub fn window(&mut self, ctx: &egui::Context, streaming: bool) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Secpar updates").open(&mut open).collapsible(false)
            .resizable(false).default_width(350.)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!("Installed: {CURRENT}"));
                ui.add_space(8.);
                ui.label(&self.message);
                if self.busy { ui.spinner(); }
                if self.release.is_some() {
                    ui.add_space(8.);
                    ui.label("Installing restarts Secpar and its hosting service. Any connected devices will disconnect. Make sure you have another way back to this PC.");
                    if streaming { ui.label("Disconnect your current stream before installing."); }
                }
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.busy, egui::Button::new("Check for updates")).clicked() {
                        self.start_check(ctx);
                    }
                    let can_download = !self.busy && self.release.is_some() && self.ready.is_none();
                    if ui.add_enabled(can_download, egui::Button::new("Download update")).clicked() {
                        self.busy = true;
                        self.message = "Downloading update...".into();
                        let r = self.release.clone().unwrap();
                        let tx = self.tx.clone();
                        let ctx = ctx.clone();
                        std::thread::spawn(move || {
                            let result = tokio::runtime::Builder::new_current_thread().enable_all().build()
                                .map_err(|_| "Could not start updater".into()).and_then(|rt| rt.block_on(download(r)));
                            let _ = tx.send(ResultEvent::Downloaded(result));
                            ctx.request_repaint();
                        });
                    }
                });
                if let Some(path) = self.ready.clone() {
                    ui.add_space(8.);
                    if ui.add_enabled(!self.busy && !streaming, egui::Button::new("Install update...")).clicked() {
                        self.message = match open::that(&path) {
                            Ok(()) => "Installer opened. Approve its system prompt to continue.".into(),
                            Err(_) => "Could not open installer. Try again.".into(),
                        };
                    }
                }
            });
        self.open = open;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn release(tag: &str, name: &str) -> Value {
        json!({"tag_name":tag,"assets":[{"name":name,"browser_download_url":format!("https://github.com/modnite/Secpar/releases/download/{tag}/{name}"),"digest":format!("sha256:{}","a".repeat(64)),"size":100}]})
    }
    #[test]
    fn filters_product_platform_and_channel() {
        let name = "Secpar-Windows-x64-setup.exe";
        let releases = json!([
            release("v0.1.0-secpar-desktop-dev.15", name),
            release("v0.1.0-secpar-desktop-dev.7", name),
            release("v3.0.3-secpar-host-preview.99", name),
            release("v0.1.0-secpar-desktop-dev.99", "client.exe")
        ]);
        assert_eq!(
            select(&releases, name, "v0.1.0-secpar-desktop-dev.6")
                .unwrap()
                .tag,
            "v0.1.0-secpar-desktop-dev.15"
        );
        assert!(select(&releases, name, "v0.1.0").is_none());
    }
    #[test]
    fn rejects_untrusted_or_unverified_assets() {
        let name = "Secpar-Windows-x64-setup.exe";
        let mut r = release("v0.1.0-secpar-desktop-dev.7", name);
        r["assets"][0]["browser_download_url"] = json!("https://evil.example/installer.exe");
        assert!(select(&json!([r]), name, "v0.1.0-secpar-desktop-dev.6").is_none());
        let mut r = release("v0.1.0-secpar-desktop-dev.7", name);
        r["assets"][0]["digest"] = Value::Null;
        assert!(select(&json!([r]), name, "v0.1.0-secpar-desktop-dev.6").is_none());
    }
    #[test]
    fn stable_beats_same_version_preview() {
        assert!(version("v0.1.0") > version("v0.1.0-secpar-desktop-dev.999"));
    }
    #[tokio::test]
    async fn downloaded_bytes_must_match_length_and_hash() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _ = rustls::crypto::ring::default_provider().install_default();
        for valid in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/test", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 4096];
                let _ = socket.read(&mut request).await;
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                    )
                    .await
                    .unwrap();
            });
            let release = Release {
                tag: "test".into(),
                url,
                hash: if valid {
                    format!("{:x}", Sha256::digest(b"hello"))
                } else {
                    "0".repeat(64)
                },
                size: 5,
            };
            let result = download(release).await;
            server.await.unwrap();
            if valid {
                let path = result.unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), b"hello");
                std::fs::remove_file(path).unwrap();
            } else {
                assert!(result.is_err());
            }
        }
    }
}
