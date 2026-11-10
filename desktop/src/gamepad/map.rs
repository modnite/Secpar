//! evdev (Linux) gamepad events to Xbox layout. Pure logic: the file descriptors live in
//! `evdev.rs`. Buttons follow the kernel names, which is what the xpad, hid-playstation,
//! hid-sony, hid-nintendo and hid-steam drivers report. Position decides the Xbox button, so a
//! DualShock cross is A and a Switch pro controller's bottom button is A as well.

use super::logic::*;
use std::collections::BTreeSet;

pub const EV_SYN: u16 = 0;
pub const EV_KEY: u16 = 1;
pub const EV_ABS: u16 = 3;
pub const SYN_DROPPED: u16 = 3;

const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_TL2: u16 = 0x138;
const BTN_TR2: u16 = 0x139;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
const BTN_THUMBL: u16 = 0x13d;
const BTN_THUMBR: u16 = 0x13e;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;

pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;
pub const ABS_Z: u16 = 0x02;
pub const ABS_RX: u16 = 0x03;
pub const ABS_RY: u16 = 0x04;
pub const ABS_RZ: u16 = 0x05;
pub const ABS_HAT0X: u16 = 0x10;
pub const ABS_HAT0Y: u16 = 0x11;

pub const VENDOR_SONY: u16 = 0x054c;
pub const VENDOR_NINTENDO: u16 = 0x057e;

/// Smallest stick dead zone (about 3.6%). Cheap pads jitter around centre and would otherwise
/// send a message on every poll.
const NOISE: i16 = 1200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbsInfo {
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub flat: i32,
}

/// A device is a gamepad when it has the south face button and a left stick.
pub fn is_gamepad(has_key: impl Fn(u16) -> bool, has_abs: impl Fn(u16) -> bool) -> bool {
    has_key(BTN_SOUTH) && has_abs(ABS_X) && has_abs(ABS_Y)
}

pub struct Mapper {
    /// Sony and Nintendo drivers name the top face button BTN_NORTH; xpad names the left one so.
    positional: bool,
    pressed: BTreeSet<u16>,
    abs: [Option<AbsInfo>; 0x12],
    /// Right stick on Z / RZ (older generic pads) instead of RX / RY.
    right_on_zrz: bool,
    abs_triggers: bool,
}

impl Mapper {
    /// `abs(code)` answers the axis info for devices that have the axis.
    pub fn new(vendor: u16, abs: impl Fn(u16) -> Option<AbsInfo>) -> Option<Mapper> {
        let mut table = [None; 0x12];
        for (code, slot) in table.iter_mut().enumerate() {
            *slot = abs(code as u16);
        }
        table[ABS_X as usize]?;
        table[ABS_Y as usize]?;
        let has = |c: u16| table[c as usize].is_some();
        let right_on_zrz = !(has(ABS_RX) && has(ABS_RY)) && has(ABS_Z) && has(ABS_RZ);
        Some(Mapper {
            positional: matches!(vendor, VENDOR_SONY | VENDOR_NINTENDO),
            pressed: BTreeSet::new(),
            right_on_zrz,
            abs_triggers: has(ABS_Z) && has(ABS_RZ) && !right_on_zrz,
            abs: table,
        })
    }

    pub fn key(&mut self, code: u16, value: i32) {
        if value == 0 {
            self.pressed.remove(&code);
        } else {
            self.pressed.insert(code);
        }
    }

    pub fn axis(&mut self, code: u16, value: i32) {
        if let Some(Some(info)) = self.abs.get_mut(code as usize) {
            info.value = value;
        }
    }

    /// Replaces everything after a SYN_DROPPED (events were lost): `keys` are the pressed ones.
    pub fn resync(&mut self, keys: impl IntoIterator<Item = u16>, abs: impl Fn(u16) -> Option<AbsInfo>) {
        self.pressed = keys.into_iter().collect();
        for (code, slot) in self.abs.iter_mut().enumerate() {
            if slot.is_some() {
                *slot = abs(code as u16).or(*slot);
            }
        }
    }

