#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
fn main() {
    #[cfg(windows)] secpar_desktop::run_tray_review().unwrap();
}
