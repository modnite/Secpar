//! Remote keyboard and pointer input through CGEvent. Needs the Accessibility permission.
//! macOS only; not compiled or run anywhere yet.
use crate::{
    keymap::{mac_key, mouse_button},
    media::InputSink,
};
use secpar_host_core::proto::ClientMsg;
use std::{collections::HashSet, ffi::c_void};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: f64,
    y: f64,
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> Point;
    fn CGEventCreateMouseEvent(source: *mut c_void, kind: u32, at: Point, button: u32) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventCreateScrollWheelEvent2(source: *mut c_void, units: u32, count: u32, w1: i32, w2: i32, w3: i32) -> *mut c_void;
    fn CGEventKeyboardSetUnicodeString(event: *mut c_void, len: usize, text: *const u16);
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(object: *const c_void);
}
const HID_SYSTEM_STATE: i32 = 1;
const HID_TAP: u32 = 0;
const FIELD_DX: u32 = 4;
const FIELD_DY: u32 = 5;
const FIELD_BUTTON: u32 = 3;

pub struct MacInput {
    source: *mut c_void,
    /// The streamed display in global points: x, y, width, height.
    rect: [f64; 4],
    pos: Point,
    buttons: HashSet<u32>,
    keys: HashSet<u16>,
}
// SAFETY: used from the single input thread that owns it.
unsafe impl Send for MacInput {}
impl MacInput {
    pub fn new(rect: [i32; 4]) -> Self {
        unsafe {
            let source = CGEventSourceCreate(HID_SYSTEM_STATE);
            let current = CGEventCreate(std::ptr::null_mut());
            let pos = if current.is_null() { Point::default() } else { CGEventGetLocation(current) };
            if !current.is_null() {
                CFRelease(current);
            }
            Self { source, rect: rect.map(f64::from), pos, buttons: HashSet::new(), keys: HashSet::new() }
        }
    }
    fn post(&self, event: *mut c_void) {
        if !event.is_null() {
            unsafe {
                CGEventPost(HID_TAP, event);
                CFRelease(event);
            }
        }
    }
    fn mouse(&mut self, at: Point, delta: (i32, i32)) {
        let [x, y, w, h] = self.rect;
        self.pos = Point { x: at.x.clamp(x, x + (w - 1.0).max(0.0)), y: at.y.clamp(y, y + (h - 1.0).max(0.0)) };
        // Held buttons make this a drag; otherwise a plain move.
        let (kind, button) = match self.buttons.iter().min() {
            Some(0) => (6, 0),
            Some(1) => (7, 1),
            Some(b) => (27, *b),
            None => (5, 0),
        };
        unsafe {
            let e = CGEventCreateMouseEvent(self.source, kind, self.pos, button);
            if !e.is_null() {
                CGEventSetIntegerValueField(e, FIELD_DX, delta.0 as i64);
                CGEventSetIntegerValueField(e, FIELD_DY, delta.1 as i64);
            }
            self.post(e);
        }
    }
    fn button(&mut self, button: u32, down: bool) {
        let kind = match (button, down) {
            (0, true) => 1,
            (0, false) => 2,
            (1, true) => 3,
            (1, false) => 4,
            (_, true) => 25,
            (_, false) => 26,
        };
        if down {
            self.buttons.insert(button);
        } else {
            self.buttons.remove(&button);
        }
        unsafe {
            let e = CGEventCreateMouseEvent(self.source, kind, self.pos, button.min(31));
            if !e.is_null() && button > 1 {
                CGEventSetIntegerValueField(e, FIELD_BUTTON, button as i64);
            }
            self.post(e);
        }
    }
    fn key(&mut self, code: u16, down: bool) {
        if down {
            self.keys.insert(code);
        } else {
            self.keys.remove(&code);
        }
        unsafe { self.post(CGEventCreateKeyboardEvent(self.source, code, down)) }
    }
    fn text(&mut self, s: &str) {
        for unit in s.chars().take(1000) {
            let mut buf = [0u16; 2];
            let units = unit.encode_utf16(&mut buf);
            for down in [true, false] {
                unsafe {
                    let e = CGEventCreateKeyboardEvent(self.source, 0, down);
                    if !e.is_null() {
                        CGEventKeyboardSetUnicodeString(e, units.len(), units.as_ptr());
                    }
                    self.post(e);
                }
            }
        }
    }
}
impl InputSink for MacInput {
    fn apply(&mut self, message: ClientMsg) {
        match message.sanitize() {
            ClientMsg::MouseMove { dx, dy } => {
                let at = Point { x: self.pos.x + dx as f64, y: self.pos.y + dy as f64 };
                self.mouse(at, (dx, dy));
            }
            ClientMsg::MousePos { x, y } => {
                let [rx, ry, w, h] = self.rect;
                let at = Point { x: rx + x * (w - 1.0).max(0.0), y: ry + y * (h - 1.0).max(0.0) };
                self.mouse(at, (0, 0));
            }
            ClientMsg::MouseButton { b, down } => {
                if let Some(button) = mouse_button(&b) {
                    self.button(button, down);
                }
            }
            ClientMsg::MouseScroll { dx, dy } => unsafe {
                // Windows wheel steps are 120 per notch; one line per notch here, natural direction left to the OS.
                let notch = |v: i32| if v == 0 { 0 } else { (v / 120).clamp(-20, 20).max(if v > 0 { 1 } else { -20 }).min(if v > 0 { 20 } else { -1 }) };
                let e = CGEventCreateScrollWheelEvent2(self.source, 1, 2, notch(dy), -notch(dx), 0);
                self.post(e);
            },
            ClientMsg::Key { vk, down } => {
                if let Some(code) = mac_key(vk) {
                    self.key(code, down);
                }
            }
            ClientMsg::Text { s } => self.text(&s),
            _ => {}
        }
    }
    fn release_all(&mut self) {
        for code in std::mem::take(&mut self.keys) {
            unsafe { self.post(CGEventCreateKeyboardEvent(self.source, code, false)) }
        }
        for button in std::mem::take(&mut self.buttons) {
            self.button(button, false);
        }
    }
}
impl Drop for MacInput {
    fn drop(&mut self) {
        self.release_all();
        if !self.source.is_null() {
            unsafe { CFRelease(self.source) }
        }
    }
}
