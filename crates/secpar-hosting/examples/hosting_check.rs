//! CONTAINER ONLY. The hosting side with the real engine and the real stream
//! component: a device on the access list comes in through the tunnel, pairs,
//! is removed from the list and loses both the tunnel and the pairing.
//!
//! Usage: hosting_check <engine executable> <empty state folder> <stream component>
//! The access list is signed here by a stand-in key; the account service is not
//! involved. Run it inside the headless test session, never on a computer in use.
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use ring::{
    digest,
    rand::SystemRandom,
    signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
};
use secpar_core::{
    account::{Key, ORIGIN},
    hosting::Access,
    identity::hex,
};
use secpar_gamestream_host::{Capture, Config, Encoder, Host};
use secpar_hosting::{request_pairing, Answer, Hosting, Pairing};
use secpar_tunnel::{Identity, Ports};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::process::{Child, Command};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PORT: u16 = 48989;
const PIN: &str = "1234";

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}
fn list(pair: &EcdsaKeyPair, host: &Identity, own: &[&Identity]) -> String {
    let header = serde_json::json!({"alg": "ES256", "typ": "secpar-access+jwt", "kid": "check"});
    let claims = serde_json::json!({
        "iss": ORIGIN, "aud": "secpar-host", "sub": hex(&host.fingerprint()), "iat": now() - 10, "exp": now() + 3600, "owner": "check-owner",
        "devices": own.iter().map(|d| serde_json::json!({"fingerprint": hex(&d.fingerprint()), "name": "Laptop"})).collect::<Vec<_>>(),
        "guests": [],
    });
    let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), URL_SAFE_NO_PAD.encode(claims.to_string()));
    let signature = pair.sign(&SystemRandom::new(), input.as_bytes()).unwrap();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

