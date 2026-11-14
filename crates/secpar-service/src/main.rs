//! The background part of Secpar as a program.
//!
//!   secpar-service --state <folder> --channel <name> [--engine <executable>]
//!                  [--tunnel-port <port>] [--test-account <origin> <address>]
//!                  [--test-identity]
//!
//! With `--engine` this computer hosts. `--channel` is a pipe name on Windows and
//! a socket file path on Linux. `--test-account` points at a test account
//! service on this computer instead of the real one. `--test-identity` uses a
//! throwaway device identity and leaves the system credential store alone; it is
//! only accepted together with `--test-account`.
use secpar_core::{account, identity, store::SystemStore};
use secpar_gamestream_host::{Capture, Config, Encoder, Host};
use secpar_hosting::ENGINE_BASE;
use secpar_service::{channel, Core, Settings};
use std::{path::PathBuf, sync::Arc};

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("secpar-service: {message}");
    std::process::exit(1)
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let (mut state, mut name, mut engine, mut port, mut test, mut throwaway) = (None, None, None, 0u16, None, false);
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| fail(format!("{arg} needs a value")));
        match arg.as_str() {
            "--state" => state = Some(PathBuf::from(value())),
            "--channel" => name = Some(value()),
            "--engine" => engine = Some(PathBuf::from(value())),
            "--tunnel-port" => port = value().parse().unwrap_or_else(|_| fail("--tunnel-port needs a number")),
            "--test-account" => test = Some((value(), value())),
            "--test-identity" => throwaway = true,
            _ => fail(format!("unknown option {arg}")),
        }
    }
    let (Some(state), Some(name)) = (state, name) else { fail("--state and --channel are required") };
    if !state.is_absolute() {
        fail("--state must be a full path");
    }
    if let Err(error) = std::fs::create_dir_all(&state) {
        fail(format!("the state folder cannot be used: {error}"));
    }
    let account = match &test {
        Some((origin, address)) => account::Service::local_test(origin, address),
        None => account::Service::new(account::ORIGIN),
    }
    .unwrap_or_else(|error| fail(error));

    // The device key lives in the system credential store. If it cannot be read,
    // or was lost, this part stops instead of becoming a new device by itself.
    let identity = if throwaway && test.is_some() {
        secpar_tunnel::Identity::generate().unwrap_or_else(|error| fail(error.0))
    } else if throwaway {
        fail("--test-identity needs --test-account")
    } else {
        identity::device(&SystemStore, &state.join("device.txt")).unwrap_or_else(|error| fail(format!("device identity: {error:?}")))
    };
    println!("device {}", identity::hex(&identity.fingerprint()));

    let engine = match engine {
        None => None,
        Some(executable) => {
            let kde_wayland = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.contains("KDE")) && std::env::var_os("WAYLAND_DISPLAY").is_some();
            let config = Config {
                executable,
                state: state.join("engine"),
                base_port: ENGINE_BASE,
                capture: if kde_wayland { Capture::KWin } else { Capture::Automatic },
                encoder: Encoder::Automatic,
                audio_sink: None,
            };
            let host = Host::start(config).await.unwrap_or_else(|error| fail(format!("engine: {error}")));
            println!("engine started, version {}", host.info.version);
            Some(Arc::new(host))
        }
    };
    let bind = ([0, 0, 0, 0], port).into();
    let core = Core::start(Settings { account, state, bind }, identity, engine.clone()).await.unwrap_or_else(|error| fail(error));
    if let Some(address) = core.tunnel_address() {
        println!("hosting, tunnel on UDP port {}", address.port());
    }
    println!("ready");

    let serving = channel::serve(core.clone(), &name);
    tokio::select! {
        result = serving => if let Err(error) = result { eprintln!("secpar-service: channel: {error}") },
        _ = async { core.keep_current().await; std::future::pending::<()>().await } => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    // Close the tunnel, then stop the engine this part started.
    drop(core);
    if let Some(mut engine) = engine {
        // Open conversations with the window let go of the engine a moment later.
        for _ in 0..50 {
            match Arc::try_unwrap(engine) {
                Ok(host) => {
                    if host.stop().await.is_err() {
                        eprintln!("secpar-service: the engine did not stop cleanly");
                    }
                    return;
                }
                Err(shared) => engine = shared,
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // Dropping the last handle on exit ends the engine as well.
    }
}
