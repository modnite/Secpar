//! Keeps this computer's clipboard text in step with a computer it is connected
//! to, in both directions. Nothing is read or written while no computer is
//! connected, and text that was already in the clipboard when the connection
//! started is never shared.
//!
//! This computer can host and connect at the same time, so one instance serves
//! both sides. The hosting side answers the other computer's requests; the
//! connecting side asks about twice a second.
use secpar_clipboard::{Bridge, Event};
use secpar_hosting::{Clipboard, CLIPBOARD_TICK};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

/// This computer's OS clipboard, as both roles use it.
pub struct Shared {
    /// Whether the person allows clipboard sharing. On unless they turn it off.
    on: AtomicBool,
    /// Which sides are using the clipboard right now.
    used: Mutex<Used>,
    /// Made when a side starts using it, dropped when the last one stops, so the
    /// OS clipboard is left alone while nothing is connected.
    bridge: Mutex<Option<Bridge>>,
    /// The text this computer copied while connected. Text from before is never
    /// kept, so it is never shared.
    copied: Mutex<Option<String>>,
    open: Box<dyn Fn() -> Bridge + Send + Sync>,
}
#[derive(Default)]
struct Used {
    hosting: bool,
    connecting: bool,
}
impl Used {
    fn any(&self) -> bool {
        self.hosting || self.connecting
    }
}

impl Shared {
    pub fn start(on: bool) -> Arc<Self> {
        Self::with(on, || Bridge::start())
    }
    /// Like `start`, on a clipboard the caller supplies instead of the OS one.
    pub fn with(on: bool, open: impl Fn() -> Bridge + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            on: AtomicBool::new(on),
            used: Mutex::new(Used::default()),
            bridge: Mutex::new(None),
            copied: Mutex::new(None),
            open: Box::new(open),
        })
    }
    pub fn sharing(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }
    /// Turns sharing on or off. Switching it off lets the OS clipboard go.
    pub fn set_sharing(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
        if !on {
            self.let_go();
        }
    }
    /// The hosting or the connecting side starts or stops using the clipboard.
    /// The OS clipboard is only watched while one of them does.
    pub fn watch(&self, connecting: bool, connected: bool) {
        {
            let mut used = self.used.lock().unwrap();
            if connecting {
                used.connecting = connected;
            } else {
                used.hosting = connected;
            }
            if self.sharing() && used.any() {
                let mut bridge = self.bridge.lock().unwrap();
                if bridge.is_none() {
                    // Whatever is in the clipboard now is not shared.
                    *self.copied.lock().unwrap() = None;
                    *bridge = Some((self.open)());
                }
                return;
            }
        }
        self.let_go_now();
    }
    /// Takes in what this computer copied since the last look.
    pub fn read(&self) {
        if !self.sharing() {
            return;
        }
        let mut bridge = self.bridge.lock().unwrap();
        let Some(bridge) = bridge.as_mut() else { return };
        let mut newest = None;
        while let Some(event) = bridge.poll() {
            if let Event::Text(text) = event {
                newest = Some(text);
            }
        }
        if let Some(text) = newest {
            *self.copied.lock().unwrap() = Some(text);
        }
    }
    /// The text this computer copied while connected, if any.
    pub fn copied(&self) -> Option<String> {
        self.copied.lock().unwrap().clone()
    }
    /// Puts text from the other computer into this one's clipboard.
    pub fn put(&self, text: &str) -> bool {
        if !self.sharing() || !secpar_clipboard::valid(text) {
            return false;
        }
        {
            let mut bridge = self.bridge.lock().unwrap();
            let Some(bridge) = bridge.as_mut() else { return false };
            if !bridge.set(text.to_string()) {
                return false;
            }
        }
        // The bridge writes the text itself, so this is not a new copy.
        *self.copied.lock().unwrap() = Some(text.to_string());
        true
    }
    fn let_go(&self) {
        let mut used = self.used.lock().unwrap();
        *used = Used::default();
        drop(used);
        self.let_go_now();
    }
    fn let_go_now(&self) {
        // A side stopped using the clipboard, or sharing was switched off.
        *self.bridge.lock().unwrap() = None;
        *self.copied.lock().unwrap() = None;
    }
}
impl Clipboard for Shared {
    fn text(&self) -> Option<String> {
        self.copied()
    }
    fn set(&self, text: &str) -> bool {
        self.put(text)
    }
    fn attach(&self, connected: bool) {
        self.watch(false, connected);
    }
    fn poll(&self) {
        self.read();
    }
}

