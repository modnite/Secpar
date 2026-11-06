//! Reopening the Windows shortcut shows the existing window, including from the tray.
use windows_sys::Win32::{
    Foundation::*, System::Threading::CreateMutexW, UI::WindowsAndMessaging::*,
};
pub struct Instance(HANDLE);
impl Instance {
    pub fn acquire() -> Option<Self> {
        let name: Vec<u16> = "Local\\SecparDesktop".encode_utf16().chain([0]).collect();
        // SAFETY: valid null-terminated name, default security, no initial ownership.
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if mutex.is_null() {
                return None;
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(mutex);
                if !std::env::args().any(|a| a == "--hidden") {
                    let title: Vec<u16> = "Secpar"
                        .encode_utf16()
                        .chain([0])
                        .collect();
                    let window = FindWindowW(std::ptr::null(), title.as_ptr());
                    if !window.is_null() {
                        ShowWindowAsync(window, SW_RESTORE);
                        ShowWindowAsync(window, SW_SHOW);
                        SetForegroundWindow(window);
                    }
                }
                None
            } else {
                Some(Self(mutex))
            }
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
