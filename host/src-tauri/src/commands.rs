//! The window's commands, served by the server over the pipe, plus their pure helpers.

use crate::server::App;
use crate::store::Config;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::Serialize;
use serde_json::Value;
use std::net::Ipv4Addr;
use std::sync::Arc;

const NAME_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
pub const PORT: u16 = 48999;

pub fn qr_payload(name: &str, ips: &[String], port: u16, fp: &str, otp: &str) -> String {
    format!(
        "secpar://pair?n={}&h={}&p={port}&fp={fp}&t={otp}",
        utf8_percent_encode(name, NAME_SET),
        ips.join(",")
    )
}

/// The most addresses a pairing link carries. The client refuses a link with more.
pub const MAX_ADDRESSES: usize = 16;

/// LAN IPv4s, best first: private ranges, then adapters with a gateway. Loopback and link-local dropped.
/// At most [`MAX_ADDRESSES`], each once.
pub fn rank_ips(mut ips: Vec<(Ipv4Addr, bool)>) -> Vec<String> {
    ips.retain(|(ip, _)| !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified());
    ips.sort_by_key(|(ip, gw)| (!ip.is_private(), !gw));
    let mut seen = std::collections::HashSet::new();
    ips.into_iter().filter(|(ip, _)| seen.insert(*ip)).map(|(ip, _)| ip.to_string()).take(MAX_ADDRESSES).collect()
}

fn version(v: &str) -> Option<Vec<u64>> {
    v.trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((version(latest), version(current)), (Some(l), Some(c)) if l > c)
}

#[derive(Serialize)]
pub struct DeviceView {
    id: String,
    name: String,
    connected: bool,
}

#[derive(Serialize)]
pub struct StateView {
    name: String,
    devices: Vec<DeviceView>,
    locked: bool,
}

fn state(app: &App) -> StateView {
    let connected = app.agent.connected();
    let devices = app
        .store
        .lock()
        .unwrap()
        .devices()
        .into_iter()
        .map(|d| DeviceView { connected: connected.contains(&d.id), id: d.id, name: d.name })
        .collect();
    StateView { name: crate::win::sys::pc_name(), devices, locked: crate::win::desk::is_locked() }
}

/// A fresh pairing code: the QR (SVG) and the same one-time link as text, for "Copy pairing link".
#[derive(Serialize)]
pub struct PairCode {
    svg: String,
    link: String,
}

fn new_qr(app: &App) -> PairCode {
    let link = pair_url(app);
    let svg = qrcode::QrCode::with_error_correction_level(&link, qrcode::EcLevel::M)
        .map(|q| {
            q.render::<qrcode::render::svg::Color>()
                .quiet_zone(true)
                .dark_color(qrcode::render::svg::Color("#000000"))
                .light_color(qrcode::render::svg::Color("#FFFFFF"))
                .build()
        })
        .unwrap_or_default();
    PairCode { svg, link }
}

/// A fresh one-time pairing link (what the QR code encodes).
pub fn pair_url(app: &App) -> String {
    let mut store = app.store.lock().unwrap();
    let otp = store.new_otp();
    pair_url_using(app, &store, &otp)
}
pub fn owner_pair_url(app: &App, request: &str, owner: &str) -> Result<String,String> {
    let mut store = app.store.lock().unwrap();
    let otp = store.owner_otp(request,owner)?;
    Ok(pair_url_using(app, &store, &otp))
}
fn pair_url_using(app: &App, store: &crate::store::Store, otp: &str) -> String {
    let mut link = qr_payload(&crate::win::sys::pc_name(), &rank_ips(crate::win::sys::adapters().0), PORT, &app.fp, &otp);
    if let Some(base) = crate::remote::url().filter(|_| store.relay_registered()) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        // The Worker's fixed +/-600s window expires this URL after 120s, matching the OTP.
        // Only a signature is exported, never the ticket that could mint fresh URLs.
        let dev = format!("pair_{}", crate::store::b64url(&crate::store::random::<16>()));
        let pc = store.pc_id();
        let relay = pairing_relay_url(base, pc, &dev, &store.ticket(&dev), now);
        link.push_str(&format!("&pc={pc}&r={}", utf8_percent_encode(&relay, NAME_SET)));
    }
    link
}

