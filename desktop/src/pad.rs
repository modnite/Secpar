//! Local controllers for a friend session. `gilrs` reads every pad and its SDL controller database
//! gives them one layout. This file maps that layout to the XInput one the host expects, gives
//! each pad an id from 0 to 3 and sends `pad` messages. Nothing here runs outside a friend session
//! that was granted `gamepad`.
use gilrs::{ev::EventType, ff, Axis, Button, Gilrs, MappingSource};
use secpar_client::session::Command;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc, Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

/// A pad that does not change is announced again this often.
pub const IDLE_REPEAT: Duration = Duration::from_secs(1);
/// Changes are sent at most this often per pad.
const MIN_GAP: Duration = Duration::from_millis(8);
/// Stick travel below this counts as centred. Games add their own dead zone on the host.
pub const STICK_DEAD_ZONE: f32 = 0.08;
/// Trigger travel below this counts as released.
pub const TRIGGER_DEAD_ZONE: f32 = 0.02;
/// The host takes at most four controllers.
pub const MAX_PADS: usize = 4;

const XINPUT_BUTTONS: [(Button, u16); 15] = [
    (Button::DPadUp, 0x0001),
    (Button::DPadDown, 0x0002),
    (Button::DPadLeft, 0x0004),
    (Button::DPadRight, 0x0008),
    (Button::Start, 0x0010),
    (Button::Select, 0x0020),
    (Button::LeftThumb, 0x0040),
    (Button::RightThumb, 0x0080),
    (Button::LeftTrigger, 0x0100),
    (Button::RightTrigger, 0x0200),
    (Button::Mode, 0x0400),
    (Button::South, 0x1000),
    (Button::East, 0x2000),
    (Button::West, 0x4000),
    (Button::North, 0x8000),
];

/// One controller as the host sees an XInput one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadState {
    pub btn: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}
impl PadState {
    pub fn message(&self, id: u8) -> Value {
        json!({"t":"pad","id":id,"btn":self.btn,"lt":self.lt,"rt":self.rt,"lx":self.lx,"ly":self.ly,"rx":self.rx,"ry":self.ry})
    }
}

fn to_i16(v: f32) -> i16 {
    if !v.is_finite() {
        return 0;
    }
    let v = v.clamp(-1.0, 1.0);
    if v >= 0.0 {
        (v * 32767.0).round() as i16
    } else {
        (v * 32768.0).round() as i16
    }
}
/// Round dead zone with the rest of the travel stretched over the full range. Up is positive on
/// `y`, as in gilrs and XInput.
pub fn stick(x: f32, y: f32) -> (i16, i16) {
    let (x, y) = (if x.is_finite() { x } else { 0.0 }, if y.is_finite() { y } else { 0.0 });
    let magnitude = x.hypot(y);
    if magnitude <= STICK_DEAD_ZONE {
        return (0, 0);
    }
    let scale = (magnitude.min(1.0) - STICK_DEAD_ZONE) / (1.0 - STICK_DEAD_ZONE) / magnitude;
    (to_i16(x * scale), to_i16(y * scale))
}
pub fn trigger(v: f32) -> u8 {
    if !v.is_finite() || v <= TRIGGER_DEAD_ZONE {
        return 0;
    }
    (v.min(1.0) * 255.0).round() as u8
}
/// The whole mapping, written against closures so it runs without a controller. `pressed` and
/// `analog` read gilrs buttons, `axis` reads gilrs axes.
pub fn map_state(pressed: impl Fn(Button) -> bool, analog: impl Fn(Button) -> f32, axis: impl Fn(Axis) -> f32) -> PadState {
    let mut btn = 0;
    for (button, bit) in XINPUT_BUTTONS {
        if pressed(button) {
            btn |= bit;
        }
    }
    // Some pads send the d-pad as a hat axis. gilrs turns that into buttons, but a pad that skips
    // the conversion still works.
    for (axis_id, low, high) in [(Axis::DPadX, 0x0004, 0x0008), (Axis::DPadY, 0x0002, 0x0001)] {
        let v = axis(axis_id);
        if v <= -0.5 {
            btn |= low;
        } else if v >= 0.5 {
            btn |= high;
        }
    }
    let level = |button| {
        let v = analog(button);
        trigger(if pressed(button) && v == 0.0 { 1.0 } else { v })
    };
    let (lx, ly) = stick(axis(Axis::LeftStickX), axis(Axis::LeftStickY));
    let (rx, ry) = stick(axis(Axis::RightStickX), axis(Axis::RightStickY));
    PadState { btn, lt: level(Button::LeftTrigger2), rt: level(Button::RightTrigger2), lx, ly, rx, ry }
}

