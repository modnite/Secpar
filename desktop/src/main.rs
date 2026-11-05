#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(windows)]
mod instance;
fn main() -> eframe::Result {
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
    secpar_desktop::run()
}
