//! The update path against a release feed on this computer: no GitHub, no
//! network. A real installer or AppImage can be used as the asset through
//! SECPAR_UPDATE_TEST_ASSET; otherwise a stand-in file is served.
use secpar_app::update::{self, Release};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::Arc,
};

/// Serves `feed` and `asset` on 127.0.0.1 until the test process ends.
fn serve(feed: String, asset: Vec<u8>, port: u16) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let feed = Arc::new(feed.into_bytes());
    let asset = Arc::new(asset);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (feed, asset) = (feed.clone(), asset.clone());
            std::thread::spawn(move || {
                let mut request = [0u8; 4096];
                let read = stream.read(&mut request).unwrap_or(0);
                let line = String::from_utf8_lossy(&request[..read]).lines().next().unwrap_or_default().to_string();
                let path = line.split(' ').nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
                let body: &[u8] = if path == "/releases" { &feed } else { &asset };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
            });
        }
    });
    port
}

/// The asset used for the test: the built package when one is named, else a
/// small stand-in with the same shape.
fn asset_bytes() -> Vec<u8> {
    match std::env::var_os("SECPAR_UPDATE_TEST_ASSET") {
        Some(path) => std::fs::read(PathBuf::from(path)).expect("SECPAR_UPDATE_TEST_ASSET is readable"),
        None => {
            let mut bytes = if cfg!(windows) { b"MZ".to_vec() } else { b"\x7fELF".to_vec() };
            bytes.extend(vec![0x5a; 4096]);
            bytes
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The feed a release server would answer for one asset.
fn feed_json(origin: &str, name: &str, bytes: &[u8], sha256: &str) -> String {
    json!([{
        "tag_name": "v9.9.9",
        "draft": false,
        "prerelease": false,
        "assets": [{
            "name": name,
            "browser_download_url": format!("{origin}/releases/download/v9.9.9/{name}"),
            "digest": format!("sha256:{sha256}"),
            "size": bytes.len(),
        }],
    }])
    .to_string()
}

fn release_for(port: u16, name: &str, bytes: &[u8]) -> Release {
    Release {
        tag: "v9.9.9".into(),
        version: "9.9.9".into(),
        url: format!("http://127.0.0.1:{port}/releases/download/v9.9.9/{name}"),
        sha256: digest(bytes),
        size: bytes.len() as u64,
    }
}

#[test]
fn a_feed_on_this_computer_is_checked_downloaded_checked_and_installed() {
    let bytes = asset_bytes();
    let name = update::asset_name().expect("this system has a package");
    let sha256 = digest(&bytes);
    println!("serving {} bytes of {} ({})", bytes.len(), name, sha256);
    // The feed has to name the port it is served on, so the port is reserved
    // first and the server then uses it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let origin = format!("http://127.0.0.1:{port}");
    let port = serve(feed_json(&origin, name, &bytes, &sha256), bytes.clone(), port);
    let origin = format!("http://127.0.0.1:{port}");

    // 1. The feed names a newer release than this copy.
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let found = runtime.block_on(update::check_at(&origin)).unwrap().expect("a newer release");
    assert_eq!(found.version, "9.9.9");
    assert_eq!(found.sha256, sha256, "the size and SHA-256 come from the feed");

    // 2. A release that is not newer is not offered.
    assert!(update::select(&json!([{
        "tag_name": format!("v{}", update::CURRENT),
        "draft": false, "prerelease": false,
        "assets": [{"name": name, "browser_download_url": format!("{origin}/releases/download/v{}/{name}", update::CURRENT), "digest": format!("sha256:{sha256}"), "size": bytes.len()}]
    }]), name, update::CURRENT, &origin).is_none());

    // 3. The download is written only when its size and SHA-256 match.
    let folder = tempfile::tempdir().unwrap();
    let file = runtime.block_on(update::download_at(folder.path(), &release_for(port, name, &bytes))).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), bytes, "the downloaded file is the published one");
    // With a real package named as the asset, that is what the updater keeps.
    if let Some(named) = std::env::var_os("SECPAR_UPDATE_TEST_ASSET") {
        let magic: &[u8] = if cfg!(windows) { b"MZ" } else { b"\x7fELF" };
        assert!(bytes.starts_with(magic), "the real package is a program for this system");
        eprintln!("downloaded {named:?}: {} bytes, sha256 {sha256}", bytes.len());
    }

    // 4. A wrong digest is refused, and nothing is left behind.
    let mut wrong = release_for(port, name, &bytes);
    wrong.sha256 = "0".repeat(64);
    let refused = runtime.block_on(update::download_at(folder.path(), &wrong));
    assert_eq!(refused.err().as_deref(), Some("checksum_mismatch"));
    let leftovers: Vec<_> = std::fs::read_dir(folder.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(update::asset_name().unwrap()))
        .collect();
    assert!(leftovers.is_empty(), "a refused download is removed, found {leftovers:?}");

    // 5. A wrong size is refused too.
    let mut short = release_for(port, name, &bytes);
    short.size += 1;
    assert_eq!(runtime.block_on(update::download_at(folder.path(), &short)).err().as_deref(), Some("checksum_mismatch"));
}

/// Linux: the update replaces the AppImage this app was started from, byte for
/// byte, keeps it executable, and leaves no half-written file beside it.
#[cfg(target_os = "linux")]
#[test]
fn installing_replaces_the_running_appimage() {
    use std::os::unix::fs::PermissionsExt;
    let bytes = asset_bytes();
    let name = update::asset_name().unwrap();
    let sha256 = digest(&bytes);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let origin = format!("http://127.0.0.1:{port}");
    let port = serve(feed_json(&origin, name, &bytes, &sha256), bytes.clone(), port);

    let folder = tempfile::tempdir().unwrap();
    let running = folder.path().join("Secpar-Linux-x64.AppImage");
    std::fs::write(&running, b"\x7fELF old version").unwrap();
    std::fs::set_permissions(&running, std::fs::Permissions::from_mode(0o700)).unwrap();

    // The updater reads these from the running app's environment.
    std::env::set_var("SECPAR_UPDATE_ORIGIN", format!("http://127.0.0.1:{port}"));
    std::env::set_var("APPIMAGE", &running);
    let installed = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(update::install());
    assert_eq!(installed.as_deref(), Ok("9.9.9"), "the newer version is installed");
    assert_eq!(std::fs::read(&running).unwrap(), bytes, "the file this copy ran from is replaced");
    assert_eq!(std::fs::metadata(&running).unwrap().permissions().mode() & 0o777, 0o755);
    let beside: Vec<_> = std::fs::read_dir(folder.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(beside.is_empty(), "no partial file is left beside the AppImage, found {beside:?}");
}