/// What to send for one controller.
#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    State(u8, PadState),
    Off(u8),
}
#[derive(Clone, Default)]
struct Slot {
    uuid: [u8; 16],
    used: bool,
    live: Option<usize>,
    released: u64,
    last: Option<(PadState, Instant)>,
}
/// Gives pads ids and decides when a state goes out. Pure, so it is tested without hardware.
#[derive(Default)]
pub struct Tracker {
    slots: [Slot; MAX_PADS],
    clock: u64,
}
impl Tracker {
    /// Id for a controller that was plugged in. The first pad is 0, the next 1 and so on. A pad of
    /// the same model that was unplugged earlier gets its old id back. With four slots taken the
    /// pad that has been gone longest gives up its id. Four pads in use leaves no id.
    pub fn connect(&mut self, key: usize, uuid: [u8; 16]) -> Option<u8> {
        if let Some(id) = self.id_of(key) {
            return Some(id);
        }
        let pick = self
            .slots
            .iter()
            .position(|s| s.used && s.live.is_none() && s.uuid == uuid)
            .or_else(|| self.slots.iter().position(|s| !s.used))
            .or_else(|| {
                self.slots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.live.is_none())
                    .min_by_key(|(_, s)| s.released)
                    .map(|(i, _)| i)
            })?;
        self.slots[pick] = Slot { uuid, used: true, live: Some(key), released: 0, last: None };
        Some(pick as u8)
    }
    /// The id to take off the host when a pad is unplugged.
    pub fn disconnect(&mut self, key: usize) -> Option<u8> {
        let id = self.id_of(key)?;
        self.clock += 1;
        let slot = &mut self.slots[id as usize];
        slot.live = None;
        slot.released = self.clock;
        slot.last = None;
        Some(id)
    }
    pub fn id_of(&self, key: usize) -> Option<u8> {
        self.slots.iter().position(|s| s.live == Some(key)).map(|i| i as u8)
    }
    pub fn key_of(&self, id: u8) -> Option<usize> {
        self.slots.get(id as usize).and_then(|s| s.live)
    }
    pub fn live(&self) -> Vec<u8> {
        (0..MAX_PADS as u8).filter(|i| self.slots[*i as usize].live.is_some()).collect()
    }
    /// A state read just now. It goes out when it differs from the last one sent, or when the last
    /// one is a second old.
    pub fn observe(&mut self, key: usize, state: PadState, now: Instant) -> Option<Out> {
        let id = self.id_of(key)?;
        let slot = &mut self.slots[id as usize];
        let due = match slot.last {
            None => true,
            Some((last, at)) => {
                let age = now.saturating_duration_since(at);
                (last != state && age >= MIN_GAP) || age >= IDLE_REPEAT
            }
        };
        if !due {
            return None;
        }
        slot.last = Some((state, now));
        Some(Out::State(id, state))
    }
}

