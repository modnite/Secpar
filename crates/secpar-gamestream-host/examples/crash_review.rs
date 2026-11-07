//! Starts an owned host, records its pid, then dies WITHOUT running destructors.
//! The review script checks that the engine child is gone afterwards.
use secpar_gamestream_host::{Capture, Config, Encoder, Host};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("SECPAR_SUPERVISOR_REVIEW").as_deref() != Ok("1") {
        return Err("This gate requires the isolated review container".into());
    }
    let mut args = std::env::args_os().skip(1);
    let executable = PathBuf::from(args.next().ok_or("Missing host binary")?);
    let root = PathBuf::from(args.next().ok_or("Missing state root")?);
    let pid_file = PathBuf::from(args.next().ok_or("Missing pid file")?);
    let host = Host::start(Config {
        executable,
        state: root.join("host"),
        base_port: 48989,
        capture: Capture::X11,
        encoder: Encoder::Software,
        audio_sink: None,
    })
    .await?;
    std::fs::write(&pid_file, host.id().ok_or("no pid")?.to_string())?;
    std::process::abort();
}
