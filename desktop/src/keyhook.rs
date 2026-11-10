//! Windows low-level keyboard hook. It exists only while a stream is on screen, the window has
//! focus and the keys belong to the host. Then Win, Alt+Tab, Alt+F4 and the rest are swallowed
//! here and sent to the host as key events. The hook is removed on focus loss, disconnect,
//! window close, error or panic. A stuck hook would take the keyboard from the user, so the
//! callback also passes every key through when the window thread stops sending heartbeats.
use crate::keys::{Escape, EscapeKeys, Filter};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Without a heartbeat for this long the hook lets every key through.
pub const STALL: Duration = Duration::from_secs(2);
/// How often the window thread must beat while the hook is up.
pub const BEAT_EVERY: Duration = Duration::from_millis(500);

/// Delivers one input message to the stream. False when the queue is full: it is retried.
pub type Sink = Box<dyn FnMut(Value) -> bool + Send>;

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Escape(Escape),
    /// The window thread stopped beating. The hook passed keys through and wants removing.
    Stalled,
    /// Another window had the keyboard. The hook passed keys through and wants removing.
    FocusLost,
}

struct Active {
    filter: Filter,
    sink: Sink,
    pending: VecDeque<Value>,
    outcomes: Vec<Outcome>,
    wake: mpsc::Sender<()>,
    hwnd: usize,
    foreground: fn(usize) -> bool,
    /// Stalled or unfocused: everything passes until the hook is removed.
    tripped: bool,
}
impl Active {
    fn push(&mut self, message: Value) {
        self.pending.push_back(message);
        self.flush();
    }
    fn flush(&mut self) {
        while let Some(front) = self.pending.front() {
            if (self.sink)(front.clone()) {
                self.pending.pop_front();
            } else {
                break;
            }
        }
    }
    fn trip(&mut self, outcome: Outcome) {
        self.tripped = true;
        for (vk, down) in self.filter.release_all() {
            self.push(key(vk, down));
        }
        self.outcomes.push(outcome);
        let _ = self.wake.send(());
    }
    /// One key event from the hook. True means swallow it.
    fn dispatch(&mut self, vk: u16, down: bool, now: u64, beat: u64) -> bool {
        if self.tripped {
            return false;
        }
        if now.saturating_sub(beat) > STALL.as_millis() as u64 {
            self.trip(Outcome::Stalled);
            return false;
        }
        if !(self.foreground)(self.hwnd) {
            self.trip(Outcome::FocusLost);
            return false;
        }
        let verdict = self.filter.key(vk, down);
        for (vk, down) in verdict.send {
            self.push(key(vk, down));
        }
        if let Some(action) = verdict.escape {
            self.outcomes.push(Outcome::Escape(action));
            let _ = self.wake.send(());
        }
        verdict.swallow
    }
}
fn key(vk: u16, down: bool) -> Value {
    json!({"t":"key","vk":vk,"down":down})
}
fn is_foreground(hwnd: usize) -> bool {
    // SAFETY: plain query.
    unsafe { GetForegroundWindow() as usize == hwnd }
}

static HOOK: Mutex<Option<Active>> = Mutex::new(None);
static HANDLE: AtomicIsize = AtomicIsize::new(0);
static BEAT: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();
fn now_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}
/// Called by the window thread on every frame while the hook is up.
pub fn heartbeat() {
    BEAT.store(now_ms(), Ordering::Relaxed);
}

/// The callback body without the call into the next hook. False means let the key through.
fn handle(message: u32, info: &KBDLLHOOKSTRUCT) -> bool {
    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    if !down && !matches!(message, WM_KEYUP | WM_SYSKEYUP) {
        return false;
    }
    // Input injected by other software (on-screen keyboards, remote tools, input methods) is not ours.
    if info.flags & LLKHF_INJECTED != 0 || info.vkCode > 0xFF {
        return false;
    }
    // Never wait for the window thread: a held lock means pass the key through.
    let Ok(mut guard) = HOOK.try_lock() else { return false };
    let Some(active) = guard.as_mut() else { return false };
    active.dispatch(info.vkCode as u16, down, now_ms(), BEAT.load(Ordering::Relaxed))
}

unsafe extern "system" fn proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam != 0 {
        // SAFETY: for HC_ACTION the system passes a valid KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let swallow = std::panic::catch_unwind(|| handle(wparam as u32, info)).unwrap_or(false);
        if swallow {
            return 1;
        }
    }
    // SAFETY: forwards the untouched arguments.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Owns the hook. Dropping it removes the hook and lifts every key the host still holds.
