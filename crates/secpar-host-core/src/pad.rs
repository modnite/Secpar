//! Virtual Xbox 360 style controllers, one per phone pad id. The platform supplies the `Bus`
//! (ViGEmBus on Windows, uinput on Linux); everything else is shared.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Controllers kept briefly after a phone disconnects, per device (with a generation stamp).
#[derive(Clone, Default)]
pub struct Lingering(Arc<Mutex<HashMap<String, Parked>>>);

/// A disconnected phone's controllers and the generation that parked them.
type Parked = (u64, Vec<Box<dyn Target>>);

/// XInput state from a `pad` message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadState {
    pub btn: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

/// Rumble from a game: (lo = large motor, hi = small motor).
pub type Rumble = Box<dyn Fn(u8, u8) + Send>;

/// The controller bus; `plug` creates a controller that is unplugged when dropped.
pub trait Bus: Send + Sync {
    fn plug(&self, id: u8, rumble: Rumble) -> Option<Box<dyn Target>>;
}

pub trait Target: Send {
    fn update(&mut self, s: &PadState) -> bool;
}

/// One session's controllers, ids 0..=3.
pub struct Pads {
    bus: Arc<dyn Bus>,
    on_rumble: Arc<dyn Fn(u8, u8, u8) + Send + Sync>,
    targets: [Option<Box<dyn Target>>; 4],
    device: Option<(String, Lingering)>,
}

impl Pads {
    pub fn new(bus: Arc<dyn Bus>, on_rumble: Box<dyn Fn(u8, u8, u8) + Send + Sync>) -> Self {
        Pads { bus, on_rumble: on_rumble.into(), targets: Default::default(), device: None }
    }

    /// Ties these controllers to a phone: its lingering controllers from a previous connection
    /// are released now, so a game never sees two controllers for one phone.
    pub fn for_device(mut self, device: &str, lingering: Lingering) -> Self {
        drop(lingering.0.lock().unwrap().remove(device));
        self.device = Some((device.to_string(), lingering));
        self
    }

