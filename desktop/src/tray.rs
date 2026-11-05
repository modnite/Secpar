use eframe::egui::{Context, ViewportCommand};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub struct Tray {
    _icon: TrayIcon,
    quitting: Arc<AtomicBool>,
}
impl Tray {
    pub fn new(ctx: &Context) -> Option<Self> {
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
        let quit = MenuItem::new("Quit Secpar", true, None);
        menu.append_items(&[&open, &PredefinedMenuItem::separator(), &quit])
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
        let quit_id = quit.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == quit_id {
                should_quit.store(true, Ordering::Relaxed);
                ctx_menu.send_viewport_cmd(ViewportCommand::Close);
            } else if event.id == open_id {
                show(&ctx_menu);
            }
        }));
        let ctx_tray = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show(&ctx_tray);
            }
        }));
        Some(Self {
            _icon: tray,
            quitting,
        })
    }
    pub fn hide_on_close(&self, ctx: &Context) -> bool {
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            true
        } else {
            false
        }
    }
}
fn show(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(ViewportCommand::Focus);
}