/// A fixed URL valid for roughly two minutes under the existing Worker's timestamp policy.
fn pairing_relay_url(base: &str, pc: &str, dev: &str, ticket: &[u8], now: u64) -> String {
    let ts = now.saturating_sub(480);
    let sig = crate::tls::hex(&crate::store::hmac(ticket, &format!("phone:{pc}:{dev}:{ts}")));
    format!("{}/v1/phone?pc={pc}&dev={dev}&ts={ts}&sig={sig}", base.trim_end_matches('/'))
}

#[derive(Serialize)]
pub struct ConfigView {
    config: Config,
    displays: Vec<crate::engine::capture::Display>,
    pad: PadView,
}

#[derive(Serialize)]
struct PadView {
    ok: bool,
    text: &'static str,
}

fn get_config(app: &App) -> ConfigView {
    let store = app.store.lock().unwrap();
    let mut config = store.config().clone();
    config.files_dir = store.files_dir().to_string_lossy().into_owned();
    let (ok, text) = crate::pad::status();
    ConfigView { config, displays: crate::engine::capture::displays(), pad: PadView { ok, text } }
}

async fn set_config(app: &App, config: Config) -> Result<(), String> {
    let old = app.store.lock().unwrap().config().clone();
    app.store.lock().unwrap().set_config(config.clone()).map_err(|e| e.to_string())?;
    if old.anywhere != config.anywhere {
        let st = crate::net::apply_now(config.anywhere).await;
        *app.net.lock().unwrap() = st;
    }
    Ok(())
}

/// Offers `path` to `device`, or to every connected phone; returns how many got the offer.
pub fn offer_to(app: &App, device: Option<&str>, path: &std::path::Path) -> Result<usize, String> {
    let mut devices = device.map(|d| vec![d.to_string()]).unwrap_or_else(|| app.agent.connected());
    devices.sort();
    devices.dedup(); // one offer per phone, even with two sessions open
    let mut sent = 0;
    for d in devices {
        for m in crate::files::offer_tree(&app.outgoing, path).map_err(|e| e.to_string())? {
            sent += app.agent.send(&d, m).min(1);
        }
    }
    Ok(sent)
}

#[derive(Serialize)]
pub struct NetView {
    #[serde(flatten)]
    status: crate::net::NetStatus,
    lan: Vec<String>,
}

#[derive(Serialize)]
pub struct UpdateView {
    current: String,
    latest: String,
    newer: bool,
    preview: bool,
    url: String,
}

async fn check_update() -> Result<UpdateView, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let Some(v) = crate::update::latest_release().await? else {
        return Ok(UpdateView { current, latest: String::new(), newer: false, preview: true, url: "https://github.com/modnite/Secpar/releases".into() });
    };
    let latest = v["tag_name"].as_str().unwrap_or_default().to_string();
    let url = v["html_url"].as_str().unwrap_or("https://github.com/modnite/Secpar/releases").to_string();
    Ok(UpdateView { newer: is_newer(&latest, &current), preview: false, current, latest, url })
}

fn arg<T: serde::de::DeserializeOwned>(args: &Value, k: &str) -> Result<T, String> {
    serde_json::from_value(args[k].clone()).map_err(|e| format!("{k}: {e}"))
}

