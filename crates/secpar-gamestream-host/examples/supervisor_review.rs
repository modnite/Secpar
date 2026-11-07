//! Private generated-display gate. Never run against an installed/user host.
use secpar_gamestream_host::{Capture, ClientIdentity, Config, Encoder, Error, Host};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("SECPAR_SUPERVISOR_REVIEW").as_deref() != Ok("1") {
        return Err("This gate requires the isolated supervisor review container".into());
    }
    let mut args = std::env::args_os().skip(1);
    let executable = PathBuf::from(args.next().ok_or("Missing test host binary")?);
    let root = PathBuf::from(args.next().ok_or("Missing private test state root")?);
    let config = |name: &str, port| Config {
        executable: executable.clone(),
        state: root.join(name),
        base_port: port,
        capture: Capture::X11,
        encoder: Encoder::Software,
        audio_sink: Some("host_audio".into()),
    };
    let first = Host::start(config("first", 48989)).await?;
    let fingerprint = first.info.certificate_sha256.clone();
    assert_eq!(fingerprint.len(), 64);
    assert!(first.info.version.contains("2026.914.233613"));
    assert!(matches!(
        Host::start(config("first", 48989)).await,
        Err(Error("This private host state is already in use"))
    ));
    assert!(first.pending().await?.pairings.is_empty());
    assert_eq!(
        first
            .approve(
                &ClientIdentity {
                    client_id: "fixture".into(),
                    certificate_sha256: "a".repeat(64)
                },
                "1234",
                "Fixture"
            )
            .await,
        Err(Error("Bundled host does not support device-bound pairing"))
    );
    println!("Private authenticated management ready; duplicate start rejected; unbound pairing refused.");
    let other = Host::start(config("other", 49989)).await?;
    first.stop().await?;
    assert!(other.pending().await?.pairings.is_empty());
    println!("Stopping the owned host preserved the independent host.");
    let restarted = Host::start(config("first", 48989)).await?;
    assert_eq!(restarted.info.certificate_sha256, fingerprint);
    restarted.stop().await?;
    other.stop().await?;
    println!("Restart retained the TLS identity; both owned children stopped and were reaped.");
    Ok(())
}
