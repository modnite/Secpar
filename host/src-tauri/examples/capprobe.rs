//! Temporary local probe: counts DXGI frames and their cadence. Delete after use.
use pc_remote_host::engine::{capture::SharedCapture, Com};
use std::time::{Duration, Instant};

fn main() {
    let _com = Com::init();
    let Some(mut cap) = SharedCapture::open("") else {
        println!("open failed");
        return;
    };
    println!("opened size={:?}", cap.size());
    let mut last: Option<Instant> = None;
    let mut times = Vec::new();
    let mut frames = 0u32;
    let mut ptrs = 0u32;
    let end = Instant::now() + Duration::from_secs(4);
    while Instant::now() < end {
        if let Some(f) = cap.next(Duration::ZERO) {
            if f.tex.is_some() {
                frames += 1;
                let now = Instant::now();
                if let Some(prev) = last.replace(now) {
                    times.push(now.duration_since(prev).as_micros() as u64);
                }
            }
            if f.pointer.is_some() {
                ptrs += 1;
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    times.sort_unstable();
    let n = times.len();
    let get = |p: f64| if n == 0 { 0 } else { times[((n - 1) as f64 * p).round() as usize] };
    println!(
        "frames={frames} ptr_events={ptrs} p50={}us p95={}us max={}us",
        get(0.5),
        get(0.95),
        times.last().copied().unwrap_or(0)
    );
    let mut bins = std::collections::BTreeMap::new();
    for t in &times {
        *bins.entry(t / 1000).or_insert(0u32) += 1;
    }
    println!("ms histogram: {:?}", bins);
}
