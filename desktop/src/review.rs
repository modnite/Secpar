#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
fn main() -> eframe::Result {
    if let Some(path) = std::env::var_os("EFRAME_SCREENSHOT_TO") {
        std::env::set_var("SECPAR_REVIEW_SCREENSHOT_TO", path);
        std::env::remove_var("EFRAME_SCREENSHOT_TO");
    }
    secpar_desktop::run()
}