    fn scaled(&self, code: u16) -> i16 {
        self.abs[code as usize].map(|a| axis(a.value, a.min, a.max)).unwrap_or(0)
    }

    pub fn state(&self) -> State {
        let down = |c: u16| self.pressed.contains(&c);
        let (top, left) = if self.positional { (BTN_NORTH, BTN_WEST) } else { (BTN_WEST, BTN_NORTH) };
        let mut btn = 0u16;
        for (key, mask) in [
            (BTN_SOUTH, A),
            (BTN_EAST, B),
            (left, X),
            (top, Y),
            (BTN_TL, LB),
            (BTN_TR, RB),
            (BTN_SELECT, BACK),
            (BTN_START, START),
            (BTN_MODE, GUIDE),
            (BTN_THUMBL, LS),
            (BTN_THUMBR, RS),
            (BTN_DPAD_UP, DPAD_UP),
            (BTN_DPAD_DOWN, DPAD_DOWN),
            (BTN_DPAD_LEFT, DPAD_LEFT),
            (BTN_DPAD_RIGHT, DPAD_RIGHT),
        ] {
            if down(key) {
                btn |= mask;
            }
        }
        if let Some(h) = self.abs[ABS_HAT0X as usize] {
            btn |= match h.value.signum() {
                -1 => DPAD_LEFT,
                1 => DPAD_RIGHT,
                _ => 0,
            };
        }
        if let Some(h) = self.abs[ABS_HAT0Y as usize] {
            btn |= match h.value.signum() {
                -1 => DPAD_UP,
                1 => DPAD_DOWN,
                _ => 0,
            };
        }
        let dead = |a: u16, b: u16| {
            let flat = [a, b].iter().filter_map(|c| self.abs[*c as usize]).map(|i| i.flat).max().unwrap_or(0);
            let range = self.abs[a as usize].map(|i| (i.max - i.min).max(1)).unwrap_or(65535);
            ((flat as i64 * 65535 / range as i64).clamp(0, 8000) as i16).max(NOISE)
        };
        let (lx, ly) = stick(self.scaled(ABS_X), flip(self.scaled(ABS_Y)), dead(ABS_X, ABS_Y));
        let (rxc, ryc) = if self.right_on_zrz { (ABS_Z, ABS_RZ) } else { (ABS_RX, ABS_RY) };
        let (rx, ry) = stick(self.scaled(rxc), flip(self.scaled(ryc)), dead(rxc, ryc));
        let trig = |abs: u16, key: u16| {
            if self.abs_triggers {
                self.abs[abs as usize].map(|a| trigger(a.value, a.min, a.max)).unwrap_or(0)
            } else if down(key) {
                255
            } else {
                0
            }
        };
        State { btn, lt: trig(ABS_Z, BTN_TL2), rt: trig(ABS_RZ, BTN_TR2), lx, ly, rx, ry }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn a(min: i32, max: i32, flat: i32) -> Option<AbsInfo> {
        Some(AbsInfo { value: 0, min, max, flat })
    }
    fn xbox(code: u16) -> Option<AbsInfo> {
        match code {
            ABS_X | ABS_Y | ABS_RX | ABS_RY => a(-32768, 32767, 128),
            ABS_Z | ABS_RZ => a(0, 255, 0),
            ABS_HAT0X | ABS_HAT0Y => a(-1, 1, 0),
            _ => None,
        }
    }
    fn mapper(vendor: u16, f: fn(u16) -> Option<AbsInfo>) -> Mapper {
        Mapper::new(vendor, f).unwrap()
    }

    #[test]
    fn detects_gamepads_only() {
        assert!(is_gamepad(|k| k == BTN_SOUTH, |c| c == ABS_X || c == ABS_Y));
        assert!(!is_gamepad(|_| false, |_| true)); // touchpad / accelerometer style devices
        assert!(!is_gamepad(|k| k == BTN_SOUTH, |c| c == ABS_X)); // no second axis
    }

    #[test]
    fn xbox_buttons_sticks_and_triggers() {
        let mut m = mapper(0x045e, xbox);
        m.key(BTN_SOUTH, 1);
        m.key(BTN_NORTH, 1); // xpad: this is the Xbox X button
        m.key(BTN_TL, 1);
        m.key(BTN_MODE, 1);
        m.axis(ABS_Z, 255);
        m.axis(ABS_RZ, 128);
        m.axis(ABS_X, 32767);
        m.axis(ABS_Y, -32768); // up
        let s = m.state();
        assert_eq!(s.btn, A | X | LB | GUIDE);
        assert_eq!((s.lt, s.rt), (255, 128));
        assert_eq!(s.lx, 32767);
        assert_eq!(s.ly, 32767, "up is positive in XInput");
        m.key(BTN_SOUTH, 0);
        m.key(BTN_NORTH, 0);
        assert_eq!(m.state().btn, LB | GUIDE);
    }

    #[test]
    fn playstation_and_switch_use_position() {
        for vendor in [VENDOR_SONY, VENDOR_NINTENDO] {
            let mut m = mapper(vendor, xbox);
            m.key(BTN_NORTH, 1); // top button
            assert_eq!(m.state().btn, Y);
            m.key(BTN_NORTH, 0);
            m.key(BTN_WEST, 1); // left button
            assert_eq!(m.state().btn, X);
        }
        let mut m = mapper(0x045e, xbox);
        m.key(BTN_WEST, 1);
        assert_eq!(m.state().btn, Y);
    }

    #[test]
    fn hat_and_dpad_buttons_both_work() {
        let mut m = mapper(0x045e, xbox);
        m.axis(ABS_HAT0X, -1);
        m.axis(ABS_HAT0Y, 1);
        assert_eq!(m.state().btn, DPAD_LEFT | DPAD_DOWN);
        m.axis(ABS_HAT0X, 0);
        m.axis(ABS_HAT0Y, 0);
        m.key(BTN_DPAD_UP, 1);
        assert_eq!(m.state().btn, DPAD_UP);
    }

    #[test]
    fn digital_triggers_when_no_trigger_axes() {
        let mut m = mapper(VENDOR_NINTENDO, |c| match c {
            ABS_X | ABS_Y | ABS_RX | ABS_RY => a(-32767, 32767, 250),
            _ => None,
        });
        m.key(BTN_TL2, 1);
        assert_eq!((m.state().lt, m.state().rt), (255, 0));
        m.key(BTN_TR2, 1);
        assert_eq!(m.state().rt, 255);
    }

    #[test]
    fn old_generic_pads_put_the_right_stick_on_z_rz() {
        let mut m = mapper(0x0079, |c| match c {
            ABS_X | ABS_Y | ABS_Z | ABS_RZ => a(0, 255, 15),
            _ => None,
        });
        m.axis(ABS_X, 128);
        m.axis(ABS_Y, 128);
        m.axis(ABS_Z, 255);
        m.axis(ABS_RZ, 128);
        let s = m.state();
        assert_eq!((s.lt, s.rt), (0, 0));
        assert!(s.rx > 30000, "{s:?}");
        assert_eq!(s.lx.abs() < 1300, true);
    }

    #[test]
    fn centre_noise_is_ignored() {
        let mut m = mapper(0x045e, xbox);
        m.axis(ABS_X, 400);
        m.axis(ABS_RY, -300);
        assert!(m.state().is_neutral());
    }

    #[test]
    fn resync_replaces_stale_state() {
        let mut m = mapper(0x045e, xbox);
        m.key(BTN_SOUTH, 1);
        m.axis(ABS_Z, 255);
        m.resync([BTN_EAST], |c| xbox(c));
        let s = m.state();
        assert_eq!(s.btn, B);
        assert_eq!(s.lt, 0);
    }

    #[test]
    fn missing_left_stick_is_not_a_gamepad() {
        assert!(Mapper::new(0, |c| (c == ABS_X).then(|| AbsInfo { value: 0, min: 0, max: 255, flat: 0 })).is_none());
    }
}
