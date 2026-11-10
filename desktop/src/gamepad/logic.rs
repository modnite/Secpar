//! Controller logic shared by every platform source: scaling, dead zones, change detection,
//! hot plug slots and the `pad` / `pad.off` messages. No I/O happens here, so all of it is tested.

use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Pads per session. The wire protocol and both hosts use ids 0..=3.
pub const MAX_PADS: usize = 4;

pub const DPAD_UP: u16 = 0x0001;
pub const DPAD_DOWN: u16 = 0x0002;
pub const DPAD_LEFT: u16 = 0x0004;
pub const DPAD_RIGHT: u16 = 0x0008;
pub const START: u16 = 0x0010;
pub const BACK: u16 = 0x0020;
pub const LS: u16 = 0x0040;
pub const RS: u16 = 0x0080;
pub const LB: u16 = 0x0100;
pub const RB: u16 = 0x0200;
pub const GUIDE: u16 = 0x0400;
pub const A: u16 = 0x1000;
pub const B: u16 = 0x2000;
pub const X: u16 = 0x4000;
pub const Y: u16 = 0x8000;

/// One controller in XInput terms, exactly what the `pad` message carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    pub btn: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

impl State {
    pub fn is_neutral(&self) -> bool {
        *self == State::default()
    }
}

/// Maps `value` from `min..=max` onto the full signed 16 bit range.
pub fn axis(value: i32, min: i32, max: i32) -> i16 {
    if max <= min {
        return 0;
    }
    let t = (value.clamp(min, max) - min) as i64;
    (t * 65535 / (max - min) as i64 - 32768).clamp(-32768, 32767) as i16
}

/// Maps `value` from `min..=max` onto 0..=255.
pub fn trigger(value: i32, min: i32, max: i32) -> u8 {
    if max <= min {
        return 0;
    }
    ((value.clamp(min, max) - min) as i64 * 255 / (max - min) as i64) as u8
}

/// Radial dead zone. Inside `dead` (0..32767) the stick reads centre. Outside it the
/// deflection is stretched so a full push still reaches full range.
pub fn stick(x: i16, y: i16, dead: i16) -> (i16, i16) {
    let (fx, fy) = (x as f64, y as f64);
    let m = fx.hypot(fy);
    let dead = dead.max(0) as f64;
    if m <= dead || m == 0.0 {
        return (0, 0);
    }
    if m >= 32767.0 {
        return (x, y);
    }
    let scaled = ((m - dead) / (32767.0 - dead) * 32767.0).min(32767.0);
    let k = scaled / m;
    ((fx * k).round().clamp(-32768.0, 32767.0) as i16, (fy * k).round().clamp(-32768.0, 32767.0) as i16)
}

/// Flips an axis whose positive direction is down (evdev) into XInput's up-is-positive.
pub fn flip(v: i16) -> i16 {
    v.saturating_neg()
}

/// What the hub tells the caller to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Out {
    State(u8, State),
    Off(u8),
}

impl Out {
    pub fn json(&self) -> Value {
        match *self {
            Out::State(id, s) => {
                json!({"t":"pad","id":id,"btn":s.btn,"lt":s.lt,"rt":s.rt,"lx":s.lx,"ly":s.ly,"rx":s.rx,"ry":s.ry})
            }
            Out::Off(id) => json!({"t":"pad.off","id":id}),
        }
    }
    pub fn id(&self) -> u8 {
        match *self {
            Out::State(id, _) | Out::Off(id) => id,
        }
    }
}

/// Change-only sending with a short refresh. Input travels over UDP and the host sees only the
/// latest state, so a lost release would leave a button stuck. After a change the state is
/// repeated a few times quickly, and a held (non neutral) state is refreshed every 250 ms.
struct Tracker {
    last: Option<State>,
    at: Instant,
    quick: u8,
}

const QUICK_GAP: Duration = Duration::from_millis(40);
const HELD_GAP: Duration = Duration::from_millis(250);

impl Tracker {
    fn new(now: Instant) -> Self {
        Tracker { last: None, at: now, quick: 0 }
    }
    fn offer(&mut self, s: State, now: Instant) -> bool {
        let changed = self.last != Some(s);
        let gap = now.saturating_duration_since(self.at);
        let due = if changed {
            true
        } else if self.quick > 0 {
            gap >= QUICK_GAP
        } else {
            !s.is_neutral() && gap >= HELD_GAP
        };
        if !due {
            return false;
        }
        self.quick = if changed { 2 } else { self.quick.saturating_sub(1) };
        self.last = Some(s);
        self.at = now;
        true
    }
}

/// Hot plug bookkeeping: which physical device owns which pad id, and what to send for it.
#[derive(Default)]
pub struct Hub {
    slots: [Option<(u64, Tracker)>; MAX_PADS],
}

