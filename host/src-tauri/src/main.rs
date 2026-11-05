#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--service") => return pc_remote_host::service::run(),
        Some("--server") => return pc_remote_host::server::run(),
        _ => {}
    }
    // The combined package uses one desktop UI; service/server modes above retain
    // the existing store, TLS identity, owner pairings and Windows session handling.
    let _ = pc_remote_host::win::sys::register_send_verb();
    if std::env::args().nth(1).is_none()
        || matches!(
            std::env::args().nth(1).as_deref(),
            Some("--hidden" | "--send")
        )
    {
        if let Ok(exe) = std::env::current_exe() {
            let desktop = exe.with_file_name("secpar-desktop.exe");
            if desktop.is_file() {
                if std::process::Command::new(desktop)
                    .args(std::env::args().skip(1))
                    .spawn()
                    .is_ok()
                {
                    return;
                }
            }
        }
    }
    pc_remote_host::run_with(|b| {
        b.plugin(tauri_plugin_notification::init()).setup(|app| {
            let handle = app.app_handle().clone();
            pc_remote_host::notify::set_hook(Box::new(move |title, body| {
                let _ = handle
                    .notification()
                    .builder()
                    .title(title)
                    .body(body)
                    .show();
            }));
            Ok(())
        })
    })
}
