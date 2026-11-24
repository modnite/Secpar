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
    /// Moves the window one pixel and back, as dragging it would. A window shown again after being
    /// hidden or minimized (first after an update, or after hours minimized) could draw garbled
    /// until it was moved.
    ///
    /// False while the window still sits at the minimized position (Windows reports -32000 for a
    /// moment while restoring): moving it there would do nothing, so the caller tries again.
    pub fn nudge(&self) -> bool {
        if !self.valid() { return true; }
        let mut r = windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if unsafe { GetWindowRect(self.0 as _, &mut r) } == 0 { return true; }
        if r.left <= -32000 || unsafe { IsIconic(self.0 as _) } != 0 { return false; }
        let flags = SWP_ASYNCWINDOWPOS | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
        unsafe {
            SetWindowPos(self.0 as _, std::ptr::null_mut(), r.left + 1, r.top, 0, 0, flags);
            SetWindowPos(self.0 as _, std::ptr::null_mut(), r.left, r.top, 0, 0, flags);
        }
        true
    }
    /// Minimized or hidden: nothing of it is on screen.
    pub fn away(&self) -> bool { self.valid() && unsafe { IsIconic(self.0 as _) != 0 || IsWindowVisible(self.0 as _) == 0 } }
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