impl Hub {
    /// `devices` is every controller present right now, keyed by something stable for the
    /// lifetime of the connection. Returns the messages to send. New devices take the lowest free
    /// id, devices that went away free theirs and produce `Off`. Extra devices beyond four wait.
    pub fn update(&mut self, devices: &[(u64, State)], now: Instant) -> Vec<Out> {
        let mut out = Vec::new();
        for (id, slot) in self.slots.iter_mut().enumerate() {
            if slot.as_ref().is_some_and(|(key, _)| !devices.iter().any(|(k, _)| k == key)) {
                *slot = None;
                out.push(Out::Off(id as u8));
            }
        }
        for (key, state) in devices {
            let id = match self.slots.iter().position(|s| s.as_ref().is_some_and(|(k, _)| k == key)) {
                Some(id) => id,
                None => match self.slots.iter().position(Option::is_none) {
                    Some(id) => {
                        self.slots[id] = Some((*key, Tracker::new(now)));
                        id
                    }
                    None => continue,
                },
            };
            if let Some((_, tracker)) = self.slots[id].as_mut() {
                if tracker.offer(*state, now) {
                    out.push(Out::State(id as u8, *state));
                }
            }
        }
        out
    }

    /// Focus lost or the stream ended: every pad goes away so no input stays held on the host.
    pub fn release_all(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        for (id, slot) in self.slots.iter_mut().enumerate() {
            if slot.take().is_some() {
                out.push(Out::Off(id as u8));
            }
        }
        out
    }

    pub fn key_of(&self, id: u8) -> Option<u64> {
        self.slots.get(id as usize)?.as_ref().map(|(k, _)| *k)
    }

    pub fn count(&self) -> usize {
        self.slots.iter().flatten().count()
    }
}

/// Merges a new message into a pending list so a stalled UI never builds a backlog: a newer
/// state for a pad replaces an older one that was not sent yet.
pub fn coalesce(pending: &mut Vec<Out>, new: Out) {
    if let Out::State(id, _) = new {
        if let Some(i) = pending.iter().rposition(|o| o.id() == id) {
            if matches!(pending[i], Out::State(..)) {
                pending[i] = new;
                return;
            }
        }
    }
    pending.push(new);
}

