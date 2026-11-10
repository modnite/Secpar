//! Two (or three) viewers on one Windows host, with the real capture, convert and encode path.
//!
//! Each viewer is a headless PRS receiver: it rebuilds frames, decodes them with OpenH264 and
//! measures corruption. Needs an unlocked desktop and a GPU encoder, so it is ignored by default:
//! `cargo test --test multi_client_video -- --ignored --nocapture`.
#![cfg(windows)]

use openh264::{decoder::Decoder, formats::YUVSource};
use pc_remote_host::engine::session::{Params, StreamSession};
use pc_remote_host::proto::ServerMsg;
use secpar_stream::frame::assemble;
use secpar_stream::{Crypt, Dir, Header, Type};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

#[derive(Default, Clone)]
struct Stats {
    frames: u32,
    /// Frames with a visible share of chroma outside the legal 16..240 range (zero chroma = green).
    bad_chroma: u32,
    worst_bad_share: f64,
    /// Frames far from the other viewer's newest frame (same screen, same size).
    mismatched: u32,
    worst_diff: f64,
    compared: u32,
    decode_errors: u32,
    key_requests: u32,
}

struct Viewer {
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<Stats>>,
    thread: std::thread::JoinHandle<()>,
    session: StreamSession,
}

type Grid = Arc<Mutex<Option<(Vec<u8>, Instant)>>>;

const GRID: usize = 64;

fn start(label: &'static str, w: u32, h: u32, mine: Grid, other: Option<Grid>) -> Viewer {
    let p = Params { w, h, fps: 60, codecs: vec!["h264".into()], max_kbps: 8000, audio: false, cands: vec![], public: None, display: String::new() };
    let (ready, session) = StreamSession::start(p, IpAddr::from([127, 0, 0, 1]), Arc::new(|_| {}), Box::new(|_| {})).expect("stream must start");
    let ServerMsg::StreamReady { port, session: sid, key, .. } = ready else { panic!("no stream.ready") };
    let key: [u8; 32] = b64(&key).try_into().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(Mutex::new(Stats::default()));
    let (done, st) = (stop.clone(), stats.clone());
    let thread = std::thread::spawn(move || {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(10))).unwrap();
        let crypt = Crypt::new(&key, sid);
        let host: SocketAddr = ([127, 0, 0, 1], port).into();
        let mut seq = 0u32;
        let mut send = |typ: Type, plain: &[u8]| {
            let h = Header { typ, flags: 0, k: 0, seq, frame_id: 0, frag: 0, count: 0 };
            seq += 1;
            let _ = sock.send_to(&crypt.seal(Dir::PhoneToHost, &h, plain), host);
        };
        let mut dec = Decoder::new().unwrap();
        struct Part {
            k: usize,
            p: usize,
            shards: Vec<Option<Vec<u8>>>,
            at: Instant,
        }
        let (mut partial, mut done_frames) = (BTreeMap::<u32, Part>::new(), BTreeMap::<u32, (bool, Vec<u8>)>::new());
        let (mut next, mut need_key, mut got_video) = (None::<u32>, true, false);
        let (mut last_probe, mut last_idr) = (Instant::now() - Duration::from_secs(1), Instant::now() - Duration::from_secs(1));
        let mut buf = [0u8; 2048];
        while !done.load(Ordering::Relaxed) {
            if last_probe.elapsed() > Duration::from_millis(if got_video { 1000 } else { 200 }) {
                let mut pr = [0u8; 16];
                pr[..8].copy_from_slice(&1u64.to_be_bytes());
                send(Type::Probe, &pr);
                last_probe = Instant::now();
            }
            if let Ok((n, _)) = sock.recv_from(&mut buf) {
                if let Some((hd, plain)) = crypt.open(Dir::HostToPhone, &buf[..n]) {
                    if matches!(hd.typ, Type::Video | Type::Fec) {
                        got_video = true;
                        if !done_frames.contains_key(&hd.frame_id) {
                            let f = partial.entry(hd.frame_id).or_insert_with(|| Part { k: hd.k as usize, p: hd.flags as usize, shards: vec![None; hd.count as usize], at: Instant::now() });
                            f.shards[hd.frag as usize] = Some(plain);
                            if f.shards.iter().flatten().count() >= f.k {
                                if let Some((fh, data)) = assemble(f.k, f.p, f.shards.clone()) {
                                    done_frames.insert(hd.frame_id, (fh.keyframe, data));
                                    partial.remove(&hd.frame_id);
                                }
                            }
                        }
                    }
                }
            }
            if next.is_none() {
                next = done_frames.iter().find(|(_, (k, _))| *k).map(|(id, _)| *id);
            }
            while let Some(id) = next {
                let Some((key, data)) = done_frames.remove(&id) else { break };
                next = Some(id.wrapping_add(1));
                done_frames.retain(|k, _| *k >= id);
                if need_key && !key {
                    continue;
                }
                need_key = false;
                match dec.decode(&data) {
                    Ok(Some(yuv)) => {
                        let (w, h) = yuv.dimensions();
                        let (sy, su, sv) = yuv.strides();
                        let (ys, us, vs) = (yuv.y(), yuv.u(), yuv.v());
                        let (mut bad, mut total) = (0u64, 0u64);
                        for r in 0..h / 2 {
                            for c in 0..w / 2 {
                                let (u, v) = (us[r * su + c], vs[r * sv + c]);
                                total += 1;
                                if u < 16 || v < 16 {
                                    bad += 1;
                                }
                            }
                        }
                        let mut grid = vec![0u8; GRID * GRID];
                        for gy in 0..GRID {
                            for gx in 0..GRID {
                                grid[gy * GRID + gx] = ys[(gy * h / GRID) * sy + gx * w / GRID];
                            }
                        }
                        let share = bad as f64 / total.max(1) as f64;
                        let mut s = st.lock().unwrap();
                        s.frames += 1;
                        if share > 0.02 {
                            s.bad_chroma += 1;
                        }
                        s.worst_bad_share = s.worst_bad_share.max(share);
                        if let Some(o) = &other {
                            if let Some((og, at)) = o.lock().unwrap().as_ref() {
                                if at.elapsed() < Duration::from_millis(100) {
                                    let d = grid.iter().zip(og).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as f64).sum::<f64>() / grid.len() as f64;
                                    s.compared += 1;
                                    s.worst_diff = s.worst_diff.max(d);
                                    if d > 12.0 {
                                        s.mismatched += 1;
                                    }
                                }
                            }
                        }
                        drop(s);
                        *mine.lock().unwrap() = Some((grid, Instant::now()));
                    }
                    Ok(None) => {}
                    Err(_) => {
                        st.lock().unwrap().decode_errors += 1;
                        need_key = true;
                    }
                }
            }
            // A frame that never completes: skip it and wait for a keyframe.
            if let Some(id) = next {
                if partial.get(&id).is_some_and(|f| f.at.elapsed() > Duration::from_millis(60)) || done_frames.keys().next_back().is_some_and(|l| l.wrapping_sub(id) > 2 && *l > id) {
                    partial.remove(&id);
                    next = Some(id.wrapping_add(1));
                    need_key = true;
                }
            }
            if need_key && got_video && last_idr.elapsed() > Duration::from_millis(250) {
                send(Type::Idr, &[]);
                last_idr = Instant::now();
                st.lock().unwrap().key_requests += 1;
            }
        }
        let _ = label;
    });
    Viewer { stop, stats, thread, session }
}

