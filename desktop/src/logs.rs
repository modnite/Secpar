//! Settings, General, Logs: saves this computer's connection log and its hosting log into one
//! folder in Downloads and opens it, ready to send.
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Panel {
    /// What the last save did; None while saving.
    note: Arc<Mutex<Option<String>>>,
    busy: Arc<std::sync::atomic::AtomicBool>,
}

fn downloads() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let downloads = home.join("Downloads");
    Some(if downloads.is_dir() { downloads } else { home })
}

fn copy_if_there(from: &Path, to: &Path) {
    if from.is_file() {
        let _ = std::fs::copy(from, to);
    }
}

/// The hosting log. Windows: the service keeps it where only it can read, so it is asked for it.
#[cfg(windows)]
fn host_log() -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let value = rt.block_on(crate::local_host::call("host_log", serde_json::json!({})))?;
    Ok(value.as_str().unwrap_or_default().to_string())
}
/// The hosting log. Linux: hosting runs in this app and writes to the user journal.
#[cfg(not(windows))]
fn host_log() -> Result<String, String> {
    let out = std::process::Command::new("journalctl")
        .args(["--user", "--no-pager", "-o", "short-iso", "--since", "-2 days"])
        .output()
        .map_err(|_| "journalctl is not available".to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().filter(|l| l.to_ascii_lowercase().contains("secpar")).collect::<Vec<_>>().join("\n"))
}

fn save() -> Result<PathBuf, String> {
    secpar_client::connlog::flush();
    let stamp = secpar_client::connlog::timestamp(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default());
    let stamp: String = stamp.chars().take(19).map(|c| if c == ':' { '-' } else if c == 'T' { ' ' } else { c }).collect();
    let folder = downloads().ok_or("No Downloads folder")?.join(format!("Secpar logs {stamp} UTC"));
    std::fs::create_dir_all(&folder).map_err(|e| format!("Could not make the folder: {e}"))?;
    if let Some(dir) = secpar_client::connlog::state_dir().map(|d| d.join("logs")) {
        copy_if_there(&dir.join("connect.log.1"), &folder.join("connect.old.log"));
        copy_if_there(&dir.join("connect.log"), &folder.join("connect.log"));
    }
    let host = match host_log() {
        Ok(text) if !text.is_empty() => text,
        Ok(_) => "The hosting log is empty.\n".into(),
        Err(e) => format!("The hosting log could not be read: {e}\n"),
    };
    std::fs::write(folder.join("host.log"), host).map_err(|e| format!("Could not save the hosting log: {e}"))?;
    Ok(folder)
}

impl Panel {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        use crate::design;
        let busy = self.busy.load(std::sync::atomic::Ordering::Relaxed);
        design::group(ui, "Logs", |ui| {
            design::row(ui, "Connection and hosting logs", |ui| {
                if ui.add_enabled(!busy, egui::Button::new("Save logs").min_size(egui::vec2(140., design::CONTROL))).clicked() {
                    self.busy.store(true, std::sync::atomic::Ordering::Relaxed);
                    let (note, busy, ctx) = (self.note.clone(), self.busy.clone(), ui.ctx().clone());
                    std::thread::spawn(move || {
                        let text = match save() {
                            Ok(folder) => {
                                let _ = open::that(&folder);
                                format!("Saved to {}", folder.display())
                            }
                            Err(e) => e,
                        };
                        *note.lock().unwrap() = Some(text);
                        busy.store(false, std::sync::atomic::Ordering::Relaxed);
                        ctx.request_repaint();
                    });
                }
            });
            if let Some(text) = self.note.lock().unwrap().as_ref() {
                ui.add(egui::Label::new(egui::RichText::new(text).color(design::MUTED)).wrap());
                ui.add_space(4.);
            }
        });
    }
}