/// Starts the stream component's pairing against the tunnel's local end, with
/// its own private profile, and reads back the identity it made for itself.
async fn start_pairing(component: &Path, home: &Path) -> Result<(Child, Pairing)> {
    let child = Command::new(component)
        .args(["pair", &format!("127.0.0.2:{PORT}"), "--pin", PIN])
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("SECPAR_MANAGED_PAIRING", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let conf = home.join("config/Secpar/SecparStream.conf");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(pairing) = std::fs::read_to_string(&conf).ok().and_then(|text| identity(&text)) {
            return Ok((child, pairing));
        }
        if Instant::now() > deadline {
            return Err("the stream component did not create its identity".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
fn identity(conf: &str) -> Option<Pairing> {
    let value = |key: &str| conf.lines().find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_owned));
    let id = value("uniqueid")?;
    let pem = value("certificate")?.replace("\\n", "\n");
    let body: String = pem.split("-----").nth(2)?.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')).collect();
    let der = STANDARD.decode(body).ok()?;
    let hash: String = digest::digest(&digest::SHA256, &der).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    Some(Pairing { client_id: id.trim_start_matches("@ByteArray(").trim_end_matches(')').into(), certificate_sha256: hash, pin: PIN.into() })
}
async fn finished(mut child: Child) -> Result<bool> {
    Ok(tokio::time::timeout(Duration::from_secs(25), child.wait()).await??.success())
}
async fn enrolled(engine: &Host, pairing: &Pairing) -> Result<bool> {
    Ok(engine.enrolled().await?.iter().any(|c| c.certificate_sha256 == pairing.certificate_sha256))
}
fn check(what: &str, ok: bool) -> Result {
    println!("{}: {what}", if ok { "ok" } else { "FAILED" });
    if ok { Ok(()) } else { Err(what.to_string().into()) }
}

#[tokio::main]
async fn main() -> Result {
    if std::env::var("SECPAR_SUPERVISOR_REVIEW").as_deref() != Ok("1") {
        return Err("This check only runs in the isolated test container".into());
    }
    let mut args = std::env::args_os().skip(1);
    let executable = PathBuf::from(args.next().ok_or("Missing engine executable")?);
    let root = PathBuf::from(args.next().ok_or("Missing state folder")?);
    let component = PathBuf::from(args.next().ok_or("Missing stream component")?);

    let rng = SystemRandom::new();
    let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let signer = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref(), &rng).unwrap();
    let point = signer.public_key().as_ref().to_vec();
    let keys = [Key { kid: "check".into(), kty: "EC".into(), crv: "P-256".into(), x: URL_SAFE_NO_PAD.encode(&point[1..33]), y: URL_SAFE_NO_PAD.encode(&point[33..]) }];
    let (host_id, laptop, stranger) = (Identity::generate()?, Identity::generate()?, Identity::generate()?);

    std::fs::create_dir_all(&root)?;
    let access = Arc::new(Access::open(ORIGIN, host_id.fingerprint(), root.join("access.json"), now()));
    access.update(&list(&signer, &host_id, &[&laptop]), &keys, now())?;

    let engine = Arc::new(
        Host::start(Config { executable, state: root.join("host"), base_port: PORT, capture: Capture::X11, encoder: Encoder::Software, audio_sink: Some("host_audio".into()) }).await?,
    );
    println!("engine started, version {}", engine.info.version);
    // The engine must only listen on this computer.
    let outside = {
        let probe = std::net::UdpSocket::bind("0.0.0.0:0")?;
        probe.connect("192.0.2.1:9")?;
        probe.local_addr()?.ip()
    };
    let reachable = tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect((outside, PORT))).await.map(|r| r.is_ok()).unwrap_or(false);
    check("the engine does not answer on the network address", !reachable)?;

    let hosting = Hosting::start((outside, 0).into(), &host_id, access.clone(), engine.clone(), "127.0.0.1".parse()?, Ports::engine(PORT)).await?;
    let address = hosting.tunnel().local_addr()?;
    let pin = host_id.fingerprint();
    let connect = move |device: Identity| async move { secpar_tunnel::connect(address, &device, pin, "127.0.0.2".parse().unwrap(), Ports::engine(PORT)).await };
    let copy = |identity: &Identity| Identity::from_der(identity.certificate_der().to_vec(), identity.key_der().to_vec());

    check("a device that is not on the list gets no tunnel", connect(stranger).await.is_err())?;
    let tunnel = connect(copy(&laptop)).await?;
    println!("ok: the laptop's tunnel is up");

    // Pair through the tunnel: the component talks to the engine, the app asks the host.
    let (child, first) = start_pairing(&component, &root.join("profile-a")).await?;
    check("the host completes the pairing asked for over the tunnel", request_pairing(&tunnel, &first).await == Answer::Paired)?;
    check("the stream component reports it is paired", finished(child).await?)?;
    check("the engine has enrolled the component's certificate", enrolled(&engine, &first).await?)?;
    let grants = engine.grants().await;
    check("the grant is recorded for the laptop under the owner", grants.len() == 1 && grants[0].owner == "check-owner" && grants[0].device == hex(&laptop.fingerprint()))?;

    // The laptop is removed from the account.
    access.update(&list(&signer, &host_id, &[]), &keys, now())?;
    hosting.apply().await;
    check("its tunnel is closed", tokio::time::timeout(Duration::from_secs(5), tunnel.closed()).await.is_ok())?;
    // A closed tunnel keeps its local ports until it is dropped.
    drop(tunnel);
    check("it gets no new tunnel", connect(copy(&laptop)).await.is_err())?;
    check("the engine no longer has its certificate", !enrolled(&engine, &first).await?)?;

    // Back on the account: the old stream identity stays refused, a new one pairs.
    access.update(&list(&signer, &host_id, &[&laptop]), &keys, now())?;
    hosting.apply().await;
    let tunnel = connect(copy(&laptop)).await?;
    let (child, again) = start_pairing(&component, &root.join("profile-a")).await?;
    check("the same stream identity is used again", again.certificate_sha256 == first.certificate_sha256)?;
    check("the host answers that this identity was taken away", request_pairing(&tunnel, &again).await == Answer::Revoked)?;
    drop(child);
    let (child, fresh) = start_pairing(&component, &root.join("profile-b")).await?;
    check("a new stream identity pairs", request_pairing(&tunnel, &fresh).await == Answer::Paired && finished(child).await?)?;
    check("only the new certificate is enrolled", enrolled(&engine, &fresh).await? && !enrolled(&engine, &first).await?)?;

    drop(hosting);
    // The tunnel lets go of the engine a moment after it closes.
    let mut engine = engine;
    let engine = loop {
        match Arc::try_unwrap(engine) {
            Ok(engine) => break engine,
            Err(shared) => engine = shared,
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    engine.stop().await?;
    println!("engine stopped");
    Ok(())
}
