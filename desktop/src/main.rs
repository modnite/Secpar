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
    // A notification button starts this program with a signed link. It answers and exits.
    if let Some(link) = std::env::args().nth(1).filter(|a| a.starts_with("secpar-alert:")) {
        // Say what happened, so the owner knows one click was enough (or why it was not).
        let note = match secpar_desktop::answer_link(&link) {
            Ok(done) => done,
            Err(error) => format!("Could not answer: {error}"),
        };
        secpar_desktop::notice("Secpar", &note);
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("--send") {
        if let Some(path) = std::env::args().nth(2) {
            if let Err(error) = secpar_desktop::offer_file(path) {
                eprintln!("{error}");
            }
        }
        return Ok(());
    }
    // Another display of a connected computer: just that window, no hosting or tray.
    if std::env::args().nth(1).as_deref() == Some("--view") {
        return secpar_desktop::run_viewer();
    }
    // Layout review screenshots run beside an installed Secpar and leave its hosting alone.
    #[cfg(feature = "ui-review")]
    if std::env::var_os("SECPAR_REVIEW_SCREENSHOT_TO").is_some() {
        return secpar_desktop::run();
    }
    // Opening Secpar brings hosting back after Quit, even when its window is already open.
    #[cfg(windows)]
    let hosting = std::thread::spawn(pc_remote_host::service::ensure_running);
    #[cfg(windows)]
    let Some(_instance) = instance::Instance::acquire() else {
        let _ = hosting.join(); // a second launch only needed to bring hosting back
        return Ok(());
    };
    #[cfg(windows)]
    {
        let _ = pc_remote_host::win::sys::register_send_verb();
    }
    // One Secpar per machine: a second launch shows the running window instead.
    #[cfg(target_os = "linux")]
    if !secpar_desktop::instance_linux::acquire() {
        return Ok(());
    }
    // Connection timing log, so a slow connection can be read off a file.
    secpar_client::connlog::init_default();
    secpar_desktop::run()
}
