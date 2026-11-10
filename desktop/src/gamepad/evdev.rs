//! Linux controllers: /dev/input/event* through plain evdev ioctls (libc only, no extra crates).
//! Devices are opened without grabbing them. Virtual devices are skipped, so the pads Secpar's
//! own host creates (and Steam Input's) are never sent back to a host.

use super::logic::{rumble16, State};
use super::map::*;
use super::Source;
use std::{
    collections::HashSet,
    fs, io,
    mem::size_of,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::{Duration, Instant},
};

const fn ioc(dir: u64, nr: u64, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((b'E' as u64) << 8) | nr
}
const W: u64 = 1;
const R: u64 = 2;
const EV_FF: u16 = 0x15;
const FF_RUMBLE: u16 = 0x50;

fn eviocgbit(ev: u16, len: usize) -> u64 {
    ioc(R, 0x20 + ev as u64, len)
}
fn eviocgabs(abs: u16) -> u64 {
    ioc(R, 0x40 + abs as u64, size_of::<libc::input_absinfo>())
}
fn eviocgkey(len: usize) -> u64 {
    ioc(R, 0x18, len)
}
const EVIOCGID: u64 = ioc(R, 0x02, 8);
const EVIOCSFF: u64 = ioc(W, 0x80, size_of::<libc::ff_effect>());
const EVIOCRMFF: u64 = ioc(W, 0x81, 4);

fn ioctl<T>(fd: &OwnedFd, request: u64, arg: *mut T) -> io::Result<()> {
    // SAFETY: `arg` points to a live buffer of the size encoded in `request`.
    if unsafe { libc::ioctl(fd.as_raw_fd(), request as _, arg) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn bits(fd: &OwnedFd, ev: u16, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    if ioctl(fd, eviocgbit(ev, len), buf.as_mut_ptr()).is_err() {
        buf.fill(0);
    }
    buf
}
fn has(bits: &[u8], code: u16) -> bool {
    bits.get(code as usize / 8).is_some_and(|b| b >> (code % 8) & 1 == 1)
}

fn abs_info(fd: &OwnedFd, code: u16) -> Option<AbsInfo> {
    // SAFETY: input_absinfo is plain data.
    let mut a: libc::input_absinfo = unsafe { std::mem::zeroed() };
    ioctl(fd, eviocgabs(code), &mut a).ok()?;
    Some(AbsInfo { value: a.value, min: a.minimum, max: a.maximum, flat: a.flat })
}

struct Dev {
    fd: OwnedFd,
    key: u64,
    path: String,
    mapper: Mapper,
    /// Uploaded rumble effect id, when the device can rumble and was opened for writing.
    ff: Option<i16>,
    can_rumble: bool,
    state: State,
}

impl Dev {
    fn open(path: &str, key: u64, allow_virtual: Option<&str>) -> Option<Dev> {
        let name = path.rsplit('/').next()?;
        // Virtual devices (uinput, including Secpar's own host pads) live under /devices/virtual.
        if allow_virtual.is_none() && fs::canonicalize(format!("/sys/class/input/{name}")).ok()?.to_string_lossy().contains("/devices/virtual/") {
            return None;
        }
        let c = std::ffi::CString::new(path).ok()?;
        let flags = libc::O_NONBLOCK | libc::O_CLOEXEC;
        // SAFETY: valid C string.
        let mut raw = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | flags) };
        let writable = raw >= 0;
        if raw < 0 {
            // SAFETY: valid C string.
            raw = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | flags) };
        }
        if raw < 0 {
            return None;
        }
        // SAFETY: `raw` is a fresh descriptor we own.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        if let Some(prefix) = allow_virtual {
            // tests: only the stand in devices with this name prefix
            let mut name = [0u8; 128];
            ioctl(&fd, ioc(R, 0x06, 128), name.as_mut_ptr()).ok()?;
            if !name.starts_with(prefix.as_bytes()) {
                return None;
            }
        }
        let keys = bits(&fd, EV_KEY, 96);
        let abs = bits(&fd, EV_ABS, 8);
        if !is_gamepad(|c| has(&keys, c), |c| has(&abs, c)) {
            return None;
        }
        let mut id = [0u16; 4];
        ioctl(&fd, EVIOCGID, id.as_mut_ptr()).ok()?;
        let mapper = Mapper::new(id[1], |c| abs_info(&fd, c).filter(|_| has(&abs, c)))?;
        let can_rumble = writable && has(&bits(&fd, EV_FF, 16), FF_RUMBLE);
        let mut dev = Dev { fd, key, path: path.into(), mapper, ff: None, can_rumble, state: State::default() };
        dev.resync();
        Some(dev)
    }

    fn resync(&mut self) {
        let mut keys = [0u8; 96];
        let _ = ioctl(&self.fd, eviocgkey(96), keys.as_mut_ptr());
        let pressed: Vec<u16> = (0..96u16 * 8).filter(|c| has(&keys, *c)).collect();
        let fd = &self.fd;
        self.mapper.resync(pressed, |c| abs_info(fd, c));
        self.state = self.mapper.state();
    }

    /// Reads everything pending. Err means the device is gone.
    fn pump(&mut self) -> io::Result<()> {
        let mut dirty = false;
        loop {
            // SAFETY: input_event is plain data; the kernel only returns whole events.
            let mut ev: libc::input_event = unsafe { std::mem::zeroed() };
            let n = unsafe { libc::read(self.fd.as_raw_fd(), (&mut ev as *mut libc::input_event).cast(), size_of::<libc::input_event>()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if n as usize != size_of::<libc::input_event>() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short read"));
            }
            match ev.type_ {
                EV_KEY => self.mapper.key(ev.code, ev.value),
                EV_ABS => self.mapper.axis(ev.code, ev.value),
                EV_SYN if ev.code == SYN_DROPPED => self.resync(),
                EV_SYN => dirty = true,
                _ => {}
            }
        }
        if dirty {
            self.state = self.mapper.state();
        }
        Ok(())
    }

    fn rumble(&mut self, lo: u8, hi: u8) {
        if !self.can_rumble {
            return;
        }
        let (strong, weak) = rumble16(lo, hi);
        // SAFETY: ff_effect is plain data.
        let mut effect: libc::ff_effect = unsafe { std::mem::zeroed() };
        effect.type_ = FF_RUMBLE;
        effect.id = self.ff.unwrap_or(-1);
        effect.replay.length = 0xffff;
        effect.u[0] = strong as u64 | (weak as u64) << 16;
        if ioctl(&self.fd, EVIOCSFF, &mut effect).is_err() {
            return;
        }
        self.ff = Some(effect.id);
        self.play(effect.id, strong != 0 || weak != 0);
    }

    fn play(&self, id: i16, on: bool) {
        // SAFETY: input_event is plain data; one whole event is written.
        let mut ev: libc::input_event = unsafe { std::mem::zeroed() };
        ev.type_ = EV_FF;
        ev.code = id as u16;
        ev.value = on as i32;
        unsafe { libc::write(self.fd.as_raw_fd(), (&ev as *const libc::input_event).cast(), size_of::<libc::input_event>()) };
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        if let Some(id) = self.ff.take() {
            self.play(id, false);
            let mut n = id as i32;
            let _ = ioctl(&self.fd, EVIOCRMFF, &mut n);
        }
    }
}

