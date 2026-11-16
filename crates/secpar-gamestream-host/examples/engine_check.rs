//! Starts the bundled engine privately, checks the management connection, stops
//! it, and starts it again to confirm it keeps its identity.
//! Usage: engine_check <engine executable> <empty state folder> <base port>
//! Pick a base port whose range (base-5 to base+21) is free. Never point the
//! state folder at an installed host's data.
use secpar_gamestream_host::{Capture, Config, Encoder, Host};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let executable = PathBuf::from(args.next().ok_or("Missing engine executable")?);
    let state = PathBuf::from(args.next().ok_or("Missing state folder")?);
    let base_port: u16 = args
        .next()
        .and_then(|p| p.to_str().and_then(|p| p.parse().ok()))
        .ok_or("Missing base port")?;
    let config = || Config {
        executable: executable.clone(),
        state: state.clone(),
        base_port,
        capture: Capture::Automatic,
        encoder: Encoder::Automatic,
        audio_sink: None, preset: Default::default(), output: None,
    };
    let host = Host::start(config()).await?;
    let fingerprint = host.info.certificate_sha256.clone();
    println!("started: engine version {}, pid {:?}", host.info.version, host.id());
    println!("pending pairings: {}", host.pending().await?.pairings.len());
    println!("enrolled clients: {}", host.enrolled().await?.len());
    let second = Host::start(config()).await;
    println!("second start on the same state refused: {}", second.is_err());
    host.stop().await?;
    println!("stopped");
    let again = Host::start(config()).await?;
    println!("restart kept its certificate: {}", again.info.certificate_sha256 == fingerprint);
    again.stop().await?;
    println!("stopped again");
    Ok(())
}
