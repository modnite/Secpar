//! Virtual Xbox 360 controllers through /dev/uinput (the Linux twin of the Windows ViGEm bus).
//! The phone sends XInput state; each pad id becomes one evdev device that looks like a wired
//! Xbox 360 pad (045e:028e), so SDL, Steam and Proton map it without configuration. Rumble comes
//! back through the kernel's force-feedback protocol on the same file descriptor.
use secpar_host_core::pad::{Bus, PadState, Rumble, Target};
use std::{
    collections::HashMap,
    io,
    mem::size_of,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const EV_FF: u16 = 0x15;
const EV_UINPUT: u16 = 0x0101;
const SYN_REPORT: u16 = 0;
const UI_FF_UPLOAD: u16 = 1;
const UI_FF_ERASE: u16 = 2;
const FF_RUMBLE: u16 = 0x50;
const BUS_USB: u16 = 3;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

/// XInput button mask (what the phone sends) to evdev key, in the order the Xbox 360 driver uses.
const BUTTONS: [(u16, u16); 11] = [
    (0x1000, 0x130), // A       BTN_SOUTH
    (0x2000, 0x131), // B       BTN_EAST
    (0x4000, 0x133), // X       BTN_NORTH
    (0x8000, 0x134), // Y       BTN_WEST
    (0x0100, 0x136), // LB      BTN_TL
    (0x0200, 0x137), // RB      BTN_TR
    (0x0020, 0x13a), // Back    BTN_SELECT
    (0x0010, 0x13b), // Start   BTN_START
    (0x0400, 0x13c), // Guide   BTN_MODE
    (0x0040, 0x13d), // L3      BTN_THUMBL
    (0x0080, 0x13e), // R3      BTN_THUMBR
];
const DPAD_UP: u16 = 0x0001;
const DPAD_DOWN: u16 = 0x0002;
const DPAD_LEFT: u16 = 0x0004;
const DPAD_RIGHT: u16 = 0x0008;

/// (axis, minimum, maximum, flat)
const AXES: [(u16, i32, i32, i32); 8] = [
    (ABS_X, -32768, 32767, 128),
    (ABS_Y, -32768, 32767, 128),
    (ABS_Z, 0, 255, 0),
    (ABS_RX, -32768, 32767, 128),
    (ABS_RY, -32768, 32767, 128),
    (ABS_RZ, 0, 255, 0),
    (ABS_HAT0X, -1, 1, 0),
    (ABS_HAT0Y, -1, 1, 0),
];

/// One evdev event: (type, code, value).
pub type Ev = (u16, u16, i32);

fn hat(state: &PadState, minus: u16, plus: u16) -> i32 {
    (state.btn & plus != 0) as i32 - (state.btn & minus != 0) as i32
}

/// XInput has up as positive Y; evdev has up as negative. Centre stays exactly 0.
fn flip(v: i16) -> i32 {
    (-(v as i32)).clamp(-32768, 32767)
}

/// The events that take a pad from `prev` to `next`: only what changed, ending in SYN_REPORT.
pub fn changes(prev: &PadState, next: &PadState) -> Vec<Ev> {
    let mut v = Vec::new();
    let mut axis = |code: u16, old: i32, new: i32| {
        if old != new {
            v.push((EV_ABS, code, new));
        }
    };
    axis(ABS_X, prev.lx as i32, next.lx as i32);
    axis(ABS_Y, flip(prev.ly), flip(next.ly));
    axis(ABS_RX, prev.rx as i32, next.rx as i32);
    axis(ABS_RY, flip(prev.ry), flip(next.ry));
    axis(ABS_Z, prev.lt as i32, next.lt as i32);
    axis(ABS_RZ, prev.rt as i32, next.rt as i32);
    axis(ABS_HAT0X, hat(prev, DPAD_LEFT, DPAD_RIGHT), hat(next, DPAD_LEFT, DPAD_RIGHT));
    axis(ABS_HAT0Y, hat(prev, DPAD_UP, DPAD_DOWN), hat(next, DPAD_UP, DPAD_DOWN));
    for (mask, key) in BUTTONS {
        if (prev.btn ^ next.btn) & mask != 0 {
            v.push((EV_KEY, key, (next.btn & mask != 0) as i32));
        }
    }
    if !v.is_empty() {
        v.push((EV_SYN, SYN_REPORT, 0));
    }
    v
}

/// Where events go. The real sink is a uinput file descriptor; tests use a fake.
pub trait Sink: Send {
    fn write_events(&mut self, events: &[Ev]) -> io::Result<()>;
}

/// One virtual controller. `update` is false only when the device is gone, so the caller re-plugs.
pub struct PadDevice<S: Sink> {
    sink: S,
    last: PadState,
}

impl<S: Sink> PadDevice<S> {
    pub fn new(sink: S) -> Self {
        PadDevice { sink, last: PadState { btn: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 } }
    }
}

impl<S: Sink> Target for PadDevice<S> {
    fn update(&mut self, state: &PadState) -> bool {
        let events = changes(&self.last, state);
        if events.is_empty() {
            return true;
        }
        match self.sink.write_events(&events) {
            Ok(()) => {
                self.last = *state;
                true
            }
            // A full event queue drops this report. The next one carries the whole state again.
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => true,
            Err(_) => false,
        }
    }
}

/// Large motor = the kernel's strong magnitude, small motor = weak; both 16-bit, the phone wants 8.
pub fn motor_levels(strong: u16, weak: u16) -> (u8, u8) {
    ((strong >> 8) as u8, (weak >> 8) as u8)
}

/// The force-feedback bookkeeping of one device: uploaded effects and the one that is playing.
#[derive(Default)]
pub struct Ff {
    effects: HashMap<i16, (u16, u16)>,
    playing: Option<i16>,
}

impl Ff {
    /// A game stores (or changes) a rumble effect. If it is playing, the new levels apply now.
    pub fn upload(&mut self, id: i16, strong: u16, weak: u16) -> Option<(u8, u8)> {
        self.effects.insert(id, (strong, weak));
        (self.playing == Some(id)).then(|| motor_levels(strong, weak))
    }
    pub fn erase(&mut self, id: i16) -> Option<(u8, u8)> {
        self.effects.remove(&id);
        (self.playing == Some(id)).then(|| {
            self.playing = None;
            (0, 0)
        })
    }
    /// `EV_FF` with the effect id as code: a positive value starts it, zero stops it.
    pub fn play(&mut self, id: i16, value: i32) -> Option<(u8, u8)> {
        if value > 0 {
            let (strong, weak) = *self.effects.get(&id)?;
            self.playing = Some(id);
            Some(motor_levels(strong, weak))
        } else if self.playing == Some(id) {
            self.playing = None;
            Some((0, 0))
        } else {
            None
        }
    }
    pub fn is_playing(&self) -> bool {
        self.playing.is_some()
    }
}

const fn ioc(dir: u64, nr: u64, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((b'U' as u64) << 8) | nr
}
const W: u64 = 1;
const R: u64 = 2;
const UI_DEV_CREATE: u64 = ioc(0, 1, 0);
const UI_DEV_DESTROY: u64 = ioc(0, 2, 0);
const UI_DEV_SETUP: u64 = ioc(W, 3, size_of::<libc::uinput_setup>());
const UI_ABS_SETUP: u64 = ioc(W, 4, size_of::<libc::uinput_abs_setup>());
const UI_SET_EVBIT: u64 = ioc(W, 100, 4);
const UI_SET_KEYBIT: u64 = ioc(W, 101, 4);
const UI_SET_ABSBIT: u64 = ioc(W, 103, 4);
const UI_SET_FFBIT: u64 = ioc(W, 107, 4);
const UI_BEGIN_FF_UPLOAD: u64 = ioc(R | W, 200, size_of::<libc::uinput_ff_upload>());
const UI_END_FF_UPLOAD: u64 = ioc(W, 201, size_of::<libc::uinput_ff_upload>());
const UI_BEGIN_FF_ERASE: u64 = ioc(R | W, 202, size_of::<libc::uinput_ff_erase>());
const UI_END_FF_ERASE: u64 = ioc(W, 203, size_of::<libc::uinput_ff_erase>());

const NODE: &str = "/dev/uinput";

fn ioctl<T>(fd: &impl AsRawFd, request: u64, arg: *const T) -> io::Result<()> {
    // SAFETY: `arg` points to a live value of the type the request expects, or is a plain integer
    // passed by value for the SET_*BIT requests (see `ioctl_int`).
    let r = unsafe { libc::ioctl(fd.as_raw_fd(), request as _, arg) };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn ioctl_int(fd: &impl AsRawFd, request: u64, value: u16) -> io::Result<()> {
    // SAFETY: SET_*BIT requests take the integer itself as the argument.
    let r = unsafe { libc::ioctl(fd.as_raw_fd(), request as _, value as libc::c_ulong) };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Whether virtual controllers can be created here, and if not, why (shown in Hosting).
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub ok: bool,
    pub text: String,
}

/// The status for the errno that `access(/dev/uinput, W_OK)` gave (None = it worked).
pub fn status_for(errno: Option<i32>) -> Status {
    let (ok, text) = match errno {
        None => (true, "Gamepad ready"),
        Some(libc::ENOENT) => (false, "Gamepad unavailable: /dev/uinput is missing (load the uinput kernel module)"),
        Some(libc::EACCES) | Some(libc::EPERM) => (false, "Gamepad unavailable: no write access to /dev/uinput (see docs/linux.md)"),
        Some(_) => (false, "Gamepad unavailable: /dev/uinput cannot be opened"),
    };
    Status { ok, text: text.into() }
}

pub fn status() -> Status {
    let node = std::ffi::CString::new(NODE).unwrap();
    // SAFETY: valid C string.
    let errno = (unsafe { libc::access(node.as_ptr(), libc::W_OK) } != 0).then(|| io::Error::last_os_error().raw_os_error().unwrap_or(0));
    status_for(errno)
}

pub fn available() -> bool {
    status().ok
}

fn event(kind: u16, code: u16, value: i32) -> libc::input_event {
    // SAFETY: input_event is plain data; zero is a valid value (the kernel fills in the time).
    let mut e: libc::input_event = unsafe { std::mem::zeroed() };
    e.type_ = kind;
    e.code = code;
    e.value = value;
    e
}

/// An open uinput device. Dropping it removes the controller and stops the rumble thread.
struct Device {
    fd: Arc<OwnedFd>,
    stop: Arc<AtomicBool>,
}

impl Device {
    fn create(id: u8, rumble: Rumble) -> io::Result<Self> {
        let node = std::ffi::CString::new(NODE).unwrap();
        // SAFETY: valid C string; the returned descriptor is owned by the OwnedFd below.
        let raw = unsafe { libc::open(node.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a freshly opened descriptor nobody else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        for kind in [EV_KEY, EV_ABS, EV_FF, EV_SYN] {
            ioctl_int(&fd, UI_SET_EVBIT, kind)?;
        }
        for (_, key) in BUTTONS {
            ioctl_int(&fd, UI_SET_KEYBIT, key)?;
        }
        for (code, min, max, flat) in AXES {
            ioctl_int(&fd, UI_SET_ABSBIT, code)?;
            let setup = libc::uinput_abs_setup {
                code,
                absinfo: libc::input_absinfo { value: 0, minimum: min, maximum: max, fuzz: 0, flat, resolution: 0 },
            };
            ioctl(&fd, UI_ABS_SETUP, &setup)?;
        }
        ioctl_int(&fd, UI_SET_FFBIT, FF_RUMBLE)?;
        // SAFETY: uinput_setup is plain data.
        let mut setup: libc::uinput_setup = unsafe { std::mem::zeroed() };
        setup.id = libc::input_id { bustype: BUS_USB, vendor: 0x045e, product: 0x028e, version: 0x0114 };
        for (slot, byte) in setup.name.iter_mut().zip(b"Microsoft X-Box 360 pad") {
            *slot = *byte as libc::c_char;
        }
        setup.ff_effects_max = 16;
        ioctl(&fd, UI_DEV_SETUP, &setup)?;
        ioctl(&fd, UI_DEV_CREATE, std::ptr::null::<u8>())?;
        let fd = Arc::new(fd);
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_fd, thread_stop) = (fd.clone(), stop.clone());
        std::thread::Builder::new()
            .name(format!("secpar-pad-{id}"))
            .spawn(move || rumble_loop(thread_fd, thread_stop, rumble))
            .map_err(io::Error::other)?;
        Ok(Device { fd, stop })
    }
}

impl Sink for Device {
    fn write_events(&mut self, events: &[Ev]) -> io::Result<()> {
        let raw: Vec<libc::input_event> = events.iter().map(|&(kind, code, value)| event(kind, code, value)).collect();
        let bytes = raw.len() * size_of::<libc::input_event>();
        // SAFETY: `raw` is a contiguous array of `bytes` readable bytes.
        let n = unsafe { libc::write(self.fd.as_raw_fd(), raw.as_ptr().cast(), bytes) };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = ioctl(&*self.fd, UI_DEV_DESTROY, std::ptr::null::<u8>());
    }
}

/// Answers the kernel's force-feedback requests until the device is dropped.
fn rumble_loop(fd: Arc<OwnedFd>, stop: Arc<AtomicBool>, rumble: Rumble) {
    let mut ff = Ff::default();
    let size = size_of::<libc::input_event>();
    while !stop.load(Ordering::Acquire) {
        let mut poll = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd; a 100 ms timeout lets the loop notice `stop`.
        if unsafe { libc::poll(&mut poll, 1, 100) } <= 0 {
            continue;
        }
        if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            break;
        }
        // SAFETY: input_event is plain data.
        let mut ev: libc::input_event = unsafe { std::mem::zeroed() };
        // SAFETY: reads at most one event into `ev`.
        let n = unsafe { libc::read(fd.as_raw_fd(), (&mut ev as *mut libc::input_event).cast(), size) };
        if n != size as isize {
            continue;
        }
        let levels = match (ev.type_, ev.code) {
            (EV_UINPUT, UI_FF_UPLOAD) => {
                // SAFETY: plain data.
                let mut up: libc::uinput_ff_upload = unsafe { std::mem::zeroed() };
                up.request_id = ev.value as u32;
                if ioctl(&*fd, UI_BEGIN_FF_UPLOAD, &up).is_err() {
                    continue;
                }
                // The union starts with the rumble effect's two magnitudes.
                let (strong, weak) = ((up.effect.u[0] & 0xffff) as u16, ((up.effect.u[0] >> 16) & 0xffff) as u16);
                let levels = (up.effect.type_ == FF_RUMBLE).then(|| ff.upload(up.effect.id, strong, weak)).flatten();
                up.retval = 0;
                let _ = ioctl(&*fd, UI_END_FF_UPLOAD, &up);
                levels
            }
            (EV_UINPUT, UI_FF_ERASE) => {
                // SAFETY: plain data.
                let mut erase: libc::uinput_ff_erase = unsafe { std::mem::zeroed() };
                erase.request_id = ev.value as u32;
                if ioctl(&*fd, UI_BEGIN_FF_ERASE, &erase).is_err() {
                    continue;
                }
                let levels = ff.erase(erase.effect_id as i16);
                erase.retval = 0;
                let _ = ioctl(&*fd, UI_END_FF_ERASE, &erase);
                levels
            }
            (EV_FF, code) if code < 0x60 => ff.play(code as i16, ev.value),
            _ => None,
        };
        if let Some((lo, hi)) = levels {
            rumble(lo, hi);
        }
    }
    if ff.is_playing() {
        rumble(0, 0);
    }
}

/// The real bus. Stateless: each `plug` opens its own uinput device.
pub struct UinputBus;

impl Bus for UinputBus {
    fn plug(&self, id: u8, rumble: Rumble) -> Option<Box<dyn Target>> {
        let device = Device::create(id, rumble).ok()?;
        Some(Box::new(PadDevice::new(device)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Mutex, time::Duration};

    fn st(btn: u16, lt: u8, rt: u8, lx: i16, ly: i16, rx: i16, ry: i16) -> PadState {
        PadState { btn, lt, rt, lx, ly, rx, ry }
    }
    const ZERO: PadState = PadState { btn: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 };

    #[test]
    fn ioctl_numbers_match_the_kernel_headers() {
        // Values of <linux/uinput.h> on 64-bit Linux, taken from a C program run on the OMV box.
        if cfg!(target_pointer_width = "64") {
            assert_eq!(UI_DEV_CREATE, 0x5501);
            assert_eq!(UI_DEV_DESTROY, 0x5502);
            assert_eq!(UI_DEV_SETUP, 0x405c5503);
            assert_eq!(UI_ABS_SETUP, 0x401c5504);
            assert_eq!(UI_SET_EVBIT, 0x40045564);
            assert_eq!(UI_SET_KEYBIT, 0x40045565);
            assert_eq!(UI_SET_ABSBIT, 0x40045567);
            assert_eq!(UI_SET_FFBIT, 0x4004556b);
            assert_eq!(UI_BEGIN_FF_UPLOAD, 0xc06855c8);
            assert_eq!(UI_END_FF_UPLOAD, 0x406855c9);
            assert_eq!(UI_BEGIN_FF_ERASE, 0xc00c55ca);
            assert_eq!(UI_END_FF_ERASE, 0x400c55cb);
        }
    }

    #[test]
    fn idle_pad_sends_nothing() {
        assert!(changes(&ZERO, &ZERO).is_empty());
    }

    #[test]
    fn buttons_map_like_the_kernel_xbox_driver() {
        let pressed = changes(&ZERO, &st(0x1000 | 0x0100 | 0x0010, 0, 0, 0, 0, 0, 0));
        assert_eq!(pressed, vec![(EV_KEY, 0x130, 1), (EV_KEY, 0x136, 1), (EV_KEY, 0x13b, 1), (EV_SYN, SYN_REPORT, 0)]);
        let released = changes(&st(0x1000 | 0x0100, 0, 0, 0, 0, 0, 0), &st(0x0100, 0, 0, 0, 0, 0, 0));
        assert_eq!(released, vec![(EV_KEY, 0x130, 0), (EV_SYN, SYN_REPORT, 0)]);
        // every XInput face, shoulder, stick and menu bit has exactly one distinct evdev key
        let masks: u16 = BUTTONS.iter().fold(0, |m, (mask, _)| m | mask);
        assert_eq!(masks, 0xf7f0);
        let keys: std::collections::HashSet<u16> = BUTTONS.iter().map(|(_, k)| *k).collect();
        assert_eq!(keys.len(), BUTTONS.len());
        let all = changes(&ZERO, &st(masks, 0, 0, 0, 0, 0, 0));
        assert_eq!(all.len(), BUTTONS.len() + 1);
    }

    #[test]
    fn dpad_is_a_hat_and_opposites_cancel() {
        assert_eq!(changes(&ZERO, &st(DPAD_UP, 0, 0, 0, 0, 0, 0)), vec![(EV_ABS, ABS_HAT0Y, -1), (EV_SYN, 0, 0)]);
        assert_eq!(changes(&ZERO, &st(DPAD_DOWN | DPAD_RIGHT, 0, 0, 0, 0, 0, 0)), vec![(EV_ABS, ABS_HAT0X, 1), (EV_ABS, ABS_HAT0Y, 1), (EV_SYN, 0, 0)]);
        assert_eq!(changes(&ZERO, &st(DPAD_LEFT | DPAD_RIGHT | DPAD_UP | DPAD_DOWN, 0, 0, 0, 0, 0, 0)), vec![]);
        assert_eq!(changes(&st(DPAD_LEFT, 0, 0, 0, 0, 0, 0), &ZERO), vec![(EV_ABS, ABS_HAT0X, 0), (EV_SYN, 0, 0)]);
    }

    #[test]
    fn sticks_and_triggers_keep_their_values_and_y_is_inverted() {
        let v = changes(&ZERO, &st(0, 200, 255, 1200, 32767, -32768, -1));
        assert!(v.contains(&(EV_ABS, ABS_X, 1200)));
        assert!(v.contains(&(EV_ABS, ABS_Y, -32767)), "XInput up is evdev up (negative): {v:?}");
        assert!(v.contains(&(EV_ABS, ABS_RX, -32768)));
        assert!(v.contains(&(EV_ABS, ABS_RY, 1)));
        assert!(v.contains(&(EV_ABS, ABS_Z, 200)));
        assert!(v.contains(&(EV_ABS, ABS_RZ, 255)));
        assert_eq!(v.last(), Some(&(EV_SYN, SYN_REPORT, 0)));
        // full deflection both ways stays inside the declared axis ranges
        for state in [st(0, 255, 255, i16::MIN, i16::MIN, i16::MAX, i16::MAX), st(0, 0, 0, i16::MAX, i16::MAX, i16::MIN, i16::MIN)] {
            for &(kind, code, value) in &changes(&ZERO, &state) {
                if kind == EV_ABS {
                    let (_, min, max, _) = AXES.iter().find(|a| a.0 == code).unwrap();
                    assert!((*min..=*max).contains(&value), "{code}: {value}");
                }
            }
        }
    }

    #[test]
    fn centred_sticks_are_the_devices_resting_value() {
        // A new device rests at 0 on every axis, so a centred first report must not move anything.
        assert_eq!(flip(0), 0);
        assert!(changes(&ZERO, &st(0, 0, 0, 0, 0, 0, 0)).is_empty());
        assert_eq!(changes(&st(0, 0, 0, 0, 5, 0, 0), &ZERO), vec![(EV_ABS, ABS_Y, 0), (EV_SYN, 0, 0)]);
    }

    #[derive(Default, Clone)]
    struct Fake(Arc<Mutex<Vec<Vec<Ev>>>>, Arc<Mutex<Option<io::ErrorKind>>>);
    impl Sink for Fake {
        fn write_events(&mut self, events: &[Ev]) -> io::Result<()> {
            if let Some(kind) = *self.1.lock().unwrap() {
                return Err(kind.into());
            }
            self.0.lock().unwrap().push(events.to_vec());
            Ok(())
        }
    }

    #[test]
    fn device_writes_only_changes_and_reports_a_gone_device() {
        let fake = Fake::default();
        let mut pad = PadDevice::new(fake.clone());
        assert!(pad.update(&st(0x1000, 0, 0, 0, 0, 0, 0)));
        assert!(pad.update(&st(0x1000, 0, 0, 0, 0, 0, 0)), "no change, no write");
        assert!(pad.update(&st(0x1000, 0, 90, 0, 0, 0, 0)));
        let writes = fake.0.lock().unwrap().clone();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[1], vec![(EV_ABS, ABS_RZ, 90), (EV_SYN, 0, 0)]);
        *fake.1.lock().unwrap() = Some(io::ErrorKind::WouldBlock);
        assert!(pad.update(&st(0, 0, 0, 0, 0, 0, 0)), "a full queue only drops this report");
        *fake.1.lock().unwrap() = None;
        assert!(pad.update(&st(0, 0, 0, 0, 0, 0, 0)));
        assert_eq!(fake.0.lock().unwrap().last().unwrap(), &vec![(EV_ABS, ABS_RZ, 0), (EV_KEY, 0x130, 0), (EV_SYN, 0, 0)], "the dropped report is resent as part of the next change");
        *fake.1.lock().unwrap() = Some(io::ErrorKind::NotFound);
        assert!(!pad.update(&st(0x2000, 0, 0, 0, 0, 0, 0)), "ENODEV and friends mean the device is gone");
    }

    #[test]
    fn rumble_levels_follow_the_effect_lifecycle() {
        let mut ff = Ff::default();
        assert_eq!(ff.play(3, 1), None, "unknown effect");
        assert_eq!(ff.upload(3, 0xffff, 0x8000), None, "stored, not playing yet");
        assert_eq!(ff.play(3, 1), Some((255, 128)));
        assert!(ff.is_playing());
        assert_eq!(ff.upload(3, 0x4000, 0), Some((64, 0)), "changing a playing effect applies at once");
        assert_eq!(ff.upload(4, 0xffff, 0xffff), None, "another effect does not interrupt it");
        assert_eq!(ff.play(4, 0), None, "stopping an effect that is not playing does nothing");
        assert_eq!(ff.play(3, 0), Some((0, 0)));
        assert!(!ff.is_playing());
        assert_eq!(ff.play(4, 2), Some((255, 255)));
        assert_eq!(ff.erase(4), Some((0, 0)), "erasing the playing effect stops the motors");
        assert_eq!(ff.erase(4), None);
        assert_eq!(motor_levels(0x00ff, 0x0100), (0, 1));
    }

    #[test]
    fn status_text_explains_each_failure() {
        assert_eq!(status_for(None), Status { ok: true, text: "Gamepad ready".into() });
        assert!(!status_for(Some(libc::ENOENT)).ok && status_for(Some(libc::ENOENT)).text.contains("missing"));
        assert!(status_for(Some(libc::EACCES)).text.contains("docs/linux.md"));
        assert!(status_for(Some(libc::EIO)).text.contains("cannot be opened"));
    }

    /// The real kernel path, by hand as root: creates a pad, checks that an evdev reader sees the
    /// events, then uploads a rumble effect through that device and expects the callback.
    #[test]
    #[ignore = "needs write access to /dev/uinput (run as root) and /dev/input"]
    fn real_device_events_and_rumble() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel::<(u8, u8)>();
        let tx = Mutex::new(tx);
        let bus = UinputBus;
        let mut pad = bus.plug(0, Box::new(move |lo, hi| { let _ = tx.lock().unwrap().send((lo, hi)); })).expect("plug needs /dev/uinput");
        std::thread::sleep(Duration::from_millis(500));
        // find our event node by name
        let node = (0..64)
            .map(|n| format!("/dev/input/event{n}"))
            .find(|p| {
                let n: Option<String> = std::fs::read_to_string(format!("/sys/class/input/{}/device/name", p.rsplit('/').next().unwrap())).ok();
                n.is_some_and(|n| n.trim() == "Microsoft X-Box 360 pad")
            })
            .expect("virtual pad node");
        let cpath = std::ffi::CString::new(node).unwrap();
        // SAFETY: valid C string.
        let reader = unsafe { libc::open(cpath.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
        assert!(reader >= 0, "open event node");
        // SAFETY: freshly opened descriptor.
        let reader = unsafe { OwnedFd::from_raw_fd(reader) };
        assert!(pad.update(&st(0x1000, 0, 255, 100, 0, 0, 0)));
        std::thread::sleep(Duration::from_millis(200));
        let mut got = vec![];
        loop {
            let mut ev: libc::input_event = unsafe { std::mem::zeroed() };
            // SAFETY: reads one event.
            let n = unsafe { libc::read(reader.as_raw_fd(), (&mut ev as *mut libc::input_event).cast(), size_of::<libc::input_event>()) };
            if n != size_of::<libc::input_event>() as isize { break; }
            got.push((ev.type_, ev.code, ev.value));
        }
        assert!(got.contains(&(EV_KEY, 0x130, 1)) && got.contains(&(EV_ABS, ABS_RZ, 255)) && got.contains(&(EV_ABS, ABS_X, 100)), "{got:?}");
        // EVIOCSFF = _IOW('E', 0x80, ff_effect): upload a rumble effect, then play it
        let mut effect: libc::ff_effect = unsafe { std::mem::zeroed() };
        effect.type_ = FF_RUMBLE;
        effect.id = -1;
        effect.u[0] = 0xc000 | (0x4000u64 << 16);
        let eviocsff = (1u64 << 30) | ((size_of::<libc::ff_effect>() as u64) << 16) | ((b'E' as u64) << 8) | 0x80;
        ioctl(&reader, eviocsff, &effect).expect("EVIOCSFF");
        let play = event(EV_FF, effect.id as u16, 1);
        // SAFETY: writes one event.
        assert_eq!(unsafe { libc::write(reader.as_raw_fd(), (&play as *const libc::input_event).cast(), size_of::<libc::input_event>()) }, size_of::<libc::input_event>() as isize);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).expect("rumble"), (0xc0, 0x40));
        let stop = event(EV_FF, effect.id as u16, 0);
        // SAFETY: writes one event.
        unsafe { libc::write(reader.as_raw_fd(), (&stop as *const libc::input_event).cast(), size_of::<libc::input_event>()) };
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).expect("rumble stop"), (0, 0));
        drop(pad);
        std::thread::sleep(Duration::from_millis(300));
    }
}