/// Runs while a friend session with `gamepad` is open. Dropping it stops the thread.
pub struct Pads {
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    rumble: std_mpsc::Sender<(u8, u8, u8)>,
    thread: Option<JoinHandle<()>>,
}
impl Pads {
    pub fn start(input: mpsc::Sender<Command>) -> Self {
        let (stop, failed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (rumble, rumble_rx) = std_mpsc::channel();
        let (s, f) = (stop.clone(), failed.clone());
        let thread = std::thread::Builder::new()
            .name("pads".into())
            .spawn(move || {
                if !run(input, &s, rumble_rx) {
                    f.store(true, Ordering::Relaxed);
                }
            })
            .ok();
        if thread.is_none() {
            failed.store(true, Ordering::Relaxed);
        }
        Self { stop, failed, rumble, thread }
    }
    /// Both motors at 0 stop the rumble.
    pub fn rumble(&self, id: u8, lo: u8, hi: u8) {
        let _ = self.rumble.send((id, lo, hi));
    }
    /// The controller library could not start on this system.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}
impl Drop for Pads {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn send(input: &mpsc::Sender<Command>, out: Out) -> bool {
    let value = match out {
        Out::State(id, state) => state.message(id),
        Out::Off(id) => json!({"t":"pad.off","id":id}),
    };
    // A full queue drops this state. The next change or the idle repeat carries the newest one.
    !matches!(input.try_send(Command::Input(value)), Err(mpsc::error::TrySendError::Closed(_)))
}
fn rumble_effect(gilrs: &mut Gilrs, pad: gilrs::GamepadId, lo: u8, hi: u8) -> Option<ff::Effect> {
    let length = ff::Ticks::from_ms(1000);
    let scheduling = ff::Replay { play_for: length, ..Default::default() };
    let effect = ff::EffectBuilder::new()
        .add_effect(ff::BaseEffect { kind: ff::BaseEffectType::Strong { magnitude: u16::from(lo) * 257 }, scheduling, ..Default::default() })
        .add_effect(ff::BaseEffect { kind: ff::BaseEffectType::Weak { magnitude: u16::from(hi) * 257 }, scheduling, ..Default::default() })
        .gamepads(&[pad])
        .repeat(ff::Repeat::Infinitely)
        .finish(gilrs)
        .ok()?;
    effect.play().ok()?;
    Some(effect)
}
/// False when the controller library cannot start.
fn run(input: mpsc::Sender<Command>, stop: &AtomicBool, rumble: std_mpsc::Receiver<(u8, u8, u8)>) -> bool {
    let Ok(mut gilrs) = Gilrs::new() else {
        return false;
    };
    let mut tracker = Tracker::default();
    let mut pads: HashMap<usize, gilrs::GamepadId> = HashMap::new();
    let mut effects: [Option<ff::Effect>; MAX_PADS] = Default::default();
    let attach = |gilrs: &Gilrs, tracker: &mut Tracker, pads: &mut HashMap<usize, gilrs::GamepadId>, id: gilrs::GamepadId| {
        let pad = gilrs.gamepad(id);
        if pad.is_connected() && pad.mapping_source() != MappingSource::None {
            if tracker.connect(usize::from(id), pad.uuid()).is_some() {
                pads.insert(usize::from(id), id);
            }
        }
    };
    let existing: Vec<_> = gilrs.gamepads().map(|(id, _)| id).collect();
    for id in existing {
        attach(&gilrs, &mut tracker, &mut pads, id);
    }
    let mut alive = true;
    while alive && !stop.load(Ordering::Relaxed) {
        let mut event = gilrs.next_event_blocking(Some(Duration::from_millis(4)));
        while let Some(e) = event {
            match e.event {
                EventType::Connected => attach(&gilrs, &mut tracker, &mut pads, e.id),
                EventType::Disconnected => {
                    let key = usize::from(e.id);
                    pads.remove(&key);
                    if let Some(id) = tracker.disconnect(key) {
                        effects[id as usize] = None;
                        alive &= send(&input, Out::Off(id));
                    }
                }
                _ => {}
            }
            event = gilrs.next_event();
        }
        while let Ok((id, lo, hi)) = rumble.try_recv() {
            let Some(slot) = effects.get_mut(id as usize) else { continue };
            *slot = None;
            let target = tracker.key_of(id).and_then(|key| pads.get(&key)).copied();
            if let Some(pad) = target.filter(|pad| (lo | hi) != 0 && gilrs.gamepad(*pad).is_ff_supported()) {
                effects[id as usize] = rumble_effect(&mut gilrs, pad, lo, hi);
            }
        }
        let now = Instant::now();
        let keys: Vec<_> = pads.iter().map(|(k, id)| (*k, *id)).collect();
        for (key, id) in keys {
            let pad = gilrs.gamepad(id);
            let state = map_state(
                |b| pad.is_pressed(b),
                |b| pad.button_data(b).map_or(0.0, |d| d.value()),
                |a| pad.value(a),
            );
            if let Some(out) = tracker.observe(key, state, now) {
                alive &= send(&input, out);
            }
        }
    }
    for id in tracker.live() {
        let _ = send(&input, Out::Off(id));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: Button) -> bool {
        false
    }
    fn flat(_: Button) -> f32 {
        0.0
    }
    fn centred(_: Axis) -> f32 {
        0.0
    }
    const A: [u8; 16] = [1; 16];
    const B: [u8; 16] = [2; 16];
    const C: [u8; 16] = [3; 16];

    #[test]
    fn the_south_button_is_the_a_button_of_the_contract_example() {
        let s = map_state(|b| b == Button::South, flat, centred);
        assert_eq!(s.btn, 4096);
        assert_eq!(s.message(0), json!({"t":"pad","id":0,"btn":4096,"lt":0,"rt":0,"lx":0,"ly":0,"rx":0,"ry":0}));
    }
    #[test]
    fn every_button_has_its_own_xinput_bit() {
        let mut seen = 0u16;
        for (button, bit) in XINPUT_BUTTONS {
            let s = map_state(|b| b == button, flat, centred);
            assert_eq!(s.btn, bit, "{button:?}");
            assert_eq!(seen & bit, 0);
            seen |= bit;
        }
        assert_eq!(seen, 0xF7FF);
        assert_eq!(map_state(none, flat, centred), PadState::default());
    }
    #[test]
    fn the_dpad_works_as_buttons_and_as_a_hat() {
        let up_right = map_state(|b| matches!(b, Button::DPadUp | Button::DPadRight), flat, centred);
        assert_eq!(up_right.btn, 0x0001 | 0x0008);
        let hat = |x: f32, y: f32| map_state(none, flat, move |a| if a == Axis::DPadX { x } else if a == Axis::DPadY { y } else { 0.0 }).btn;
        assert_eq!(hat(-1.0, 0.0), 0x0004);
        assert_eq!(hat(1.0, 0.0), 0x0008);
        assert_eq!(hat(0.0, 1.0), 0x0001);
        assert_eq!(hat(0.0, -1.0), 0x0002);
        assert_eq!(hat(0.3, -0.3), 0);
    }
    #[test]
    fn sticks_have_a_dead_zone_and_reach_both_ends() {
        assert_eq!(stick(0.0, 0.0), (0, 0));
        assert_eq!(stick(STICK_DEAD_ZONE, 0.0), (0, 0));
        assert_eq!(stick(0.05, -0.05), (0, 0));
        assert_eq!(stick(1.0, 0.0), (32767, 0));
        assert_eq!(stick(-1.0, 0.0), (-32768, 0));
        let (x, _) = stick(0.5, 0.0);
        assert!(x > 0 && x < 32767 / 2 + 1, "{x}");
        // The first bit of travel past the dead zone starts near zero, not with a jump.
        assert!(stick(STICK_DEAD_ZONE + 0.01, 0.0).0 < 400);
    }
    #[test]
    fn sticks_are_round_and_up_is_positive() {
        let (x, y) = stick(1.0, 1.0);
        assert_eq!((x, y), (23170, 23170));
        let state = map_state(none, flat, |a| match a {
            Axis::LeftStickY => 1.0,
            Axis::RightStickY => -1.0,
            _ => 0.0,
        });
        assert_eq!((state.lx, state.ly, state.rx, state.ry), (0, 32767, 0, -32768));
    }
    #[test]
    fn bad_numbers_never_become_stick_motion() {
        assert_eq!(stick(f32::NAN, f32::INFINITY), (0, 0));
        assert_eq!(stick(40.0, 0.0), (32767, 0));
        assert_eq!(trigger(f32::NAN), 0);
    }
    #[test]
    fn triggers_cover_zero_to_255_with_a_small_dead_zone() {
        assert_eq!(trigger(0.0), 0);
        assert_eq!(trigger(TRIGGER_DEAD_ZONE), 0);
        assert_eq!(trigger(0.5), 128);
        assert_eq!(trigger(1.0), 255);
        assert_eq!(trigger(7.0), 255);
        assert_eq!(trigger(-1.0), 0);
        let s = map_state(none, |b| if b == Button::RightTrigger2 { 1.0 } else { 0.25 }, centred);
        assert_eq!((s.lt, s.rt), (64, 255));
    }
    #[test]
    fn a_digital_trigger_reads_as_fully_pressed() {
        let s = map_state(|b| b == Button::LeftTrigger2, flat, centred);
        assert_eq!((s.lt, s.rt), (255, 0));
        // Pressing the shoulder button is a different bit from the trigger.
        let s = map_state(|b| b == Button::LeftTrigger, flat, centred);
        assert_eq!((s.btn, s.lt), (0x0100, 0));
    }

    #[test]
    fn ids_follow_plug_in_order() {
        let mut t = Tracker::default();
        assert_eq!(t.connect(10, A), Some(0));
        assert_eq!(t.connect(11, B), Some(1));
        assert_eq!(t.connect(12, A), Some(2));
        assert_eq!(t.connect(13, C), Some(3));
        assert_eq!(t.connect(14, C), None);
        assert_eq!(t.connect(11, B), Some(1), "plugging in twice keeps the id");
        assert_eq!(t.live(), vec![0, 1, 2, 3]);
    }
    #[test]
    fn a_pad_that_comes_back_gets_its_old_id() {
        let mut t = Tracker::default();
        t.connect(1, A);
        t.connect(2, B);
        assert_eq!(t.disconnect(1), Some(0));
        assert_eq!(t.disconnect(1), None);
        // A different model takes a fresh id and leaves id 0 waiting for the first pad.
        assert_eq!(t.connect(3, C), Some(2));
        assert_eq!(t.connect(4, A), Some(0));
        assert_eq!(t.key_of(0), Some(4));
    }
    #[test]
    fn the_pad_gone_longest_gives_up_its_id_when_all_four_are_taken() {
        let mut t = Tracker::default();
        for (key, uuid) in [(1, A), (2, B), (3, C), (4, [4; 16])] {
            t.connect(key, uuid);
        }
        t.disconnect(3);
        t.disconnect(2);
        assert_eq!(t.connect(9, [9; 16]), Some(2), "id 2 was freed first");
        assert_eq!(t.connect(8, [8; 16]), Some(1));
        assert_eq!(t.connect(7, [7; 16]), None);
    }
    #[test]
    fn a_state_goes_out_on_change_and_once_a_second_while_idle() {
        let mut t = Tracker::default();
        t.connect(1, A);
        let t0 = Instant::now();
        let rest = PadState::default();
        let pressed = PadState { btn: 4096, ..rest };
        assert_eq!(t.observe(1, rest, t0), Some(Out::State(0, rest)));
        assert_eq!(t.observe(1, rest, t0 + Duration::from_millis(500)), None);
        assert_eq!(t.observe(1, rest, t0 + Duration::from_millis(999)), None);
        assert_eq!(t.observe(1, rest, t0 + IDLE_REPEAT), Some(Out::State(0, rest)));
        let t1 = t0 + IDLE_REPEAT;
        assert_eq!(t.observe(1, rest, t1 + Duration::from_millis(900)), None);
        assert_eq!(t.observe(1, pressed, t1 + Duration::from_millis(900)), Some(Out::State(0, pressed)));
        assert_eq!(t.observe(1, pressed, t1 + Duration::from_millis(1100)), None);
        assert_eq!(t.observe(1, pressed, t1 + Duration::from_millis(1900)), Some(Out::State(0, pressed)));
        assert_eq!(t.observe(99, pressed, t1), None, "unknown pad");
    }
    #[test]
    fn fast_changes_are_spaced_and_nothing_is_lost() {
        let mut t = Tracker::default();
        t.connect(1, A);
        let t0 = Instant::now();
        let a = PadState { lx: 100, ..Default::default() };
        let b = PadState { lx: 200, ..Default::default() };
        assert!(t.observe(1, a, t0).is_some());
        assert_eq!(t.observe(1, b, t0 + Duration::from_millis(2)), None);
        assert_eq!(t.observe(1, b, t0 + Duration::from_millis(9)), Some(Out::State(0, b)));
    }
    #[test]
    fn a_returning_pad_sends_its_state_again_at_once() {
        let mut t = Tracker::default();
        t.connect(1, A);
        let t0 = Instant::now();
        let s = PadState { rt: 255, ..Default::default() };
        t.observe(1, s, t0);
        t.disconnect(1);
        t.connect(2, A);
        assert_eq!(t.observe(2, s, t0 + Duration::from_millis(1)), Some(Out::State(0, s)));
    }
}
