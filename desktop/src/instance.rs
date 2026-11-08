//! Reopening the Windows shortcut shows the existing window, including from the tray.
use windows_sys::Win32::{
    Foundation::*, System::Threading::CreateMutexW, UI::WindowsAndMessaging::*,
};
pub struct Instance(HANDLE);
impl Instance {
    /// A restarted copy waits for the old window to finish closing.
    pub fn acquire() -> Option<Self> {
        let patient = std::env::args().any(|a| a == "--restarted");
        for _ in 0..if patient { 50 } else { 1 } {
            if let Some(i) = Self::try_acquire() {
                return Some(i);
            }
            if !patient {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        None
    }
    fn try_acquire() -> Option<Self> {
        let name: Vec<u16> = "Local\\SecparDesktop".encode_utf16().chain([0]).collect();
        // SAFETY: valid null-terminated name, default security, no initial ownership.
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if mutex.is_null() {
                return None;
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(mutex);
                if !std::env::args().any(|a| a == "--hidden" || a == "--restarted") {
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