fn to_value(v: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// The UI's requests over the pipe (crate::ipc), by command name.
pub async fn dispatch(app: Arc<App>, cmd: String, args: Value) -> Result<Value, String> {
    match cmd.as_str() {
        "get_state" => to_value(state(&app)),
        "host_account_state" => to_value(app.account.as_ref().map_err(Clone::clone)?.state().await),
        "host_enrollment_proof" => {
            let challenge: crate::account_link::Challenge = arg(&args, "challenge")?;
            app.account.as_ref().map_err(Clone::clone)?.allow_owner(&challenge.owner).await?;
            to_value(crate::account_link::prove(&app.identity, challenge, &crate::win::sys::pc_name())?)
        }
        "host_account_install" => {
            let state = app.account.as_ref().map_err(Clone::clone)?.install(arg(&args, "registration")?).await?;
            let revoked = app.store.lock().unwrap().set_account_owner(state.owner.clone());
            for device in revoked { app.agent.revoke(&device); }
            to_value(state)
        }
        "host_account_unlink" => {
            let state = app.account.as_ref().map_err(Clone::clone)?.unlink().await?;
            let revoked = app.store.lock().unwrap().set_account_owner(None);
            for device in revoked { app.agent.revoke(&device); }
            to_value(state)
        }
        "new_qr" => to_value(new_qr(&app)),
        "revoke" => {
            app.agent.revoke(&arg::<String>(&args, "id")?);
            Ok(Value::Null)
        }
        "rename" => {
            let name: String = arg::<String>(&args, "name")?.trim().chars().take(40).collect();
            if name.is_empty() {
                return Err("empty name".into());
            }
            app.store.lock().unwrap().rename(&arg::<String>(&args, "id")?, &name).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "get_config" => to_value(get_config(&app)),
        "set_config" => to_value(set_config(&app, arg(&args, "config")?).await?),
        "set_files_dir" => {
            let dir: String = arg(&args, "dir")?;
            let mut store = app.store.lock().unwrap();
            let mut cfg = store.config().clone();
            cfg.files_dir = dir;
            store.set_config(cfg).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "files_dir" => {
            let dir = app.store.lock().unwrap().files_dir();
            let _ = std::fs::create_dir_all(&dir);
            to_value(dir)
        }
        "offer" => {
            let device: Option<String> = arg(&args, "device").ok().flatten();
            let path: String = arg(&args, "path")?;
            to_value(offer_to(&app, device.as_deref(), std::path::Path::new(&path))?)
        }
        "get_net" => to_value(NetView { status: app.net.lock().unwrap().clone(), lan: rank_ips(crate::win::sys::adapters().0) }),
        // The window's Quit and Restart: the service ends this server with it. Done by a helper that
        // outlives it, a second later so this reply gets out first.
        "service_stop" => {
            crate::service::stop_later(false);
            Ok(Value::Null)
        }
        "service_restart" => {
            crate::service::stop_later(true);
            Ok(Value::Null)
        }
        "check_update" => to_value(check_update().await?),
        // The window's silent update: this server runs as SYSTEM, so it installs without a prompt.
        "install_update" => to_value(crate::update::install_tag(&crate::data_dir(), &arg::<String>(&args, "tag")?).await?),
        _ => Err(format!("unknown command {cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn export_host_qr_fixture() {
        let Some(dir) = std::env::var_os("SECPAR_QR_FIXTURE_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fp = "ab".repeat(32);
        let mut link = qr_payload("DESKTOP-Secpar", &["192.168.0.126".into(), "10.0.0.5".into()], PORT, &fp, "abcdefghijklmnopqrstuv");
        let pc = "12345678-1234-1234-1234-123456789012";
        let relay = pairing_relay_url("wss://pcr-rendezvous.sadikhossain747.workers.dev", pc, "pair_abcdefghijklmnopqrstuv", &[7;32], 1_700_000_000);
        link.push_str(&format!("&pc={pc}&r={}", utf8_percent_encode(&relay, NAME_SET)));
        let q = qrcode::QrCode::with_error_correction_level(&link, qrcode::EcLevel::M).unwrap();
        let svg = q.render::<qrcode::render::svg::Color>().quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#000000"))
            .light_color(qrcode::render::svg::Color("#FFFFFF")).build();
        std::fs::write(dir.join("host-qr.svg"), svg).unwrap();
        std::fs::write(dir.join("host-qr.txt"), link).unwrap();
    }

    #[test]
    fn secpar_host_does_not_share_legacy_runtime_or_installer_identity() {
        assert_eq!(PORT, 48999);
        assert_eq!(crate::engine::session::STREAM_PORT, 48998);
        assert_eq!(crate::ipc::PIPE, r"\\.\pipe\secpar");
        assert_eq!(crate::data_dir().file_name().unwrap(), "Secpar");
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], "com.secpar.host");
        assert_eq!(config["productName"], "Secpar Host");
        let hooks = include_str!("../installer/hooks.nsh");
        assert!(hooks.contains("!define PCR_SVC \"Secpar\""));
        assert!(!hooks.contains("pc-remote-host.exe"));
        assert!(!hooks.contains("PCRemote.Send"));
        assert!(!hooks.contains("localport=47999"));
        assert!(!hooks.contains("HiberbootEnabled"));
        assert!(!hooks.contains("Set-NetAdapterPowerManagement"));
        assert!(!hooks.contains("msiexec.exe"));
    }

    #[test]
    fn relay_bootstrap_is_fixed_and_does_not_export_ticket() {
        let url = pairing_relay_url("wss://relay.example/", "PC1", "pair_test", &[7; 32], 1_700_000_000);
        let parsed = reqwest::Url::parse(&url).unwrap();
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(parsed.path(), "/v1/phone");
        assert_eq!(q["ts"], "1699999520");
        assert_eq!(q["dev"], "pair_test");
        assert_eq!(q["sig"], crate::tls::hex(&crate::store::hmac(&[7; 32], "phone:PC1:pair_test:1699999520")));
        assert!(!url.contains("ticket="));
        assert!(!url.contains(&crate::store::b64url(&[7; 32])));
    }

    #[test]
    fn qr_payload_matches_protocol() {
        let fp = "ab".repeat(32);
        let uri = qr_payload("DESKTOP-ABC", &["192.168.0.126".into(), "10.0.0.5".into()], 48999, &fp, "otp_-x");
        assert_eq!(uri, format!("secpar://pair?n=DESKTOP-ABC&h=192.168.0.126,10.0.0.5&p=48999&fp={fp}&t=otp_-x"));
    }

    #[test]
    fn qr_payload_url_encodes_name() {
        let uri = qr_payload("Sadik's PC & Co é", &[], 48999, "f", "t");
        assert!(uri.starts_with("secpar://pair?n=Sadik%27s%20PC%20%26%20Co%20%C3%A9&h=&p=48999"), "{uri}");
    }

    #[test]
    fn lan_ips_private_and_gateway_first() {
        let ips = vec![
            (Ipv4Addr::new(100, 101, 1, 2), true),
            (Ipv4Addr::new(169, 254, 1, 1), true),
            (Ipv4Addr::new(192, 168, 56, 1), false),
            (Ipv4Addr::new(127, 0, 0, 1), true),
            (Ipv4Addr::new(192, 168, 0, 126), true),
            (Ipv4Addr::new(10, 0, 0, 5), true),
            (Ipv4Addr::new(172, 20, 0, 1), false),
            (Ipv4Addr::new(172, 32, 0, 1), true),
        ];
        assert_eq!(
            rank_ips(ips),
            vec!["192.168.0.126", "10.0.0.5", "192.168.56.1", "172.20.0.1", "100.101.1.2", "172.32.0.1"]
        );
    }

    #[test]
    fn a_link_never_carries_more_than_the_client_accepts() {
        let mut ips: Vec<(Ipv4Addr, bool)> = (1..=30).map(|n| (Ipv4Addr::new(172, 20, n, 1), false)).collect();
        ips.push((Ipv4Addr::new(192, 168, 0, 126), true));
        ips.push((Ipv4Addr::new(192, 168, 0, 126), true));
        let listed = rank_ips(ips);
        assert_eq!(listed.len(), MAX_ADDRESSES);
        assert_eq!(listed[0], "192.168.0.126");
        assert_eq!(listed.iter().filter(|a| *a == "192.168.0.126").count(), 1);
    }

    #[test]
    fn newer_version_comparison() {
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("v2.0", "1.9.9"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("garbage", "1.0.0"));
    }
}
