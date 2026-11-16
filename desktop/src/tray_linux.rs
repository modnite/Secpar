//! The Linux tray icon: a StatusNotifierItem on the session bus, which KDE Plasma shows natively.
//! Hosting runs inside this process, so the icon is what keeps it visible while the window is
//! closed, and Quit is what ends it.
use eframe::egui::{Context, ViewportCommand};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub struct Tray {
    handle: Handle<Item>,
    quitting: Arc<AtomicBool>,
    disconnect_asked: Arc<AtomicBool>,
    ctx: Context,
    enabled: std::cell::Cell<bool>,
}

struct Item {
    ctx: Context,
    icon: Vec<ksni::Icon>,
    connected: bool,
    quitting: Arc<AtomicBool>,
    disconnect_asked: Arc<AtomicBool>,
}

fn open(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
    ctx.request_repaint();
}

/// Hidden where the system allows it; Wayland cannot hide a window, so it is minimised there.
fn hide(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
}

impl ksni::Tray for Item {
    const MENU_ON_ACTIVATE: bool = false;
    fn id(&self) -> String {
        "com.secpar.desktop".into()
    }
    fn title(&self) -> String {
        "Secpar".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Secpar".into(), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        open(&self.ctx);
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem { label: "Open Secpar".into(), activate: Box::new(|item: &mut Self| open(&item.ctx)), ..Default::default() }.into(),
            StandardItem {
                label: "Disconnect".into(),
                enabled: self.connected,
                activate: Box::new(|item: &mut Self| {
                    item.disconnect_asked.store(true, Ordering::Relaxed);
                    item.ctx.request_repaint();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Restart Secpar".into(),
                activate: Box::new(|item: &mut Self| {
                    // A fresh copy starts once this one has let go of its ports.
                    let program = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).or_else(|| std::env::current_exe().ok());
                    if let Some(program) = program {
                        let _ = std::process::Command::new("sh")
                            .arg("-c")
                            .arg("sleep 3; exec \"$0\"")
                            .arg(program)
                            .stdin(std::process::Stdio::null())
                            .spawn();
                    }
                    item.quitting.store(true, Ordering::Relaxed);
                    open(&item.ctx);
                    item.ctx.send_viewport_cmd(ViewportCommand::Close);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit Secpar".into(),
                activate: Box::new(|item: &mut Self| {
                    // Hosting lives in this process, so closing the window ends it too.
                    item.quitting.store(true, Ordering::Relaxed);
                    open(&item.ctx);
                    item.ctx.send_viewport_cmd(ViewportCommand::Close);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

impl Tray {
    /// `None` where the desktop shows no tray (GNOME without its AppIndicator extension, for one);
    /// closing the window then quits as before.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        if cfg!(feature = "ui-review") {
            return None;
        }
        let image = image::load_from_memory(include_bytes!("../../assets/branding/secpar-icon.png")).ok()?;
        // Two sizes so the panel need not scale a large image down itself.
        let icon = [22u32, 44]
            .into_iter()
            .map(|size| {
                let rgba = image.resize(size, size, image::imageops::FilterType::Lanczos3).to_rgba8();
                let mut data = Vec::with_capacity(rgba.len());
                for p in rgba.pixels() {
                    data.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
                }
                ksni::Icon { width: rgba.width() as i32, height: rgba.height() as i32, data }
            })
            .collect();
        let quitting = Arc::new(AtomicBool::new(false));
        let disconnect_asked = Arc::new(AtomicBool::new(false));
        let ctx = cc.egui_ctx.clone();
        let item = Item { ctx: ctx.clone(), icon, connected: false, quitting: quitting.clone(), disconnect_asked: disconnect_asked.clone() };
        let handle = item.spawn().map_err(|e| eprintln!("secpar: no tray icon: {e}")).ok()?;
        // Started at sign-in: stay out of the way until the tray icon is clicked.
        if std::env::args().any(|a| a == "--hidden") {
            hide(&ctx);
        }
        Some(Self { handle, quitting, disconnect_asked, ctx, enabled: std::cell::Cell::new(false) })
    }
    /// Greys out Disconnect while no stream is connected.
    pub fn set_connected(&self, connected: bool) {
        if self.enabled.replace(connected) != connected {
            self.handle.update(|item| item.connected = connected);
        }
    }
    /// True once after Disconnect was picked in the tray menu.
    pub fn take_disconnect(&self) -> bool {
        self.disconnect_asked.swap(false, Ordering::Relaxed)
    }
    pub fn hide_on_close(&self, ctx: &Context) -> bool {
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            hide(&self.ctx);
            true
        } else {
            false
        }
    }
}
