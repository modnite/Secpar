//! Records real encoder output from this Windows host into files, for decoder measurements.
//!
//! Needs an unlocked desktop and a GPU encoder, so it is ignored by default:
//! `SECPAR_CAPTURE_DIR=D:/x cargo test --test capture_stream -- --ignored --nocapture`.
//! Each file is a run of records: u32 big-endian length, one keyframe byte, the access unit.
#![cfg(windows)]

use pc_remote_host::engine::session::{Params, StreamSession};
use pc_remote_host::proto::ServerMsg;
use secpar_stream::{Crypt, Dir, Header, Type};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn b64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| *c != b'=') {
        acc = (acc << 6) | val(c) as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}


fn record(name: &str, codec: &str, w: u32, h: u32, seconds: u64) -> Vec<(bool, Vec<u8>)> {
    let p = Params { w, h, fps: 60, codecs: vec![codec.into()], max_kbps: 30000, audio: false, cands: vec![], public: None, display: String::new(), gate: None, encoder: Default::default() };
    let (ready, session) = StreamSession::start(p, IpAddr::from([127, 0, 0, 1]), Arc::new(|_| {}), Box::new(|_| {})).expect("stream must start");
    let ServerMsg::StreamReady { port, session: sid, key, .. } = ready else { panic!("no stream.ready") };
    let key: [u8; 32] = b64(&key).try_into().unwrap();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    // A keyframe can exceed Windows' default UDP receive queue. Match the
    // Android receive queue so this measures recovery rather than fixture loss.
    {
        use std::os::windows::io::AsRawSocket;
        use windows::Win32::Networking::WinSock::{setsockopt, SOCKET, SOL_SOCKET, SO_RCVBUF};
        let bytes=(4i32 << 20).to_ne_bytes();
        // SAFETY: the socket is live and WinSock copies the four-byte option.
        assert_eq!(unsafe {setsockopt(SOCKET(sock.as_raw_socket() as usize), SOL_SOCKET, SO_RCVBUF, Some(&bytes))},0);
    }
    sock.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
    let crypt = Crypt::new(&key, sid);
    let host: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut seq = 0u32;
    let mut send = |typ: Type, plain: &[u8]| {
        let h = Header { typ, flags: 0, k: 0, seq, frame_id: 0, frag: 0, count: 0 };
        seq += 1;
        let _ = sock.send_to(&crypt.seal(Dir::PhoneToHost, &h, plain), host);
    };
    let mut assembler = secpar_client::receiver::Assembler::default();
    let mut out: Vec<(bool, Vec<u8>)> = Vec::new();
    let mut arrivals: Vec<u64> = Vec::new();
    let mut capture_ages: Vec<u64> = Vec::new();
    let mut last_arrival: Option<Instant> = None;
    let mut drop_first_key = std::env::var_os("SECPAR_CAPTURE_DROP_FIRST_KEYFRAME").is_some();
    let (mut last_probe, mut last_idr, mut keyed) = (Instant::now() - Duration::from_secs(1), Instant::now() - Duration::from_secs(1), false);
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut buf = [0u8; 2048];
    while Instant::now() < end {
        if last_probe.elapsed() > Duration::from_millis(200) {
            let mut pr = [0u8; 16];
            pr[..8].copy_from_slice(&1u64.to_be_bytes());
            send(Type::Probe, &pr);
            last_probe = Instant::now();
        }
        if !keyed && last_idr.elapsed() > Duration::from_millis(250) {
            send(Type::Idr, &[]);
            last_idr = Instant::now();
        }
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        let Some((hd, plain)) = crypt.open(Dir::HostToPhone, &buf[..n]) else { continue };
        if let Some(frame) = assembler.push(hd, plain, Instant::now()) {
            let (fh, data) = (frame.header, frame.data);
            // Start at the first keyframe; later gaps are not expected on loopback.
            if fh.keyframe {
                if std::mem::take(&mut drop_first_key) {
                    continue;
                }
                keyed = true;
            }
            if keyed {
                if fh.capture_us > 0 {
                    capture_ages.push(pc_remote_host::engine::now_us().saturating_sub(fh.capture_us));
                }
                let now = Instant::now();
                if let Some(prev) = last_arrival.replace(now) {
                    arrivals.push(now.duration_since(prev).as_micros() as u64);
                }
                out.push((fh.keyframe, data));
            }
        }
    }
    drop(session);
    // Host and receiver share a monotonic clock in this local fixture. This
    // includes encoding, packet creation, sending and frame reconstruction.
    let mut ages:Vec<_>=capture_ages.into_iter().skip(120).collect();
    if !ages.is_empty() {
        ages.sort_unstable();let n=ages.len();
        let get=|p:f64|ages[((n-1) as f64*p).round() as usize];
        eprintln!("{name} capture_to_receive.steady: n={n} p50={}us p95={}us p99={}us max={}us",get(0.5),get(0.95),get(0.99),ages[n-1]);
    }
    if !arrivals.is_empty() {
        let mut sorted = arrivals.clone();
        sorted.sort_unstable();
        let n = sorted.len();
        let get = |p: f64| sorted[((n - 1) as f64 * p).round() as usize];
        let mut bins = BTreeMap::new();
        for t in &arrivals {
            *bins.entry(t / 1000).or_insert(0u32) += 1;
        }
        eprintln!(
            "{name} arrival: n={n} mean={}us p50={}us p95={}us max={}us ms_hist={bins:?}",
            arrivals.iter().sum::<u64>() / n as u64,
            get(0.5),
            get(0.95),
            sorted.last().copied().unwrap_or(0)
        );
        let head: Vec<u64> = arrivals.iter().skip(120).take(90).map(|x| x / 1000).collect();
        eprintln!("{name} arrival.steady_sequence_ms={head:?}");
        let late: Vec<u64> = arrivals.iter().skip(120).copied().collect();
        if !late.is_empty() {
            let mut l = late.clone();
            l.sort_unstable();
            let n = l.len();
            let get = |p: f64| l[((n - 1) as f64 * p).round() as usize];
            let over = l.iter().filter(|t| **t > 25_000).count();
            eprintln!(
                "{name} arrival.steady: n={n} p50={}us p95={}us p99={}us max={}us over25ms={over}",
                get(0.5), get(0.95), get(0.99), l.last().copied().unwrap_or(0)
            );
        }
    }
    out
}

