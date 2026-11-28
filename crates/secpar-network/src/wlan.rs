//! Wi-Fi media streaming mode while a stream runs (Windows).
//!
//! Windows' WLAN service scans other channels in the background every minute or so; the card
//! leaves the channel for a few hundred milliseconds and a stream loses a burst of packets each
//! time. `wlan_intf_opcode_media_streaming_mode` keeps the card on its channel, and the setting
//! holds only while the handle that set it stays open, so the handle lives until the last stream
//! ends. Background scanning is also turned off where Windows allows it. Both are best effort:
//! a PC without Wi-Fi, or a refusal, changes nothing.
//!
//! Hosts and clients call [`hold`] once per stream; the mode ends when the last [`Hold`] drops.

/// Keeps media streaming mode on while alive.
pub struct Hold(());

impl Drop for Hold {
    fn drop(&mut self) {
        imp::release();
    }
}

/// Media streaming mode on (or kept on) for one more stream. `diagnostics` gets one line when
/// it is switched on.
pub fn hold(diagnostics: Option<fn(&str)>) -> Hold {
    imp::acquire(diagnostics);
    Hold(())
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Mutex;

    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    #[repr(C)]
    struct Interface {
        guid: Guid,
        description: [u16; 256],
        state: u32,
    }
    #[repr(C)]
    struct Interfaces {
        count: u32,
        index: u32,
        first: [Interface; 1],
    }

    #[link(name = "wlanapi")]
    extern "system" {
        fn WlanOpenHandle(client_version: u32, reserved: *mut c_void, negotiated: *mut u32, handle: *mut isize) -> u32;
        fn WlanCloseHandle(handle: isize, reserved: *mut c_void) -> u32;
        fn WlanEnumInterfaces(handle: isize, reserved: *mut c_void, list: *mut *mut Interfaces) -> u32;
        fn WlanSetInterface(handle: isize, guid: *const Guid, opcode: u32, size: u32, data: *const c_void, reserved: *mut c_void) -> u32;
        fn WlanFreeMemory(memory: *mut c_void);
    }

    /// WLAN_INTF_OPCODE values from wlanapi.h.
    const BACKGROUND_SCAN_ENABLED: u32 = 2;
    const MEDIA_STREAMING_MODE: u32 = 3;

    /// (streams holding it, the open WLAN handle)
    static STATE: Mutex<(usize, Option<isize>)> = Mutex::new((0, None));

    pub(super) fn acquire(diagnostics: Option<fn(&str)>) {
        let mut state = STATE.lock().unwrap();
        state.0 += 1;
        if state.1.is_some() {
            return;
        }
        let (mut negotiated, mut handle) = (0u32, 0isize);
        // SAFETY: out-parameters are valid; the handle is closed in `release`.
        if unsafe { WlanOpenHandle(2, std::ptr::null_mut(), &mut negotiated, &mut handle) } != 0 {
            return; // no WLAN service: no Wi-Fi to tune
        }
        let mut list: *mut Interfaces = std::ptr::null_mut();
        // SAFETY: `list` is allocated by the WLAN API and freed with WlanFreeMemory below.
        unsafe {
            if WlanEnumInterfaces(handle, std::ptr::null_mut(), &mut list) == 0 && !list.is_null() {
                let count = (*list).count as usize;
                let first = std::ptr::addr_of!((*list).first) as *const Interface;
                let (on, off) = (1u32, 0u32);
                let mut tuned = 0;
                for i in 0..count {
                    let guid = std::ptr::addr_of!((*first.add(i)).guid);
                    if WlanSetInterface(handle, guid, MEDIA_STREAMING_MODE, 4, &on as *const u32 as *const c_void, std::ptr::null_mut()) == 0 {
                        tuned += 1;
                    }
                    // May need administrator rights; refused silently otherwise.
                    let _ = WlanSetInterface(handle, guid, BACKGROUND_SCAN_ENABLED, 4, &off as *const u32 as *const c_void, std::ptr::null_mut());
                }
                WlanFreeMemory(list as *mut c_void);
                if tuned > 0 {
                    let line = format!("wlan: media streaming mode on for {tuned} Wi-Fi adapter(s)");
                    match diagnostics {
                        Some(d) => d(&line),
                        None => log::info!("{line}"),
                    }
                }
            }
        }
        state.1 = Some(handle);
    }

    pub(super) fn release() {
        let mut state = STATE.lock().unwrap();
        state.0 = state.0.saturating_sub(1);
        if state.0 == 0 {
            if let Some(handle) = state.1.take() {
                // Closing the handle ends media streaming mode; background scanning comes back
                // with it (Windows restores it when the setting's owner goes away).
                // SAFETY: the handle came from WlanOpenHandle and is closed once.
                unsafe { WlanCloseHandle(handle, std::ptr::null_mut()) };
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn acquire(_: Option<fn(&str)>) {}
    pub(super) fn release() {}
}

#[cfg(test)]
mod tests {
    /// Holding and dropping works on any PC, with or without Wi-Fi.
    #[test]
    fn holds_nest_and_release() {
        let a = super::hold(None);
        let b = super::hold(None);
        drop(a);
        drop(b);
        let _c = super::hold(None);
    }
}
