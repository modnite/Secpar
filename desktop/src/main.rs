#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(windows)]
mod instance;
fn main() -> eframe::Result {
    // One executable: the Windows service and its server are this program in other modes.
    #[cfg(windows)]
    match std::env::args().nth(1).as_deref() {
        Some("--service") => return Ok(pc_remote_host::service::run()),
        Some("--server") => return Ok(pc_remote_host::server::run()),
        _ => {}
    }
    if std::env::args().nth(1).as_deref() == Some("--send") {
        if let Some(path) = std::env::args().nth(2) {
            if let Err(error) = secpar_desktop::offer_file(path) {
                eprintln!("{error}");
            }
        }
        return Ok(());
    }
    #[cfg(windows)]
    let Some(_instance) = instance::Instance::acquire() else {
        return Ok(());
    };
    #[cfg(windows)]
    {
        let _ = pc_remote_host::win::sys::register_send_verb();
        // Opening Secpar brings hosting back after Quit.
        std::thread::spawn(pc_remote_host::service::ensure_running);
    }
    secpar_desktop::run()
}