fn save(path: &std::path::Path, frames: &[(bool, Vec<u8>)]) {
    let mut bytes = Vec::new();
    for (key, data) in frames {
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.push(*key as u8);
        bytes.extend_from_slice(data);
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
#[ignore = "Requires an unlocked Windows desktop and a hardware encoder"]
fn capture_streams() {
    let dir = std::path::PathBuf::from(std::env::var("SECPAR_CAPTURE_DIR").expect("SECPAR_CAPTURE_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let seconds: u64 = std::env::var("SECPAR_CAPTURE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    let only = std::env::var("SECPAR_CAPTURE_ONLY").ok();
    for (name, codec, w, h) in [("h264_1080", "h264", 1920, 1080), ("h264_720", "h264", 1280, 720), ("hevc_1080", "hevc", 1920, 1080)] {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let frames = record(name, codec, w, h, seconds);
        assert!(!frames.is_empty(), "{name}: no authenticated video frames were assembled");
        let keys = frames.iter().filter(|f| f.0).count();
        assert!(keys > 0, "{name}: stream never delivered a keyframe");
        let bytes: usize = frames.iter().map(|f| f.1.len()).sum();
        eprintln!("{name}: {} frames, {keys} keyframes, {} KiB", frames.len(), bytes / 1024);
        if std::env::var_os("SECPAR_CAPTURE_STATS_ONLY").is_none() {
            save(&dir.join(format!("{name}.bin")), &frames);
        }
    }
}
