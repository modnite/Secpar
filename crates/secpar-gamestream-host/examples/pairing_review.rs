//! Private certificate-bound pairing gate. Runs only in the isolated review
//! container against an owned host and the private Moonlight worker.
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::digest;
use secpar_gamestream_host::{Capture, ClientIdentity, Config, Encoder, Host};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    process::{Child, Command},
    time::{sleep, timeout},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Worker {
    child: Child,
    identity: ClientIdentity,
}

/// Starts the worker with its own private profile and reads back the identity it
/// generated, independently of the host, from the worker's own settings file.
async fn start_worker(worker: &Path, home: &Path, port: u16, pin: &str) -> Result<Worker> {
    let child = Command::new(worker)
        .args(["pair", &format!("127.0.0.1:{port}"), "--pin", pin])
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
        if let Ok(text) = std::fs::read_to_string(&conf) {
            if let Some(identity) = read_identity(&text) {
                return Ok(Worker { child, identity });
            }
        }
        if Instant::now() > deadline {
            return Err("worker did not create its identity".into());
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn read_identity(conf: &str) -> Option<ClientIdentity> {
    let value = |key: &str| -> Option<String> {
        conf.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_owned))
    };
    let id = value("uniqueid")?;
    let pem = value("certificate")?.replace("\\n", "\n");
    let body: String = pem
        .split("-----")
        .nth(2)? // between BEGIN and END
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    let der = STANDARD.decode(body).ok()?;
    let hash = digest::digest(&digest::SHA256, &der);
    let hex: String = hash.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    Some(ClientIdentity {
        client_id: id
            .trim_start_matches("@ByteArray(")
            .trim_end_matches(')')
            .into(),
        certificate_sha256: hex,
    })
}

async fn wait_pending(host: &Host, identity: &ClientIdentity) -> Result {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let pending = host.pending().await?;
        if pending.pairings.iter().any(|p| {
            p.client_id == identity.client_id && p.certificate_sha256 == identity.certificate_sha256
        }) {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("pending pairing did not appear with the client's own identity".into());
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn exit_code(worker: &mut Worker) -> Result<Option<i32>> {
    Ok(timeout(Duration::from_secs(25), worker.child.wait())
        .await??
        .code())
}

#[tokio::main]
async fn main() -> Result {
    if std::env::var("SECPAR_SUPERVISOR_REVIEW").as_deref() != Ok("1") {
        return Err("This gate requires the isolated review container".into());
    }
    let mut args = std::env::args_os().skip(1);
    let executable = PathBuf::from(args.next().ok_or("Missing patched host binary")?);
    let root = PathBuf::from(args.next().ok_or("Missing private state root")?);
    let worker = PathBuf::from(args.next().ok_or("Missing private client worker")?);
    let port = 48989;
    let host = Host::start(Config {
        executable,
        state: root.join("host"),
        base_port: port,
        capture: Capture::X11,
        encoder: Encoder::Software,
        audio_sink: Some("host_audio".into()),
    })
    .await?;
    println!("Patched host started with stdin-provisioned management credentials.");
    assert_eq!(host.pending().await?.secpar_identity_binding, 1);

    // 1. Wrong fingerprint, wrong client id and an incomplete grant are refused by
    //    the HOST under its pairing lock. The pairing stays pending afterwards.
    let mut a = start_worker(&worker, &root.join("client-a"), port, "1234").await?;
    wait_pending(&host, &a.identity).await?;
    let pending = host.pending().await?;
    let id = pending.pairings[0].id.clone();
    let post = |client: &str, hash: &str| {
        json!({"pairing_id": id, "pin": "1234", "name": "Review A",
               "expected_client_id": client, "expected_certificate_sha256": hash})
    };
    let wrong_hash = "0".repeat(64);
    let r = host
        .review_post_pin(post(&a.identity.client_id, &wrong_hash))
        .await?;
    assert_eq!(r["status"], false, "wrong fingerprint must be refused");
    let r = host
        .review_post_pin(post("not-the-client", &a.identity.certificate_sha256))
        .await?;
    assert_eq!(r["status"], false, "wrong client id must be refused");
    let r = host
        .review_post_pin(json!({"pairing_id": id, "pin": "1234", "name": "Review A",
               "expected_client_id": a.identity.client_id}))
        .await?;
    assert_eq!(r["status"], false, "incomplete grant must be refused");
    assert!(host.enrolled().await?.is_empty());
    println!(
        "Host refused wrong fingerprint, wrong client id and an incomplete grant; nothing enrolled."
    );

    // 2. The controller's own gate refuses a different identity before the POST.
    assert!(host
        .approve(
            &ClientIdentity {
                client_id: a.identity.client_id.clone(),
                certificate_sha256: wrong_hash.clone()
            },
            "1234",
            "Review A"
        )
        .await
        .is_err());

    // 3. The exact identity completes pairing for the real worker.
    host.approve(&a.identity, "1234", "Review A").await?;
    assert_eq!(exit_code(&mut a).await?, Some(0), "worker must finish pairing");
    let list = host.enrolled().await?;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].certificate_sha256, a.identity.certificate_sha256);
    assert!(host.pending().await?.pairings.is_empty());
    println!("Exact identity approved: the real client paired and the host recorded its fingerprint.");

    // 4. A second device cannot ride on the first grant.
    let mut b = start_worker(&worker, &root.join("client-b"), port, "1234").await?;
    wait_pending(&host, &b.identity).await?;
    assert_ne!(b.identity.certificate_sha256, a.identity.certificate_sha256);
    assert!(host.approve(&a.identity, "1234", "Review A").await.is_err());
    assert_eq!(host.enrolled().await?.len(), 1);
    host.approve(&b.identity, "1234", "Review B").await?;
    assert_eq!(exit_code(&mut b).await?, Some(0));
    assert_eq!(host.enrolled().await?.len(), 2);
    println!("Second device needed its own exact approval; two distinct fingerprints enrolled.");

    // 5. A correct grant with a mismatched PIN does not enroll the client.
    let mut c = start_worker(&worker, &root.join("client-c"), port, "4321").await?;
    wait_pending(&host, &c.identity).await?;
    let _ = host.approve(&c.identity, "1234", "Review C").await;
    let code = exit_code(&mut c).await?;
    assert_ne!(code, Some(0), "mismatched PIN must not pair");
    // The approval call can block until the failed pairing is torn down.
    sleep(Duration::from_secs(2)).await;
    assert_eq!(host.enrolled().await?.len(), 2);
    println!("Mismatched PIN failed the pairing and enrolled nothing.");

    host.stop().await?;
    println!("Certificate-bound pairing review complete.");
    Ok(())
}
