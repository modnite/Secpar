//! Records real encoder output from this Windows host into files, for decoder measurements.
//!
//! Needs an unlocked desktop and a GPU encoder, so it is ignored by default:
//! `SECPAR_CAPTURE_DIR=D:/x cargo test --test capture_stream -- --ignored --nocapture`.
//! Each file is a run of records: u32 big-endian length, one keyframe byte, the access unit.
#![cfg(windows)]

use pc_remote_host::engine::session::{Params, StreamSession};
use pc_remote_host::proto::ServerMsg;
use secpar_stream::frame::assemble;
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


fn record(codec: &str, w: u32, h: u32, seconds: u64) -> Vec<(bool, Vec<u8>)> {
    let p = Params { w, h, fps: 60, codecs: vec![codec.into()], max_kbps: 30000, audio: false, cands: vec![], public: None, display: String::new() };
    let (ready, session) = StreamSession::start(p, IpAddr::from([127, 0, 0, 1]), Arc::new(|_| {}), Box::new(|_| {})).expect("stream must start");
    let ServerMsg::StreamReady { port, session: sid, key, .. } = ready else { panic!("no stream.ready") };
    let key: [u8; 32] = b64(&key).try_into().unwrap();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
    let crypt = Crypt::new(&key, sid);
    let host: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut seq = 0u32;
    let mut send = |typ: Type, plain: &[u8]| {
        let h = Header { typ, flags: 0, k: 0, seq, frame_id: 0, frag: 0, count: 0 };
        seq += 1;
        let _ = sock.send_to(&crypt.seal(Dir::PhoneToHost, &h, plain), host);
    };
    struct Part {
        k: usize,
        p: usize,
        shards: Vec<Option<Vec<u8>>>,
    }
    let mut partial = BTreeMap::<u32, Part>::new();
    let mut out: Vec<(bool, Vec<u8>)> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
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
        if !matches!(hd.typ, Type::Video | Type::Fec) || seen.contains(&hd.frame_id) {
            continue;
        }
        let f = partial.entry(hd.frame_id).or_insert_with(|| Part { k: hd.k as usize, p: hd.flags as usize, shards: vec![None; hd.count as usize] });
        f.shards[hd.frag as usize] = Some(plain);
        if f.shards.iter().flatten().count() >= f.k {
            let f = partial.remove(&hd.frame_id).unwrap();
            if let Some((fh, data)) = assemble(f.k, f.p, f.shards) {
                seen.insert(hd.frame_id);
                // Start at the first keyframe; later gaps are not expected on loopback.
                if fh.keyframe {
                    keyed = true;
                }
                if keyed {
                    out.push((fh.keyframe, data));
                }
            }
        }
    }
    drop(session);
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
    for (name, codec, w, h) in [("h264_1080", "h264", 1920, 1080), ("h264_720", "h264", 1280, 720), ("hevc_1080", "hevc", 1920, 1080)] {
        let frames = record(codec, w, h, seconds);
        let keys = frames.iter().filter(|f| f.0).count();
        let bytes: usize = frames.iter().map(|f| f.1.len()).sum();
        eprintln!("{name}: {} frames, {keys} keyframes, {} KiB", frames.len(), bytes / 1024);
        save(&dir.join(format!("{name}.bin")), &frames);
    }
}