/// Keeps the clipboard in step with the computer this one is connected to,
/// about twice a second. Returns when the tunnel closes.
///
/// `tunnel` is weak on purpose: letting go of the tunnel in the window must close
/// it, so this loop never keeps it open.
pub async fn follow(clipboard: Arc<Shared>, tunnel: std::sync::Weak<secpar_tunnel::Client>) {
    // What this computer last put in the other's clipboard.
    let mut sent: Option<String> = None;
    // What the other computer last had.
    let mut theirs: Option<String> = None;
    // A copy that has not reached the other computer yet, and how often it was tried.
    let mut pending: Option<String> = None;
    let mut tries = 0u32;
    loop {
        tokio::time::sleep(CLIPBOARD_TICK).await;
        let Some(tunnel) = tunnel.upgrade() else {
            clipboard.watch(true, false);
            return;
        };
        if tunnel.is_closed() {
            clipboard.watch(true, false);
            return;
        }
        if !clipboard.sharing() {
            clipboard.watch(true, false);
            continue;
        }
        clipboard.watch(true, true);
        clipboard.read();
        if let Some(text) = clipboard.copied() {
            if sent.as_ref() != Some(&text) {
                pending = Some(text);
                tries = 0;
            }
        }
        // This computer's copy goes over first, so its echo is not applied back.
        if let Some(text) = pending.clone() {
            if secpar_hosting::set_clipboard(&tunnel, &text).await {
                sent = Some(text.clone());
                theirs = Some(text);
                pending = None;
            } else {
                // A computer that will not take it (a friend who may not use the
                // keyboard, for example) is not asked again and again.
                tries += 1;
                if tries >= 3 {
                    sent = Some(text);
                    pending = None;
                }
            }
        }
        if let Some(text) = secpar_hosting::clipboard_text(&tunnel).await {
            if theirs.as_ref() != Some(&text) && clipboard.put(&text) {
                theirs = Some(text.clone());
                // Text from the other computer is not a copy made here.
                sent = Some(text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    /// A clipboard that is not the OS one.
    struct Fake(Arc<StdMutex<String>>);
    impl secpar_clipboard::Backend for Fake {
        fn read(&mut self) -> Option<String> {
            Some(self.0.lock().unwrap().clone())
        }
        fn write(&mut self, text: &str) -> bool {
            *self.0.lock().unwrap() = text.into();
            true
        }
    }
    fn clipboard(os: Arc<StdMutex<String>>) -> Arc<Shared> {
        Shared::with(true, move || Bridge::on({ let os = os.clone(); move || Ok(Fake(os)) }))
    }

    #[test]
    fn nothing_is_shared_before_a_copy_and_nothing_is_read_while_off() {
        let os = Arc::new(StdMutex::new("Old".into()));
        let shared = clipboard(os.clone());
        shared.watch(false, true);
        // The bridge's own thread must have read the clipboard once before a
        // change can be told apart from what was already there.
        std::thread::sleep(Duration::from_millis(200));
        shared.read();
        assert_eq!(shared.copied(), None, "the text from before is not shared");
        *os.lock().unwrap() = "New".into();
        wait(|| {
            shared.read();
            shared.copied().as_deref() == Some("New")
        });
        assert!(shared.put("From the other computer"));
        wait(|| *os.lock().unwrap() == "From the other computer");
        // Off: the OS clipboard is left alone again.
        shared.set_sharing(false);
        *os.lock().unwrap() = "Copied while off".into();
        shared.watch(false, true);
        shared.read();
        assert_eq!(shared.copied(), None);
        assert!(!shared.put("Not written"));
        assert_eq!(*os.lock().unwrap(), "Copied while off");
    }

    #[test]
    fn oversized_text_is_not_written() {
        let os = Arc::new(StdMutex::new("Old".into()));
        let shared = clipboard(os.clone());
        shared.watch(false, true);
        assert!(!shared.put(&"x".repeat(secpar_clipboard::MAX_TEXT + 1)));
        assert_eq!(*os.lock().unwrap(), "Old");
    }

    fn wait(mut done: impl FnMut() -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !done() {
            assert!(std::time::Instant::now() < end, "did not happen in time");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