pub struct Evdev {
    devices: Vec<Dev>,
    /// Event nodes already looked at and rejected (keyboards, mice, virtual pads).
    rejected: HashSet<String>,
    scanned: Option<Instant>,
    next_key: u64,
    allow_virtual: Option<String>,
}

impl Evdev {
    pub fn new() -> Self {
        Evdev { devices: Vec::new(), rejected: HashSet::new(), scanned: None, next_key: 1, allow_virtual: None }
    }

    /// Takes only virtual (uinput) devices whose name starts with `prefix`. For tests that stand
    /// in for a physical pad.
    pub fn with_virtual(prefix: &str) -> Self {
        Evdev { allow_virtual: Some(prefix.into()), ..Evdev::new() }
    }

    fn scan(&mut self) {
        let Ok(dir) = fs::read_dir("/dev/input") else { return };
        let mut present = HashSet::new();
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("event") {
                continue;
            }
            let path = format!("/dev/input/{name}");
            present.insert(path.clone());
            if self.rejected.contains(&path) || self.devices.iter().any(|d| d.path == path) {
                continue;
            }
            match Dev::open(&path, self.next_key, self.allow_virtual.as_deref()) {
                Some(dev) => {
                    self.next_key += 1;
                    self.devices.push(dev);
                }
                None => {
                    self.rejected.insert(path);
                }
            }
        }
        // a node that vanished may come back as a different device
        self.rejected.retain(|p| present.contains(p));
    }
}

impl Source for Evdev {
    fn poll(&mut self) -> Vec<(u64, State)> {
        if self.scanned.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
            self.scanned = Some(Instant::now());
            self.scan();
        }
        self.devices.retain_mut(|d| d.pump().is_ok());
        self.devices.iter().map(|d| (d.key, d.state)).collect()
    }

    fn rumble(&mut self, key: u64, lo: u8, hi: u8) {
        if let Some(d) = self.devices.iter_mut().find(|d| d.key == key) {
            d.rumble(lo, hi);
        }
    }

    fn silence(&mut self) {
        for d in &mut self.devices {
            d.rumble(0, 0);
        }
    }
}
