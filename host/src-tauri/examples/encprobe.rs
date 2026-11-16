//! Temporary local probe: the real capture -> convert -> encode scheduling loop.
//! Prints cadence histograms. Delete after use.
use pc_remote_host::engine::{
    capture::SharedCapture,
    convert::Convert,
    encode::{codecs_available, Encoder},
    now_us, Com,
};
use pc_remote_host::prs::frame::Codec;
use std::time::{Duration, Instant};

fn poll(cap: &mut SharedCapture, until: Instant) -> Option<pc_remote_host::engine::capture::GpuFrame> {
    loop {
        let f = cap.next(Duration::ZERO);
        if f.is_some() || Instant::now() >= until {
            return f;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn hist(name: &str, mut v: Vec<u64>) {
    if v.is_empty() {
        println!("{name}: none");
        return;
    }
    v.sort_unstable();
    let n = v.len();
    let get = |p: f64| v[((n - 1) as f64 * p).round() as usize];
    let mut bins = std::collections::BTreeMap::new();
    for t in &v {
        *bins.entry(t / 1000).or_insert(0u32) += 1;
    }
    println!(
        "{name}: n={n} mean={}us p50={}us p95={}us max={}us ms_hist={:?}",
        v.iter().sum::<u64>() / n as u64,
        get(0.5),
        get(0.95),
        v.last().copied().unwrap_or(0),
        bins
    );
}

fn main() {
    let _com = Com::init();
    let Some(mut cap) = SharedCapture::open("") else {
        println!("open failed");
        return;
    };
    let (dev, ctx, luid, _) = cap.device();
    let avail = codecs_available(luid);
    println!("codecs={avail:?} size={:?}", cap.size());
    if !avail.contains(&Codec::H264) {
        println!("no h264");
        return;
    }
    let size = cap.size().unwrap_or((1920, 1080));
    let fps = 60u32;
    let mut enc = Encoder::new(&dev, luid, Codec::H264, size, fps, 20000).unwrap();
    let mut conv = Convert::new(&dev, &ctx, size, size).ok().unwrap();

    if std::env::args().nth(1).as_deref() == Some("fixed") {
        // Steady 60 Hz input grid: isolates the encoder's own output cadence.
        let mut src = None;
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            if let Some(f) = cap.next(Duration::ZERO) {
                if f.tex.is_some() {
                    src = f.tex;
                    break;
                }
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let Some(src) = src else {
            println!("no source frame");
            return;
        };
        let mut encoded = Vec::new();
        let mut durations = Vec::new();
        let (mut last, mut n) = (None::<Instant>, 0u64);
        let start = Instant::now();
        let mut next = start;
        while std::time::Instant::now() < start + Duration::from_secs(6) {
            next += Duration::from_nanos(16_666_667);
            if let Some(w) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(w);
            }
            let nv12 = conv.convert(&src).ok().unwrap();
            let began = Instant::now();
            let outs = enc.encode(&nv12, now_us(), false).unwrap();
            durations.push(began.elapsed().as_micros() as u64);
            for _ in outs {
                n += 1;
                let t = Instant::now();
                if let Some(prev) = last.replace(t) {
                    encoded.push(t.duration_since(prev).as_micros() as u64);
                }
            }
        }
        println!("fixed 60Hz: encoded={n}");
        let head: Vec<u64> = encoded.iter().take(90).map(|x| x / 1000).collect();
        println!("encoded.sequence_ms={head:?}");
        hist("encoded.interval", encoded);
        hist("encode.duration", durations);
        return;
    }

    let interval = Duration::from_secs(1) / fps;
    let (mut last_enc, mut pending) = (Instant::now(), false);
    let (mut tokens, mut refill) = (2.0f64, Instant::now());
    let mut img: Option<std::sync::Arc<pc_remote_host::engine::capture::FrameTexture>> = None;
    let (mut cap_us, mut last_capture_us) = (0u64, 0u64);
    let (mut dups, mut total) = (0u32, 0u32);

    let mut capture_i = Vec::new();
    let mut selected_i = Vec::new();
    let mut encoded_i = Vec::new();
    let mut convert_d = Vec::new();
    let mut encode_d = Vec::new();
    let (mut last_capture, mut last_selected, mut last_encoded): (Option<Instant>, Option<Instant>, Option<Instant>) = (None, None, None);
    let mut last_capture_image = 0u64;

    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        let now = Instant::now();
        tokens = (tokens + now.duration_since(refill).as_secs_f64() * fps as f64).min(2.0);
        refill = now;
        let due = if tokens >= 1.0 { now } else { now + interval.mul_f64(1.0 - tokens) };
        if !(pending && now >= due) {
            let got = if pending {
                std::thread::sleep(due - now);
                cap.next(Duration::ZERO)
            } else {
                poll(&mut cap, (last_enc + interval * 2).max(due))
            };
            if let Some(f) = &got {
                if let Some(_tex) = &f.tex {
                    let t = Instant::now();
                    if let Some(prev) = last_capture.replace(t) {
                        capture_i.push(t.duration_since(prev).as_micros() as u64);
                    }
                    img = f.tex.clone();
                    cap_us = f.capture_us;
                    pending = true;
                    last_capture_image += 1;
                }
            }
            if Instant::now() < due || (got.is_some() && !pending) {
                continue;
            }
        }
        let t = Instant::now();
        if let Some(prev) = last_selected.replace(t) {
            selected_i.push(t.duration_since(prev).as_micros() as u64);
        }
        let repeat = !pending;
        pending = false;
        last_enc = t;
        tokens -= 1.0;
        let Some(src) = img.as_ref() else { continue };
        let began = Instant::now();
        let Some(nv12) = conv.convert(src).ok() else { continue };
        convert_d.push(began.elapsed().as_micros() as u64);
        let began = Instant::now();
        let Ok(outs) = enc.encode(&nv12, if repeat { now_us() } else { cap_us }, false) else { break };
        encode_d.push(began.elapsed().as_micros() as u64);
        for e in outs {
            total += 1;
            if repeat || e.capture_us == last_capture_us {
                dups += 1;
            }
            last_capture_us = e.capture_us;
            let t = Instant::now();
            if let Some(prev) = last_encoded.replace(t) {
                encoded_i.push(t.duration_since(prev).as_micros() as u64);
            }
        }
        if last_capture_image > 0 && total > 0 && total % 120 == 0 {
            // live update so a static screen is obvious
            eprintln!("progress selected={} encoded={} repeats={}", selected_i.len(), total, dups);
        }
    }
    println!("selected={} encoded={} repeats_or_dups={dups}", selected_i.len(), total);
    let seq = |name: &str, v: &[u64]| {
        let head: Vec<u64> = v.iter().take(90).map(|x| x / 1000).collect();
        println!("{name} sequence_ms={head:?}");
    };
    seq("capture", &capture_i);
    seq("selected", &selected_i);
    seq("encoded", &encoded_i);
    hist("capture.interval", capture_i);
    hist("selected.interval", selected_i);
    hist("encoded.interval", encoded_i);
    hist("convert.duration", convert_d);
    hist("encode.duration", encode_d);
}
