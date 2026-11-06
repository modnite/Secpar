//! Native hosting engine embedded in the one Secpar Linux desktop app.
#[cfg(target_os = "linux")]
pub mod account;
#[cfg(target_os = "linux")]
pub mod backend;
#[cfg(target_os = "linux")]
pub mod capture;
pub mod identity;
#[cfg(target_os = "linux")]
pub mod input;
#[cfg(target_os = "linux")]
pub mod portal;
pub mod store;
#[cfg(target_os = "linux")]
pub mod stream;

#[cfg(target_os = "linux")]
pub fn now_us() -> u64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // CLOCK_MONOTONIC is also the clock used by portal/GStreamer timestamps.
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t);
    }
    t.tv_sec as u64 * 1_000_000 + t.tv_nsec as u64 / 1000
}
