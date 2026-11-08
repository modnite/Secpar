//! Keeps this computer awake while a stream is connected: no screen dimming, no screen lock, no
//! sleep. Dropping it lets the system idle again.

pub struct KeepAwake(Inner);

impl KeepAwake {
    pub fn new() -> Option<Self> {
        Inner::new().map(Self)
    }
}

#[cfg(windows)]
struct Inner(Option<(std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>)>);

#[cfg(windows)]
impl Inner {
    fn new() -> Option<Self> {
        use windows_sys::Win32::System::Power::*;
        // The state belongs to the thread that sets it: a thread of its own holds it until dropped.
        let (stop, wait) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            // SAFETY: plain flag update for the calling thread.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED) };
            let _ = wait.recv();
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        });
        Some(Inner(Some((stop, thread))))
    }
}

#[cfg(windows)]
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some((stop, thread)) = self.0.take() {
            let _ = stop.send(());
            let _ = thread.join();
        }
    }
}

/// The desktop's own inhibit calls, over the session bus: freedesktop ScreenSaver and
/// PowerManagement (KDE, XFCE, most others) and GNOME's session manager. The inhibits last as long
/// as this connection does.
#[cfg(target_os = "linux")]
struct Inner {
    bus: dbus::blocking::Connection,
    uninhibit: Vec<(String, String, String, u32)>,
}

#[cfg(target_os = "linux")]
impl Inner {
    fn new() -> Option<Self> {
        use std::time::Duration;
        let bus = dbus::blocking::Connection::new_session().ok()?;
        let mut uninhibit = Vec::new();
        let (app, why) = ("Secpar", "Streaming a remote computer");
        let timeout = Duration::from_secs(2);
        for (dest, path, iface, un) in [
            ("org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver", "UnInhibit"),
            ("org.freedesktop.ScreenSaver", "/ScreenSaver", "org.freedesktop.ScreenSaver", "UnInhibit"),
            ("org.freedesktop.PowerManagement", "/org/freedesktop/PowerManagement/Inhibit", "org.freedesktop.PowerManagement.Inhibit", "UnInhibit"),
        ] {
            let proxy = bus.with_proxy(dest, path, timeout);
            if let Ok((cookie,)) = proxy.method_call::<(u32,), _, _, _>(iface, "Inhibit", (app, why)) {
                uninhibit.push((dest.to_string(), path.to_string(), format!("{iface}\0{un}"), cookie));
            }
        }
        let proxy = bus.with_proxy("org.gnome.SessionManager", "/org/gnome/SessionManager", timeout);
        // 4 = suspend, 8 = idle
        if let Ok((cookie,)) = proxy.method_call::<(u32,), _, _, _>("org.gnome.SessionManager", "Inhibit", (app, 0u32, why, 12u32)) {
            uninhibit.push(("org.gnome.SessionManager".into(), "/org/gnome/SessionManager".into(), "org.gnome.SessionManager\0Uninhibit".into(), cookie));
        }
        (!uninhibit.is_empty()).then_some(Inner { bus, uninhibit })
    }
}

#[cfg(target_os = "linux")]
impl Drop for Inner {
    fn drop(&mut self) {
        let timeout = std::time::Duration::from_secs(1);
        for (dest, path, iface_method, cookie) in self.uninhibit.drain(..) {
            let (iface, method) = iface_method.split_once('\0').unwrap_or_default();
            let proxy = self.bus.with_proxy(dest, path, timeout);
            let _ = proxy.method_call::<(), _, _, _>(iface, method, (cookie,));
        }
    }
}

/// macOS: the system's own `caffeinate`, held while this runs.
#[cfg(target_os = "macos")]
struct Inner(std::process::Child);

#[cfg(target_os = "macos")]
impl Inner {
    fn new() -> Option<Self> {
        std::process::Command::new("caffeinate").args(["-d", "-i"]).spawn().ok().map(Inner)
    }
}

#[cfg(target_os = "macos")]
impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
struct Inner;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
impl Inner {
    fn new() -> Option<Self> {
        None
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    /// Run by hand on a desktop session: holds the inhibit for 20 s so it can be listed.
    #[test]
    #[ignore = "needs a desktop session bus"]
    fn holds_an_inhibit_on_this_desktop() {
        let held = super::KeepAwake::new();
        assert!(held.is_some(), "no desktop inhibit service answered");
        std::thread::sleep(std::time::Duration::from_secs(20));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    /// Run by hand: holds the request for 20 s so `powercfg /requests` can list it.
    #[test]
    #[ignore = "inspect with powercfg /requests"]
    fn holds_a_display_request() {
        let held = super::KeepAwake::new();
        assert!(held.is_some());
        std::thread::sleep(std::time::Duration::from_secs(20));
    }
}
