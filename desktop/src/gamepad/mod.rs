//! Physical controller capture for the desktop client. A background thread polls the system's
//! controllers (XInput on Windows, evdev on Linux) at about 125 Hz while a stream is on screen and
//! focused. The UI thread collects the resulting `pad` / `pad.off` messages and sends them over
//! the existing input channel. macOS has no source yet.

pub mod logic;
pub mod map;

#[cfg(target_os = "linux")]
pub mod evdev;
#[cfg(windows)]
pub mod xinput;

use logic::{coalesce, Hub, Out, State};
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Where controller state comes from. Keys identify one physical device while it stays plugged in.
pub trait Source: Send {
    fn poll(&mut self) -> Vec<(u64, State)>;
    fn rumble(&mut self, key: u64, lo: u8, hi: u8);
    /// Stops all vibration.
    fn silence(&mut self);
}

fn make_source() -> Option<Box<dyn Source>> {
    #[cfg(windows)]
    return xinput::XInput::load().map(|s| Box::new(s) as Box<dyn Source>);
    #[cfg(target_os = "linux")]
    return Some(Box::new(evdev::Evdev::new()));
    #[cfg(not(any(windows, target_os = "linux")))]
    None
}

#[derive(Default)]
struct Shared {
    active: AtomicBool,
    stop: AtomicBool,
    pending: Mutex<Vec<Out>>,
    rumble: Mutex<Vec<(u8, u8, u8)>>,
    pads: AtomicUsize,
}

#[derive(Default)]
pub struct Runner {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Runner {
    /// Capture on or off. The thread starts the first time capture is switched on. Switching off
    /// releases every pad on the host (a `pad.off` for each) and stops vibration.
    /// `wake` is called from the capture thread when messages are waiting (a UI repaint).
    pub fn set_active(&mut self, on: bool, wake: impl Fn() + Send + 'static) {
        self.set_active_with(on, wake, make_source);
    }

    /// Like `set_active` with a chosen source, for tests.
    pub fn set_active_with(
        &mut self,
        on: bool,
        wake: impl Fn() + Send + 'static,
        make: impl FnOnce() -> Option<Box<dyn Source>> + Send + 'static,
    ) {
        if self.shared.active.swap(on, Ordering::Relaxed) == on {
            return;
        }
        if on && self.thread.is_none() {
            let shared = self.shared.clone();
            self.thread = std::thread::Builder::new().name("secpar-pads".into()).spawn(move || run(shared, wake, make)).ok();
        }
    }

    /// Messages ready to send, oldest first.
    pub fn take(&self) -> Vec<Value> {
        std::mem::take(&mut *self.shared.pending.lock().unwrap()).iter().map(Out::json).collect()
    }

    /// Rumble from the host for pad `id` (0..=3).
    pub fn rumble(&self, id: u8, lo: u8, hi: u8) {
        let mut q = self.shared.rumble.lock().unwrap();
        q.retain(|r| r.0 != id);
        q.push((id, lo, hi));
    }

    /// Controllers currently forwarded.
    pub fn connected(&self) -> usize {
        self.shared.pads.load(Ordering::Relaxed)
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(windows)]
struct FastTimer(bool);
#[cfg(windows)]
impl FastTimer {
    fn set(&mut self, on: bool) {
        use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
        if on != self.0 {
            // SAFETY: balanced begin / end calls for a 1 ms timer resolution.
            unsafe { if on { timeBeginPeriod(1) } else { timeEndPeriod(1) } };
            self.0 = on;
        }
    }
}
#[cfg(windows)]
impl Drop for FastTimer {
    fn drop(&mut self) {
        self.set(false);
    }
}

fn run(shared: Arc<Shared>, wake: impl Fn(), make: impl FnOnce() -> Option<Box<dyn Source>>) {
    let Some(mut source) = make() else { return };
    let mut hub = Hub::default();
    let mut was_active = false;
    #[cfg(windows)]
    let mut timer = FastTimer(false);
    while !shared.stop.load(Ordering::Relaxed) {
        if !shared.active.load(Ordering::Relaxed) {
            if was_active {
                was_active = false;
                #[cfg(windows)]
                timer.set(false);
                source.silence();
                let off = hub.release_all();
                shared.pads.store(0, Ordering::Relaxed);
                shared.rumble.lock().unwrap().clear();
                if !off.is_empty() {
                    let mut p = shared.pending.lock().unwrap();
                    off.into_iter().for_each(|o| coalesce(&mut p, o));
                    wake();
                }
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if !was_active {
            was_active = true;
            #[cfg(windows)]
            timer.set(true);
        }
        let requests = std::mem::take(&mut *shared.rumble.lock().unwrap());
        for (id, lo, hi) in requests {
            if let Some(key) = hub.key_of(id) {
                source.rumble(key, lo, hi);
            }
        }
        let devices = source.poll();
        let out = hub.update(&devices, Instant::now());
        shared.pads.store(hub.count(), Ordering::Relaxed);
        if !out.is_empty() {
            let mut p = shared.pending.lock().unwrap();
            out.into_iter().for_each(|o| coalesce(&mut p, o));
            drop(p);
            wake();
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    source.silence();
}