/// Rumble from the host (large and small motor, 0..=255) as XInput / evdev 16 bit strengths.
pub fn rumble16(lo: u8, hi: u8) -> (u16, u16) {
    (lo as u16 * 257, hi as u16 * 257)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> Instant {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_millis(ms)
    }
    fn s(btn: u16) -> State {
        State { btn, ..State::default() }
    }

    #[test]
    fn axis_scaling_covers_the_whole_range() {
        assert_eq!(axis(-128, -128, 127), -32768);
        assert_eq!(axis(127, -128, 127), 32767);
        assert!((axis(0, -128, 127) as i32).abs() <= 130);
        assert_eq!(axis(0, 0, 255), -32768);
        assert_eq!(axis(1000, 0, 255), 32767);
        assert_eq!(axis(5, 5, 5), 0);
    }

    #[test]
    fn trigger_scaling() {
        assert_eq!(trigger(0, 0, 255), 0);
        assert_eq!(trigger(255, 0, 255), 255);
        assert_eq!(trigger(512, 0, 1023), 127);
        assert_eq!(trigger(-5, 0, 1023), 0);
        assert_eq!(trigger(4000, 0, 1023), 255);
    }

    #[test]
    fn dead_zone_is_radial_and_keeps_full_range() {
        assert_eq!(stick(2000, 2000, 4000), (0, 0));
        assert_eq!(stick(0, 0, 4000), (0, 0));
        assert_eq!(stick(32767, 0, 4000), (32767, 0));
        assert_eq!(stick(-32768, 0, 4000).0, -32768);
        assert_eq!(stick(32767, -32768, 4000), (32767, -32768), "a full corner is not pulled in");
        let (x, y) = stick(20000, 20000, 4000);
        assert!(x > 0 && x == y && x < 20000 + 1);
        // a diagonal is not cut off by the corner of a square dead zone
        assert_ne!(stick(3000, 3000, 4000), (0, 0));
        // monotonic
        assert!(stick(10000, 0, 4000).0 < stick(12000, 0, 4000).0);
        assert_eq!(flip(i16::MIN), i16::MAX);
        assert_eq!(flip(100), -100);
    }

    #[test]
    fn messages_match_the_wire_format() {
        let m = Out::State(2, State { btn: 0x1000, lt: 0, rt: 255, lx: -1, ly: 2, rx: 3, ry: -32768 }).json();
        assert_eq!(m, json!({"t":"pad","id":2,"btn":4096,"lt":0,"rt":255,"lx":-1,"ly":2,"rx":3,"ry":-32768}));
        assert_eq!(Out::Off(3).json(), json!({"t":"pad.off","id":3}));
    }

    #[test]
    fn message_round_trips_through_json_text() {
        let m = Out::State(0, State { btn: 4096, lt: 0, rt: 255, lx: 0, ly: -32768, rx: 1200, ry: 0 }).json();
        let back: Value = serde_json::from_str(&m.to_string()).unwrap();
        assert_eq!((back["t"].as_str(), back["btn"].as_i64(), back["rt"].as_i64(), back["ly"].as_i64()), (Some("pad"), Some(4096), Some(255), Some(-32768)));
    }

    #[test]
    fn only_changes_are_sent_then_a_short_refresh() {
        let mut hub = Hub::default();
        let dev = |st| vec![(7u64, st)];
        assert_eq!(hub.update(&dev(s(0)), t(0)), vec![Out::State(0, s(0))]);
        // nothing changed: nothing sent until the quick repeats are due
        assert!(hub.update(&dev(s(0)), t(10)).is_empty());
        assert_eq!(hub.update(&dev(s(0)), t(45)).len(), 1);
        assert_eq!(hub.update(&dev(s(0)), t(90)).len(), 1);
        // a neutral pad then stays quiet
        assert!(hub.update(&dev(s(0)), t(150)).is_empty());
        assert!(hub.update(&dev(s(0)), t(5000)).is_empty());
        // a press goes out at once
        assert_eq!(hub.update(&dev(s(A)), t(5001)), vec![Out::State(0, s(A))]);
        // a held button is refreshed, not spammed
        assert!(hub.update(&dev(s(A)), t(5020)).is_empty());
        let mut sent = 0;
        for ms in (5000..7000).step_by(8) {
            sent += hub.update(&dev(s(A)), t(ms + 2)).len();
        }
        assert!((6..=10).contains(&sent), "{sent}");
    }

    #[test]
    fn hot_plug_takes_lowest_id_and_unplug_sends_off() {
        let mut hub = Hub::default();
        let a = (10u64, s(A));
        let b = (20u64, s(B));
        let c = (30u64, s(X));
        assert_eq!(hub.update(&[a, b], t(0)), vec![Out::State(0, s(A)), Out::State(1, s(B))]);
        // first pad unplugged
        let out = hub.update(&[(20, s(B))], t(100));
        assert_eq!(out.iter().filter(|o| matches!(o, Out::Off(_))).collect::<Vec<_>>(), vec![&Out::Off(0)]);
        assert_eq!(hub.key_of(1), Some(20));
        assert_eq!(hub.key_of(0), None);
        // a new pad reuses id 0, pad 20 keeps id 1
        let out = hub.update(&[(20, s(B)), c], t(200));
        assert!(out.contains(&Out::State(0, s(X))));
        assert!(!out.iter().any(|o| o.id() == 1 && matches!(o, Out::Off(_))));
        assert_eq!(hub.count(), 2);
    }

    #[test]
    fn at_most_four_pads_and_extra_ones_wait_for_a_free_id() {
        let mut hub = Hub::default();
        let devs: Vec<_> = (0..6u64).map(|k| (k, s(A))).collect();
        let out = hub.update(&devs, t(0));
        assert_eq!(out.len(), 4);
        assert_eq!(hub.count(), 4);
        // device 1 leaves: device 4 takes id 1
        let rest: Vec<_> = devs.iter().copied().filter(|(k, _)| *k != 1).collect();
        let out = hub.update(&rest, t(10));
        assert!(out.contains(&Out::Off(1)));
        assert!(out.contains(&Out::State(1, s(A))));
    }

    #[test]
    fn release_all_sends_off_once() {
        let mut hub = Hub::default();
        hub.update(&[(1, s(A)), (2, s(B))], t(0));
        assert_eq!(hub.release_all(), vec![Out::Off(0), Out::Off(1)]);
        assert!(hub.release_all().is_empty());
        // plugged back in later: sent again from scratch
        assert_eq!(hub.update(&[(1, s(A))], t(10)), vec![Out::State(0, s(A))]);
    }

    #[test]
    fn coalesce_keeps_only_the_newest_state_per_pad() {
        let mut p = Vec::new();
        coalesce(&mut p, Out::State(0, s(A)));
        coalesce(&mut p, Out::State(1, s(B)));
        coalesce(&mut p, Out::State(0, s(X)));
        assert_eq!(p, vec![Out::State(0, s(X)), Out::State(1, s(B))]);
        coalesce(&mut p, Out::Off(0));
        coalesce(&mut p, Out::State(0, s(Y)));
        assert_eq!(p, vec![Out::State(0, s(X)), Out::State(1, s(B)), Out::Off(0), Out::State(0, s(Y))]);
    }

    #[test]
    fn rumble_scales_to_16_bits() {
        assert_eq!(rumble16(0, 255), (0, 65535));
    }
}