    /// Creates the controller on first use, then updates it.
    pub fn pad(&mut self, id: u8, s: &PadState) -> Result<(), &'static str> {
        let slot = self.targets.get_mut(id as usize).ok_or("bad_request")?;
        if slot.is_none() {
            let cb = self.on_rumble.clone();
            *slot = Some(self.bus.plug(id, Box::new(move |lo, hi| cb(id, lo, hi))).ok_or("internal")?);
        }
        if slot.as_mut().is_some_and(|t| t.update(s)) {
            Ok(())
        } else {
            *slot = None; // re-plug on the next message
            Err("internal")
        }
    }

    pub fn off(&mut self, id: u8) {
        if let Some(slot) = self.targets.get_mut(id as usize) {
            *slot = None;
        }
    }

    /// After a disconnect, keeps the controllers for `d` so a game doesn't drop them on a blip.
    pub fn linger(&mut self, d: Duration) {
        let targets: Vec<Box<dyn Target>> = std::mem::take(&mut self.targets).into_iter().flatten().collect();
        if targets.is_empty() {
            return;
        }
        match &self.device {
            Some((dev, lingering)) => {
                static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                let gen = GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                lingering.0.lock().unwrap().insert(dev.clone(), (gen, targets));
                let (dev, lingering) = (dev.clone(), lingering.clone());
                // Without a runtime (shutdown) there is nobody to wait for: the controllers just go.
                let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                    lingering.0.lock().unwrap().remove(&dev);
                    return;
                };
                runtime.spawn(async move {
                    tokio::time::sleep(d).await;
                    let mut m = lingering.0.lock().unwrap();
                    if m.get(&dev).is_some_and(|(g, _)| *g == gen) {
                        m.remove(&dev);
                    }
                });
            }
            None => {
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn(async move {
                        tokio::time::sleep(d).await;
                        drop(targets);
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);

    struct FakeBus(Arc<Log>);
    struct FakeTarget(u8, Arc<Log>);

    impl Bus for FakeBus {
        fn plug(&self, id: u8, _rumble: Rumble) -> Option<Box<dyn Target>> {
            self.0 .0.lock().unwrap().push(format!("plug {id}"));
            Some(Box::new(FakeTarget(id, self.0.clone())))
        }
    }
    impl Target for FakeTarget {
        fn update(&mut self, s: &PadState) -> bool {
            self.1 .0.lock().unwrap().push(format!("update {} {}", self.0, s.btn));
            true
        }
    }
    impl Drop for FakeTarget {
        fn drop(&mut self) {
            self.1 .0.lock().unwrap().push(format!("unplug {}", self.0));
        }
    }

    fn st(btn: u16) -> PadState {
        PadState { btn, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 }
    }

    fn rig() -> (Pads, Arc<Log>) {
        let log = Arc::new(Log::default());
        (Pads::new(Arc::new(FakeBus(log.clone())), Box::new(|_, _, _| {})), log)
    }

    #[test]
    fn ids_0_to_3_only() {
        let (mut p, _) = rig();
        for id in 0..4 {
            assert_eq!(p.pad(id, &st(1)), Ok(()));
        }
        assert_eq!(p.pad(4, &st(1)), Err("bad_request"));
        assert_eq!(p.pad(255, &st(1)), Err("bad_request"));
    }

    #[test]
    fn creates_once_updates_and_off_removes() {
        let (mut p, log) = rig();
        p.pad(1, &st(4096)).unwrap();
        p.pad(1, &st(0)).unwrap();
        p.off(1);
        p.off(1);
        p.pad(1, &st(2)).unwrap();
        assert_eq!(*log.0.lock().unwrap(), ["plug 1", "update 1 4096", "update 1 0", "unplug 1", "plug 1", "update 1 2"]);
    }

    #[tokio::test(start_paused = true)]
    async fn removed_5s_after_disconnect() {
        let (mut p, log) = rig();
        p.pad(0, &st(1)).unwrap();
        p.pad(2, &st(1)).unwrap();
        p.linger(Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(4900)).await;
        assert!(!log.0.lock().unwrap().iter().any(|l| l.starts_with("unplug")));
        tokio::time::sleep(Duration::from_millis(200)).await;
        let l = log.0.lock().unwrap();
        assert!(l.contains(&"unplug 0".to_string()) && l.contains(&"unplug 2".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_releases_lingering_before_new_plug() {
        let log = Arc::new(Log::default());
        let lingering = Lingering::default();
        let bus: Arc<dyn Bus> = Arc::new(FakeBus(log.clone()));
        let mut p1 = Pads::new(bus.clone(), Box::new(|_, _, _| {})).for_device("phone", lingering.clone());
        p1.pad(0, &st(1)).unwrap();
        p1.linger(Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(100)).await;
        // the same phone reconnects: its old controller goes away at once, so a game never sees two
        let mut p2 = Pads::new(bus, Box::new(|_, _, _| {})).for_device("phone", lingering.clone());
        assert!(log.0.lock().unwrap().contains(&"unplug 0".to_string()));
        p2.pad(0, &st(1)).unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(log.0.lock().unwrap().iter().filter(|l| *l == "unplug 0").count(), 1);
    }

    #[test]
    fn three_devices_have_independent_controller_zero_and_disconnect_is_local() {
        struct MultiBus(Arc<Mutex<Vec<Option<PadState>>>>);
        struct MultiTarget { index: usize, states: Arc<Mutex<Vec<Option<PadState>>>> }
        impl Bus for MultiBus {
            fn plug(&self, _id: u8, _rumble: Rumble) -> Option<Box<dyn Target>> {
                let index = { let mut states=self.0.lock().unwrap(); let i=states.len();states.push(Some(st(0)));i };
                Some(Box::new(MultiTarget {index,states:self.0.clone()}))
            }
        }
        impl Target for MultiTarget {
            fn update(&mut self, state: &PadState) -> bool {self.states.lock().unwrap()[self.index]=Some(*state);true}
        }
        impl Drop for MultiTarget {fn drop(&mut self){self.states.lock().unwrap()[self.index]=None;}}
        let states=Arc::new(Mutex::new(Vec::new()));
        let bus:Arc<dyn Bus>=Arc::new(MultiBus(states.clone()));
        let lingering=Lingering::default();
        let mut devices:Vec<_>=(0..3).map(|n|Pads::new(bus.clone(),Box::new(|_,_,_|{}))
            .for_device(&format!("device-{n}"),lingering.clone())).collect();
        for (n,device) in devices.iter_mut().enumerate(){device.pad(0,&st(1<<n)).unwrap();}
        assert_eq!(*states.lock().unwrap(),vec![Some(st(1)),Some(st(2)),Some(st(4))]);
        devices[0].off(0);
        assert_eq!(*states.lock().unwrap(),vec![None,Some(st(2)),Some(st(4))]);
    }
}
