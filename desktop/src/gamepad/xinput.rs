//! Windows controllers through XInput, loaded at run time from the system (xinput1_4.dll, or
//! xinput9_1_0.dll on old systems). Covers Xbox pads and anything that presents as one: Steam
//! Input, DS4Windows, 8BitDo and the like. Bare DualShock / DualSense pads without such a
//! wrapper are not XInput devices and are not covered.

use super::logic::{rumble16, State, MAX_PADS};
use super::Source;
use std::time::{Duration, Instant};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct XState {
    packet: u32,
    buttons: u16,
    lt: u8,
    rt: u8,
    lx: i16,
    ly: i16,
    rx: i16,
    ry: i16,
}

#[repr(C)]
struct Vibration {
    left: u16,
    right: u16,
}

type GetState = unsafe extern "system" fn(u32, *mut XState) -> u32;
type SetState = unsafe extern "system" fn(u32, *mut Vibration) -> u32;

const ERROR_SUCCESS: u32 = 0;

pub struct XInput {
    get: GetState,
    set: SetState,
    /// Last poll of each user index that had nothing plugged in. Asking an empty slot is slow on
    /// some systems, so empty slots are only looked at once a second.
    empty_since: [Option<Instant>; MAX_PADS],
    present: [bool; MAX_PADS],
    last: [State; MAX_PADS],
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

impl XInput {
    pub fn load() -> Option<XInput> {
        for dll in ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll"] {
            // SAFETY: plain system DLL load by name; the module is never unloaded.
            let module = unsafe { LoadLibraryW(wide(dll).as_ptr()) };
            if module.is_null() {
                continue;
            }
            // SAFETY: the exports have the documented signatures. Ordinal 100 is the undocumented
            // XInputGetStateEx, which also reports the Guide button. The plain export is the fallback.
            unsafe {
                let ex = GetProcAddress(module, 100usize as *const u8);
                let plain = GetProcAddress(module, b"XInputGetState\0".as_ptr());
                let set = GetProcAddress(module, b"XInputSetState\0".as_ptr());
                if let (Some(get), Some(set)) = (ex.or(plain), set) {
                    return Some(XInput {
                        get: std::mem::transmute::<_, GetState>(get),
                        set: std::mem::transmute::<_, SetState>(set),
                        empty_since: [None; MAX_PADS],
                        present: [false; MAX_PADS],
                        last: [State::default(); MAX_PADS],
                    });
                }
            }
        }
        None
    }
}

fn convert(s: &XState) -> State {
    let (lx, ly) = super::logic::stick(s.lx, s.ly, 1200);
    let (rx, ry) = super::logic::stick(s.rx, s.ry, 1200);
    State { btn: s.buttons, lt: s.lt, rt: s.rt, lx, ly, rx, ry }
}

impl Source for XInput {
    fn poll(&mut self) -> Vec<(u64, State)> {
        let now = Instant::now();
        let mut out = Vec::new();
        for user in 0..MAX_PADS {
            if !self.present[user] && self.empty_since[user].is_some_and(|t| now.duration_since(t) < Duration::from_secs(1)) {
                continue;
            }
            let mut raw = XState::default();
            // SAFETY: `raw` is a valid XINPUT_STATE.
            let result = unsafe { (self.get)(user as u32, &mut raw) };
            if result == ERROR_SUCCESS {
                self.present[user] = true;
                self.empty_since[user] = None;
                self.last[user] = convert(&raw);
                out.push((user as u64, self.last[user]));
            } else {
                self.present[user] = false;
                self.empty_since[user] = Some(now);
            }
        }
        out
    }

    fn rumble(&mut self, key: u64, lo: u8, hi: u8) {
        let (left, right) = rumble16(lo, hi);
        let mut v = Vibration { left, right };
        // SAFETY: `v` is a valid XINPUT_VIBRATION.
        unsafe { (self.set)(key as u32, &mut v) };
    }

    fn silence(&mut self) {
        for user in 0..MAX_PADS {
            if self.present[user] {
                self.rumble(user as u64, 0, 0);
            }
        }
    }
}
