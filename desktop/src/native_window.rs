//! Native restoration must not depend on repainting an already hidden viewport.
use eframe::egui::{Context, ViewportCommand, ViewportId};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy)]
pub(crate) struct NativeWindow(usize);
impl NativeWindow {
    pub fn from_context(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        match cc.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(Self(handle.hwnd.get() as usize)),
            _ => None,
        }
    }
    /// The window handle for the keyboard hook's foreground check.
    pub fn raw(&self) -> usize { self.0 }
    fn valid(&self) -> bool {
        let mut pid = 0;
        // The handle is borrowed from our root viewport; reject stale/reused handles.
        unsafe { IsWindow(self.0 as _) != 0 && GetWindowThreadProcessId(self.0 as _, &mut pid) != 0 && pid == std::process::id() }
    }
    pub fn restore(&self) {
        if self.valid() { unsafe {
            ShowWindowAsync(self.0 as _, SW_RESTORE);
            ShowWindowAsync(self.0 as _, SW_SHOW);
            SetForegroundWindow(self.0 as _);
        } }
    }
    pub fn hide(&self) { if self.valid() {unsafe {ShowWindowAsync(self.0 as _, SW_HIDE);}} }
    #[cfg(feature="tray-review")]
    pub fn visible(&self) -> bool { self.valid() && unsafe {IsWindowVisible(self.0 as _) != 0} }
    #[cfg(feature="tray-review")]
    pub fn minimize(&self) { if self.valid() {unsafe {ShowWindowAsync(self.0 as _, SW_MINIMIZE);}} }
}
#[derive(Clone)]
pub(crate) struct Opener { window: NativeWindow, ctx: Context }
impl Opener {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> { Some(Self {window:NativeWindow::from_context(cc)?,ctx:cc.egui_ctx.clone()}) }
    pub fn open(&self) {
        self.window.restore();
        for command in [ViewportCommand::Visible(true),ViewportCommand::Minimized(false),ViewportCommand::Focus] {
            self.ctx.send_viewport_cmd_to(ViewportId::ROOT,command);
        }
        self.ctx.request_repaint_of(ViewportId::ROOT);
    }
    pub fn hide(&self) {
        self.ctx.send_viewport_cmd_to(ViewportId::ROOT,ViewportCommand::Visible(false));
        self.window.hide();
    }
    #[cfg(feature="tray-review")]
    pub fn visible(&self) -> bool {self.window.visible()}
    #[cfg(feature="tray-review")]
    pub fn minimize(&self) {self.window.minimize()}
}
