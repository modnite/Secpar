//! One Secpar per machine on Linux. The first copy holds an abstract socket (it disappears
//! with the process, even after a crash); a second launch asks that copy to show its window
//! and exits. A copy started by a restart or an update waits a moment for the old one to go.

use std::io::{Read, Write};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::sync::OnceLock;
use std::time::Duration;

static HELD: OnceLock<UnixListener> = OnceLock::new();

fn name() -> std::io::Result<SocketAddr> {
    SocketAddr::from_abstract_name(b"com.secpar.desktop")
}

/// True when this is the only copy. Otherwise the running copy is asked to show itself.
pub fn acquire() -> bool {
    let Ok(addr) = name() else { return true };
    let patient = std::env::args().any(|a| a == "--restarted");
    for attempt in 0..if patient { 50 } else { 1 } {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
        if let Ok(listener) = UnixListener::bind_addr(&addr) {
            let _ = HELD.set(listener);
            return true;
        }
    }
    if !std::env::args().any(|a| a == "--hidden") {
        if let Ok(mut other) = UnixStream::connect_addr(&addr) {
            let _ = other.write_all(b"show");
        }
    }
    false
}

/// Shows the window whenever another launch asks.
pub fn serve(ctx: eframe::egui::Context) {
    let Some(listener) = HELD.get() else { return };
    let Ok(listener) = listener.try_clone() else { return };
    std::thread::Builder::new()
        .name("secpar-instance".into())
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut ask = [0u8; 4];
                if stream.read_exact(&mut ask).is_ok() && &ask == b"show" {
                    use eframe::egui::ViewportCommand;
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                    ctx.request_repaint();
                }
            }
        })
        .ok();
}
