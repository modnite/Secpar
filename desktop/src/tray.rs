use eframe::egui::{Context, ViewportCommand};
use crate::native_window::Opener;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub struct Tray {
    _icon: TrayIcon,
    quitting: Arc<AtomicBool>,
    opener: Opener,
}
impl Tray {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let ctx = &cc.egui_ctx;
        let opener = Opener::new(cc)?;
        if cfg!(feature = "ui-review") {
            return None;
        }
        let image =
            image::load_from_memory(include_bytes!("../../assets/branding/secpar-icon.png"))
                .ok()?
                .to_rgba8();
        let icon =
            tray_icon::Icon::from_rgba(image.as_raw().clone(), image.width(), image.height())
                .ok()?;
        let menu = Menu::new();
        let open = MenuItem::new("Open Secpar", true, None);
        let restart = MenuItem::new("Restart Secpar", true, None);
        let quit = MenuItem::new("Quit Secpar", true, None);
        menu.append_items(&[&open, &restart, &PredefinedMenuItem::separator(), &quit])
            .ok()?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Secpar")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()
            .ok()?;
        let quitting = Arc::new(AtomicBool::new(false));
        let ctx_menu = ctx.clone();
        let should_quit = quitting.clone();
        let open_id = open.id().clone();
        let menu_opener = opener.clone();
        let quit_id = quit.id().clone();
        let restart_id = restart.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == quit_id || event.id == restart_id {
                let restart = event.id == restart_id;
                let (should_quit, opener, ctx) =
                    (should_quit.clone(), menu_opener.clone(), ctx_menu.clone());
                // Quitting ends the hosting service too; restarting brings both back.
                std::thread::spawn(move || {
                    end_hosting(restart);
                    should_quit.store(true, Ordering::Relaxed);
                    opener.open();
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                });
            } else if event.id == open_id {
                menu_opener.open();
            }
        }));
        let tray_opener = opener.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                tray_opener.open();
            }
        }));
        Some(Self {
            _icon: tray,
            quitting,
            opener,
        })
    }
    #[cfg(feature="tray-review")]
    pub fn opener(&self) -> Opener {self.opener.clone()}
    pub fn hide_on_close(&self, ctx: &Context) -> bool {
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.opener.hide();
            true
        } else {
            false
        }
    }
}

/// Asks the hosting service to stop (and start again for a restart, with a fresh window).
fn end_hosting(restart: bool) {
    #[cfg(windows)]
    {
        let cmd = if restart { "service_restart" } else { "service_stop" };
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
            let _ = rt.block_on(crate::local_host::call(cmd, serde_json::Value::Null));
        }
        if restart {
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).arg("--restarted").spawn();
            }
        }
    }
    #[cfg(not(windows))]
    let _ = restart;
}