pub struct KeyHook(PhantomData<*const ()>);
impl KeyHook {
    /// `wake` runs on a helper thread when the window should look at `take_outcomes` soon.
    pub fn install(hwnd: usize, escape: EscapeKeys, keyboard: bool, sink: Sink, wake: impl Fn() + Send + 'static) -> Option<Self> {
        Self::install_with(hwnd, escape, keyboard, sink, wake, is_foreground)
    }
    fn install_with(hwnd: usize, escape: EscapeKeys, keyboard: bool, sink: Sink, wake: impl Fn() + Send + 'static, foreground: fn(usize) -> bool) -> Option<Self> {
        if HANDLE.load(Ordering::SeqCst) != 0 {
            return None;
        }
        let already_down: Vec<u16> = (8u16..=254)
            // SAFETY: plain query of the asynchronous key state.
            .filter(|vk| unsafe { GetAsyncKeyState(*vk as i32) } as u16 & 0x8000 != 0)
            .collect();
        let (wake_tx, wake_rx) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            while wake_rx.recv().is_ok() {
                wake();
            }
        });
        heartbeat();
        *HOOK.lock().ok()? = Some(Active {
            filter: Filter::new(escape, keyboard, already_down),
            sink,
            pending: VecDeque::new(),
            outcomes: Vec::new(),
            wake: wake_tx,
            hwnd,
            foreground,
            tripped: false,
        });
        // SAFETY: a global low-level hook needs no DLL; the callback runs on this thread.
        let handle = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), GetModuleHandleW(std::ptr::null()), 0) };
        if handle.is_null() {
            if let Ok(mut g) = HOOK.lock() {
                *g = None;
            }
            return None;
        }
        HANDLE.store(handle as isize, Ordering::SeqCst);
        Some(Self(PhantomData))
    }
    pub fn take_outcomes(&self) -> Vec<Outcome> {
        HOOK.lock().ok().and_then(|mut g| g.as_mut().map(|a| std::mem::take(&mut a.outcomes))).unwrap_or_default()
    }
    /// Retries events the stream queue refused earlier.
    pub fn flush(&self) {
        if let Ok(mut g) = HOOK.lock() {
            if let Some(a) = g.as_mut() {
                a.flush();
            }
        }
    }
}
impl Drop for KeyHook {
    fn drop(&mut self) {
        let handle = HANDLE.swap(0, Ordering::SeqCst);
        if handle != 0 {
            // SAFETY: the handle came from SetWindowsHookExW and is removed once.
            unsafe { UnhookWindowsHookEx(handle as _) };
        }
        let active = HOOK.lock().ok().and_then(|mut g| g.take());
        if let Some(mut active) = active {
            for (vk, down) in active.filter.release_all() {
                active.push(key(vk, down));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    type Log = Arc<Mutex<Vec<Value>>>;
    fn active(foreground: fn(usize) -> bool) -> (Active, Log, mpsc::Receiver<()>) {
        let log: Log = Arc::default();
        let sink_log = log.clone();
        let (wake, rx) = mpsc::channel();
        let active = Active {
            filter: Filter::new(EscapeKeys::CtrlAltShift, true, []),
            sink: Box::new(move |v| {
                sink_log.lock().unwrap().push(v);
                true
            }),
            pending: VecDeque::new(),
            outcomes: vec![],
            wake,
            hwnd: 1,
            foreground,
            tripped: false,
        };
        (active, log, rx)
    }
    fn pairs(log: &Log) -> Vec<(u64, bool)> {
        log.lock().unwrap().iter().map(|v| (v["vk"].as_u64().unwrap(), v["down"].as_bool().unwrap())).collect()
    }
    fn yes(_: usize) -> bool {
        true
    }
    fn no(_: usize) -> bool {
        false
    }

    #[test]
    fn swallowed_keys_are_sent_in_order() {
        let (mut a, log, _rx) = active(yes);
        assert!(a.dispatch(0xA4, true, 10, 10));
        assert!(a.dispatch(0x09, true, 11, 10));
        assert!(a.dispatch(0x09, false, 12, 10));
        assert!(a.dispatch(0xA4, false, 13, 10));
        assert_eq!(pairs(&log), vec![(0xA4, true), (0x09, true), (0x09, false), (0xA4, false)]);
    }

    #[test]
    fn a_stalled_window_thread_passes_keys_and_lifts_the_host() {
        let (mut a, log, rx) = active(yes);
        assert!(a.dispatch(0x5B, true, 100, 100));
        assert!(!a.dispatch(0x41, true, 100 + 2001, 100), "no heartbeat for over two seconds");
        assert_eq!(pairs(&log), vec![(0x5B, true), (0x5B, false)]);
        assert_eq!(a.outcomes, vec![Outcome::Stalled]);
        assert!(rx.try_recv().is_ok());
        assert!(!a.dispatch(0x42, true, 100, 100), "stays passing once tripped");
    }

    #[test]
    fn just_inside_the_stall_limit_still_swallows() {
        let (mut a, _log, _rx) = active(yes);
        assert!(a.dispatch(0x41, true, 2100, 100));
    }

    #[test]
    fn losing_the_foreground_passes_keys_and_lifts_the_host() {
        let (mut a, log, _rx) = active(yes);
        assert!(a.dispatch(0xA4, true, 5, 5));
        a.foreground = no;
        assert!(!a.dispatch(0x09, true, 6, 5));
        assert_eq!(pairs(&log), vec![(0xA4, true), (0xA4, false)]);
        assert_eq!(a.outcomes, vec![Outcome::FocusLost]);
    }

    #[test]
    fn escape_chord_reports_once_and_never_reaches_the_host() {
        let (mut a, log, rx) = active(yes);
        for vk in [0xA2, 0xA4, 0xA0] {
            assert!(a.dispatch(vk, true, 1, 1));
        }
        assert!(a.dispatch(0x51, true, 1, 1));
        assert_eq!(a.outcomes, vec![Outcome::Escape(Escape::Disconnect)]);
        assert!(rx.try_recv().is_ok());
        let sent = pairs(&log);
        assert!(!sent.iter().any(|(vk, _)| *vk == 0x51), "Q never reaches the host");
        for vk in [0xA2, 0xA4, 0xA0] {
            assert!(sent.contains(&(vk, true)) && sent.contains(&(vk, false)));
        }
    }

    #[test]
    fn a_full_queue_keeps_events_until_it_drains() {
        let log: Log = Arc::default();
        let (sink_log, open) = (log.clone(), Arc::new(Mutex::new(false)));
        let gate = open.clone();
        let (wake, _rx) = mpsc::channel();
        let mut a = Active {
            filter: Filter::new(EscapeKeys::CtrlAltShift, true, []),
            sink: Box::new(move |v| {
                if !*gate.lock().unwrap() {
                    return false;
                }
                sink_log.lock().unwrap().push(v);
                true
            }),
            pending: VecDeque::new(),
            outcomes: vec![],
            wake,
            hwnd: 1,
            foreground: yes,
            tripped: false,
        };
        a.dispatch(0x41, true, 1, 1);
        a.dispatch(0x41, false, 1, 1);
        assert!(log.lock().unwrap().is_empty());
        *open.lock().unwrap() = true;
        a.flush();
        assert_eq!(pairs(&log), vec![(0x41, true), (0x41, false)]);
    }

    fn info(vk: u32, flags: u32) -> KBDLLHOOKSTRUCT {
        KBDLLHOOKSTRUCT { vkCode: vk, scanCode: 0, flags, time: 0, dwExtraInfo: 0 }
    }

    /// Installs the real hook, then drives the callback body with synthetic events. The
    /// system callback is never called with them, so no other hook in the chain and no
    /// application sees a key, and nothing is injected into the desktop.
    #[test]
    fn real_hook_installs_swallows_synthetic_events_and_uninstalls() {
        let log: Log = Arc::default();
        let sink_log = log.clone();
        let hook = KeyHook::install_with(
            7,
            EscapeKeys::CtrlAltShift,
            true,
            Box::new(move |v| {
                sink_log.lock().unwrap().push(v);
                true
            }),
            || {},
            |_| true,
        )
        .expect("hook installs");
        assert_ne!(HANDLE.load(Ordering::SeqCst), 0);
        assert!(KeyHook::install_with(7, EscapeKeys::default(), true, Box::new(|_| true), || {}, |_| true).is_none(), "only one hook at a time");

        assert!(handle(WM_SYSKEYDOWN, &info(0xA4, 0x20)), "Alt swallowed");
        assert!(handle(WM_SYSKEYDOWN, &info(0x09, 0x20)), "Alt+Tab swallowed");
        assert!(handle(WM_SYSKEYUP, &info(0x09, 0xA0)));
        assert!(handle(WM_KEYDOWN, &info(0x5B, 1)), "Win swallowed");
        assert!(!handle(WM_KEYDOWN, &info(0x41, 0x10)), "injected input is left alone");
        assert!(!handle(WM_KEYDOWN, &info(0xAF, 0)), "volume stays local");
        assert!(!handle(WM_KEYDOWN, &info(0x100, 0)));
        assert!(!handle(WM_CHAR, &info(0x41, 0)));
        assert_eq!(pairs(&log), vec![(0xA4, true), (0x09, true), (0x09, false), (0x5B, true)]);

        heartbeat();
        drop(hook);
        assert_eq!(HANDLE.load(Ordering::SeqCst), 0);
        assert!(HOOK.lock().unwrap().is_none());
        // The host got Alt and Win lifted on removal.
        let sent = pairs(&log);
        assert!(sent.contains(&(0xA4, false)) && sent.contains(&(0x5B, false)));
        assert!(!handle(WM_KEYDOWN, &info(0x41, 0)), "no hook, no swallowing");
    }
}