fn stop(v: Viewer) -> Stats {
    v.stop.store(true, Ordering::Relaxed);
    v.thread.join().unwrap();
    drop(v.session);
    let s = v.stats.lock().unwrap().clone();
    s
}

fn report(label: &str, s: &Stats) {
    eprintln!(
        "{label}: frames {} bad_chroma_frames {} worst_bad_chroma_share {:.4} compared {} mismatched {} worst_grid_diff {:.2} decode_errors {} key_requests {}",
        s.frames, s.bad_chroma, s.worst_bad_share, s.compared, s.mismatched, s.worst_diff, s.decode_errors, s.key_requests
    );
}

#[test]
#[ignore = "Requires an unlocked Windows desktop and a hardware encoder"]
fn two_viewers_get_clean_video() {
    let (ga, gb): (Grid, Grid) = (Default::default(), Default::default());
    // Phase 1: one viewer alone.
    let a = start("a", 1280, 720, ga.clone(), None);
    std::thread::sleep(Duration::from_secs(4));
    let alone = a.stats.lock().unwrap().clone();
    report("single viewer", &alone);
    // Phase 2: a second viewer joins; both run together.
    let before = alone.frames;
    let b = start("b", 1280, 720, gb.clone(), Some(ga.clone()));
    let at_join = a.stats.lock().unwrap().clone();
    std::thread::sleep(Duration::from_secs(10));
    let together_a = a.stats.lock().unwrap().clone();
    let together_b = b.stats.lock().unwrap().clone();
    // Phase 3: the second viewer leaves; the first keeps going.
    let sb = stop(b);
    let at_leave = a.stats.lock().unwrap().clone();
    std::thread::sleep(Duration::from_secs(3));
    let after = a.stats.lock().unwrap().clone();
    let sa = stop(a);
    report("viewer a, both connected (cumulative)", &together_a);
    report("viewer b", &together_b);
    report("viewer b final", &sb);
    eprintln!(
        "viewer a frames: alone {before}, while sharing {}, after b left {}; new bad chroma frames sharing {}, after {}; decode errors total {}",
        at_leave.frames - at_join.frames,
        after.frames - at_leave.frames,
        at_leave.bad_chroma - at_join.bad_chroma,
        after.bad_chroma - at_leave.bad_chroma,
        sa.decode_errors
    );
    assert!(alone.frames > 30 && sb.frames > 30, "both viewers must receive video");
    assert_eq!(sa.bad_chroma + sb.bad_chroma, 0, "corrupt chroma (green) frames");
    assert_eq!(sa.mismatched + sb.mismatched, 0, "frames that differ from the other viewer's");
    assert_eq!(sa.decode_errors + sb.decode_errors, 0, "decoder errors");
    assert!(after.frames - at_leave.frames > 10, "the first viewer keeps streaming after the second leaves");
}
