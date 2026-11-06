//! Three remote streaming devices, with reservations held through worker cleanup.
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
pub const MAX_REMOTE_DEVICES: usize = 3;
pub const LIMIT_MESSAGE: &str = "Connection limit reached: 3 devices are already streaming. Disconnect a device and try again.";
#[derive(Default)]
pub struct StreamSlots { active: AtomicUsize }
impl StreamSlots {
    pub fn count(&self) -> usize { self.active.load(Ordering::Acquire) }
    pub fn try_acquire(self: &Arc<Self>) -> Option<StreamPermit> {
        let mut count = self.active.load(Ordering::Acquire);
        while count < MAX_REMOTE_DEVICES {
            match self.active.compare_exchange_weak(count, count + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(StreamPermit(self.clone())),
                Err(current) => count = current,
            }
        }
        None
    }
}
pub struct StreamPermit(Arc<StreamSlots>);
impl Drop for StreamPermit {
    fn drop(&mut self) { self.0.active.fetch_sub(1, Ordering::AcqRel); }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn three_devices_fit_and_disconnect_releases_only_its_slot() {
        let slots = Arc::new(StreamSlots::default());
        let a=slots.try_acquire().unwrap();let b=slots.try_acquire().unwrap();let c=slots.try_acquire().unwrap();
        assert!(slots.try_acquire().is_none());
        drop(b);assert_eq!(slots.count(),2);
        let replacement=slots.try_acquire().unwrap();
        assert!(slots.try_acquire().is_none());
        drop((a,c,replacement));assert_eq!(slots.count(),0);
    }
    #[test]
    fn racing_connections_never_exceed_three() {
        let slots=Arc::new(StreamSlots::default());
        let ready=Arc::new(std::sync::Barrier::new(9));let finish=Arc::new(std::sync::Barrier::new(9));
        let workers:Vec<_>=(0..8).map(|_| {
            let(s,r,f)=(slots.clone(),ready.clone(),finish.clone());
            std::thread::spawn(move||{let permit=s.try_acquire();r.wait();f.wait();permit.is_some()})
        }).collect();
        ready.wait();assert_eq!(slots.count(),3);finish.wait();
        assert_eq!(workers.into_iter().map(|w| w.join().unwrap()).filter(|ok| *ok).count(),3);
        assert_eq!(slots.count(),0);
    }
    #[test]
    fn failed_start_returns_its_reservation() {
        let slots=Arc::new(StreamSlots::default());
        fn fail(slots:&Arc<StreamSlots>)->Result<(),()> {let _permit=slots.try_acquire().unwrap();Err(())}
        assert!(fail(&slots).is_err());assert_eq!(slots.count(),0);
    }
}
