use super::*;

#[test]
fn idle_frames_keep_two_slot_cadence_when_timer_wakeups_vary() {
    let interval=Duration::from_nanos(16_666_667);
    let origin=Instant::now();let mut sent=origin;let mut sent_slot=0;
    let jitter=[700i64,-400,900,-600,300,-700];
    let mut count=0;
    for slot in 1..=120 {
        let due=origin+interval*slot;
        let us=jitter[(slot as usize-1)%jitter.len()];
        let woke=if us>=0 {due+Duration::from_micros(us as u64)} else {due-Duration::from_micros((-us) as u64)};
        if idle_repeat_due(Duration::from_secs(1),woke.duration_since(sent),interval) {
            assert_eq!(slot-sent_slot,2);sent=woke;sent_slot=slot;count+=1;
        }
    }
    assert_eq!(count,60);
    assert!(idle_repeat_due(Duration::from_millis(100),Duration::ZERO,interval));
}

#[test]
fn packet_schedule_finishes_systematic_data_at_the_same_offset_for_every_size() {
    for size in [1, 1200, 4800, 8400, 9600, 20_000, 80_000, 160_000, 400_000] {
        for fec in [5, 20, 50] {
            let data:Vec<u8>=(0..size).map(|i|(i%251) as u8).collect();
            let fh=FrameHeader{len:size as u32,capture_us:0,keyframe:true,codec:Codec::H264};
            let packets=packetize(0,&fh,&data,fec);
            let (width,offset)=video_packet_schedule(&packets);
            let mut shards=vec![None;packets.len()];
            let mut completed=None;
            for (batch,chunk) in packets.chunks(width).enumerate() {
                for (h,payload) in chunk {shards[h.frag as usize]=Some(payload.clone());}
                if let Some(frame)=assemble(packets[0].0.k as usize,fec,shards.clone()) {
                    completed=Some((offset+batch as u64,frame));break;
                }
            }
            let (at,(header,payload))=completed.expect("a lossless frame must assemble");
            assert_eq!(at,3,"size={size} fec={fec}");
            assert_eq!(header,fh);assert_eq!(payload,data);
            let batches=packets.len().div_ceil(width);
            assert!(batches<=4);
            assert!(offset+(batches-1) as u64<=4,"parity must remain within the pacing window");
        }
    }
}
use crate::prs::frame::assemble;
use std::collections::BTreeMap;
use std::sync::mpsc;

#[test]
fn host_quality_ceiling_limits_native_and_requested_sizes_without_upscaling() {
    assert_eq!(quality_limit("smooth",0,0,120),(1920,1080,60));
    assert_eq!(quality_limit("battery",3840,2160,60),(1280,720,30));
    assert_eq!(quality_limit("sharp",1280,720,30),(1280,720,30));
    assert_eq!(quality_limit("2160p60",7680,4320,120),(3840,2160,60));
    assert_eq!(quality_limit("auto",0,0,120),(0,0,120));
}

const KEY: [u8; 32] = [7; 32];

/// Exercises actual concurrent capture/encoding, rather than only the slot counter.
#[test]
#[ignore = "Requires an unlocked Windows desktop and hardware encoders; run explicitly"]
fn concurrent_hardware_streams_deliver_independent_video() {
    fn start(codecs: Vec<String>) -> (StreamSession, Arc<AtomicBool>, std::thread::JoinHandle<u32>) {
        let params=Params{w:640,h:360,fps:30,codecs,max_kbps:3000,audio:false,cands:vec![],public:None,display:String::new(),gate:None};
        let (ready,session)=StreamSession::start(params,IpAddr::from([127,0,0,1]),Arc::new(|_|{}),Box::new(|_|{})).expect("Concurrent hardware stream must start");
        let ServerMsg::StreamReady{port,session:sid,key,codec,..}=ready else {panic!("Missing stream readiness")};
        eprintln!("Concurrent hardware stream opened: {codec}");
        let key:[u8;32]=STANDARD.decode(key).unwrap().try_into().unwrap();
        let mut peer=Phone::new(&key,sid); peer.sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let stop=Arc::new(AtomicBool::new(false)); let done=stop.clone();
        let worker=std::thread::spawn(move || {
            let address=SocketAddr::from(([127,0,0,1],port)); let mut packets=0; let mut probe=Instant::now()-Duration::from_secs(1);
            while !done.load(Ordering::Relaxed) {
                if probe.elapsed()>Duration::from_millis(200) {peer.probe(address,1);probe=Instant::now();}
                if let Some((header,_))=peer.recv() {if header.typ==Type::Video {packets+=1;}}
            }
            packets
        });
        (session,stop,worker)
    }
    let (first,stop_first,read_first)=start(vec!["av1".into(),"hevc".into(),"h264".into()]);
    let (second,stop_second,read_second)=start(vec!["h264".into()]);
    let (third,stop_third,read_third)=start(vec!["h264".into()]);
    assert_eq!(SharedCapture::active_source_count(),1,"All viewers share one monitor capture");
    let overflow=Params{w:640,h:360,fps:30,codecs:vec!["h264".into()],max_kbps:3000,audio:false,cands:vec![],public:None,display:String::new(),gate:None};
    assert!(matches!(StreamSession::start(overflow,IpAddr::from([127,0,0,1]),Arc::new(|_|{}),Box::new(|_|{})),Err(ServerMsg::StreamError{code,..}) if code=="connection_limit"));
    std::thread::sleep(Duration::from_secs(1));
    stop_first.store(true,Ordering::Relaxed); let first_packets=read_first.join().unwrap(); drop(first);
    let (replacement,stop_replacement,read_replacement)=start(vec!["h264".into()]);
    std::thread::sleep(Duration::from_millis(300));
    stop_second.store(true,Ordering::Relaxed); let second_packets=read_second.join().unwrap(); drop(second);
    stop_third.store(true,Ordering::Relaxed); let third_packets=read_third.join().unwrap(); drop(third);
    stop_replacement.store(true,Ordering::Relaxed); let replacement_packets=read_replacement.join().unwrap(); drop(replacement);
    assert!(first_packets>0,"First session received no video");
    assert!(second_packets>0,"Second session received no video");
    assert!(third_packets>0,"Third session received no video");
    assert!(replacement_packets>0,"Replacement session received no video");
    assert_eq!(SharedCapture::active_source_count(),0,"Capture stops after the last viewer leaves");
    eprintln!("Independent video packets received: {first_packets}, {second_packets}, {third_packets}; replacement {replacement_packets}");
}

struct Phone {
    sock: UdpSocket,
    crypt: Crypt,
    seq: u32,
}

impl Phone {
    fn new(key: &[u8; 32], session: u32) -> Phone {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        Phone { sock, crypt: Crypt::new(key, session), seq: 0 }
    }

    fn send(&mut self, to: SocketAddr, typ: Type, plain: &[u8]) {
        let h = Header { seq: self.seq, ..hdr(typ, 0) };
        self.seq += 1;
        self.sock.send_to(&self.crypt.seal(Dir::PhoneToHost, &h, plain), to).unwrap();
    }

    fn probe(&mut self, to: SocketAddr, t: u64) {
        let mut p = [0u8; 16];
        p[..8].copy_from_slice(&t.to_be_bytes());
        self.send(to, Type::Probe, &p);
    }

    fn recv(&self) -> Option<(Header, Vec<u8>)> {
        let mut b = [0u8; 2048];
        let (n, _) = self.sock.recv_from(&mut b).ok()?;
        self.crypt.open(Dir::HostToPhone, &b[..n])
    }
}

type Sink = mpsc::Receiver<ClientMsg>;

/// A host-side control loop on 127.0.0.1 with no media threads.
fn host(timeout: Duration) -> (Arc<Shared>, SocketAddr, Sink, std::thread::JoinHandle<()>) {
    host_with(timeout, |_| {})
}

fn host_with(timeout: Duration, setup: impl FnOnce(&mut Shared)) -> (Arc<Shared>, SocketAddr, Sink, std::thread::JoinHandle<()>) {
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sock.local_addr().unwrap();
    let mut sh = Shared::new(sock, &KEY, 42, Target { kbps: 10000, fec: 15, notch: 0 });
    setup(&mut sh);
    let sh = Arc::new(sh);
    let (tx, rx) = mpsc::channel();
    let s = sh.clone();
    let t = std::thread::spawn(move || {
        let mut br = Bitrate::new(10000, 3);
        control(&s, &mut |m| tx.send(m).unwrap(), &mut br, timeout)
    });
    (sh, addr, rx, t)
}

fn stop(sh: &Shared, t: std::thread::JoinHandle<()>) {
    sh.stop.store(true, Ordering::SeqCst);
    t.join().unwrap();
}

#[test]
fn follows_new_probe_address() {
    let (sh, host_addr, _, t) = host(TIMEOUT);
    let (mut a, mut b) = (Phone::new(&KEY, 42), Phone::new(&KEY, 42));
    a.probe(host_addr, 1234);
    let (h, p) = a.recv().expect("PROBE answered");
    assert_eq!(h.typ, Type::Probe);
    assert_eq!(&p[8..16], &1234u64.to_be_bytes(), "echoes the phone's t");
    assert_eq!(sh.peer(), Some(a.sock.local_addr().unwrap()));
    assert!(sh.idr.swap(false, Ordering::SeqCst), "first PROBE asks for a keyframe");

    // the phone moved networks: same session, new address
    b.seq = a.seq;
    b.probe(host_addr, 99);
    assert!(b.recv().is_some());
    assert_eq!(sh.peer(), Some(b.sock.local_addr().unwrap()));
    assert!(sh.send(hdr(Type::Cursor, 0), &[0; 5]));
    assert_eq!(b.recv().map(|(h, _)| h.typ), Some(Type::Cursor), "media follows the new address");
    assert!(a.recv().is_none(), "old address gets nothing");
    stop(&sh, t);
}

#[test]
fn ignores_unauthenticated_probe() {
    let (sh, host_addr, _, t) = host(TIMEOUT);
    let mut bad = Phone::new(&[9; 32], 42);
    bad.probe(host_addr, 1);
    assert!(bad.recv().is_none());
    assert_eq!(sh.peer(), None);

    // a replayed (old seq) PROBE from another address can't hijack the stream
    let mut a = Phone::new(&KEY, 42);
    a.seq = 10;
    a.probe(host_addr, 1);
    assert!(a.recv().is_some());
    let mut replay = Phone::new(&KEY, 42);
    replay.seq = 10;
    replay.probe(host_addr, 1);
    assert!(replay.recv().is_none());
    assert_eq!(sh.peer(), Some(a.sock.local_addr().unwrap()));
    stop(&sh, t);
}

#[test]
fn dedup_by_event_id() {
    let (sh, host_addr, rx, t) = host(TIMEOUT);
    let mut p = Phone::new(&KEY, 42);
    let ev = |i: u32| format!(r#"{{"i":{i},"t":"mouse.move","dx":{i},"dy":0}}"#);
    // each event goes out three times (newest plus the previous two), one packet lost, one reordered
    let packets = [vec![1], vec![2, 1], vec![4, 3, 2], vec![3, 2, 1], vec![5, 4, 3]];
    for ids in packets {
        let body = format!("[{}]", ids.iter().map(|&i| ev(i)).collect::<Vec<_>>().join(","));
        p.send(host_addr, Type::Input, body.as_bytes());
    }
    let mut got = Vec::new();
    while let Ok(m) = rx.recv_timeout(Duration::from_millis(500)) {
        got.push(m);
    }
    assert_eq!(got, (1..=5).map(|dx| ClientMsg::MouseMove { dx, dy: 0 }).collect::<Vec<_>>());
    stop(&sh, t);
}

#[test]
fn times_out_after_5s() {
    assert_eq!(TIMEOUT, Duration::from_secs(5));
    let (sh, host_addr, _, t) = host(Duration::from_millis(300));
    let mut p = Phone::new(&KEY, 42);
    let start = Instant::now();
    for _ in 0..4 {
        p.probe(host_addr, 1); // activity keeps it alive
        std::thread::sleep(Duration::from_millis(150));
    }
    assert!(!sh.stopped());
    t.join().unwrap();
    assert!(sh.stopped());
    let took = start.elapsed();
    assert!(took >= Duration::from_millis(700) && took < Duration::from_secs(2), "{took:?}");

    // BYE ends it at once
    let (sh, host_addr, _, t) = host(TIMEOUT);
    p.seq = 0;
    p.send(host_addr, Type::Bye, &[]);
    t.join().unwrap();
    assert!(sh.stopped());
}

#[test]
fn report_drives_bitrate_and_idr_is_rate_limited() {
    let (sh, host_addr, _, t) = host(TIMEOUT);
    let mut p = Phone::new(&KEY, 42);
    let mut report = [0u8; 16];
    report[4..6].copy_from_slice(&30u16.to_be_bytes()); // 3% loss
    report[10..12].copy_from_slice(&10u16.to_be_bytes());
    for _ in 0..3 {
        p.send(host_addr, Type::Report, &report);
    }
    p.send(host_addr, Type::Idr, &[]);
    // the receive thread shares the machine with other tests: let every packet land, then wait for the target
    std::thread::sleep(Duration::from_millis(200));
    let end = Instant::now() + Duration::from_secs(2);
    while *sh.target.lock().unwrap() != (Target { kbps: 7500, fec: 25, notch: 0 }) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*sh.target.lock().unwrap(), Target { kbps: 7500, fec: 25, notch: 0 });
    assert!(sh.idr.swap(false, Ordering::SeqCst));
    p.send(host_addr, Type::Idr, &[]);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!sh.idr.load(Ordering::SeqCst), "at most one keyframe per 250 ms");
    std::thread::sleep(Duration::from_millis(200));
    p.send(host_addr, Type::Idr, &[]);
    let end = Instant::now() + Duration::from_secs(2);
    while !sh.idr.load(Ordering::SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(sh.idr.load(Ordering::SeqCst));
    stop(&sh, t);
}

/// Shards of one frame as they arrive.
struct Partial {
    k: usize,
    p: usize,
    shards: Vec<Option<Vec<u8>>>,
    first_us: u64,
}

/// H6: real capture → hardware encode → PRS on 127.0.0.1 → a receiver built on `prs` that
/// drops 10% of media packets, rebuilds frames through FEC and asks for keyframes like the phone.
#[test]
fn loopback_streams_real_frames() {
    let p = Params { w: 1280, h: 720, fps: 60, codecs: vec!["h264".into()], max_kbps: 10000, audio: true, cands: vec![], public: None, display: String::new(), gate: None };
    let notify: Notify = Arc::new(|_| {});
    let (ready, session) = match StreamSession::start(p, IpAddr::from([127, 0, 0, 1]), notify, Box::new(|_| {})) {
        Ok(r) => r,
        Err(e) => return eprintln!("no stream on this PC: {e:?}"),
    };
    let ServerMsg::StreamReady { codec, w, h, port, session: sid, key, .. } = ready else { panic!("{ready:?}") };
    assert_eq!((codec.as_str(), w % 2, h % 2), ("h264", 0, 0));
    let key: [u8; 32] = STANDARD.decode(key).unwrap().try_into().unwrap();
    let host_addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut ph = Phone::new(&key, sid);
    ph.sock.set_read_timeout(Some(Duration::from_millis(20))).unwrap();

    let mut rng = 0x2545F4914F6CDD1Du64;
    let mut drop10 = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng.is_multiple_of(10)
    };
    let (mut partial, mut done) = (BTreeMap::<u32, Partial>::new(), BTreeMap::<u32, (bool, u64, u64)>::new());
    let (mut got_video, mut last_probe, mut last_report) = (false, Instant::now() - Duration::from_secs(1), Instant::now());
    let (mut media, mut dropped, mut idrs, mut audio) = (0u32, 0u32, 0u32, 0u32);
    let (mut decodable, mut need_key, mut next_id) = (Vec::new(), true, 0u32);
    let start = Instant::now();
    let mut first_frame_at = None;
    let mut last_idr = Instant::now();
    let (mut max_seq, mut received) = (0u32, 0u32);
    while decodable.len() < 120 && start.elapsed() < Duration::from_secs(15) {
        if last_probe.elapsed() >= Duration::from_millis(if got_video { 1000 } else { 200 }) {
            ph.probe(host_addr, now_us());
            last_probe = Instant::now();
        }
        if last_report.elapsed() >= Duration::from_millis(100) {
            let mut r = [0u8; 16];
            r[4..6].copy_from_slice(&((dropped * 1000 / media.max(1)) as u16).to_be_bytes());
            r[10..12].copy_from_slice(&1u16.to_be_bytes());
            ph.send(host_addr, Type::Report, &r);
            (media, dropped, last_report) = (0, 0, Instant::now());
        }
        let Some((hd, plain)) = ph.recv() else { continue };
        (max_seq, received) = (max_seq.max(hd.seq), received + 1);
        match hd.typ {
            Type::Audio => {
                assert!(plain.len() > 8);
                audio += 1;
            }
            Type::Video | Type::Fec => {
                got_video = true;
                media += 1;
                if drop10() {
                    dropped += 1;
                    continue;
                }
                if done.contains_key(&hd.frame_id) {
                    continue;
                }
                let f = partial.entry(hd.frame_id).or_insert_with(|| Partial {
                    k: hd.k as usize,
                    p: hd.flags as usize,
                    shards: vec![None; hd.count as usize],
                    first_us: now_us(),
                });
                f.shards[hd.frag as usize] = Some(plain);
                if f.shards.iter().flatten().count() >= f.k {
                    if let Some((fh, data)) = assemble(f.k, f.p, f.shards.clone()) {
                        assert_eq!(data.len(), fh.len as usize);
                        assert!(data.starts_with(&[0, 0, 0, 1]) || data.starts_with(&[0, 0, 1]), "Annex B");
                        done.insert(hd.frame_id, (fh.keyframe, fh.capture_us, f.first_us));
                        partial.remove(&hd.frame_id);
                    }
                }
            }
            _ => {}
        }
        // In-order delivery like the phone: a frame missing for 50 ms (or overtaken) is dropped,
        // and decoding resumes at the next keyframe, which is requested at most every 250 ms.
        while let Some(&(key, cap, at)) = done.get(&next_id) {
            first_frame_at.get_or_insert_with(Instant::now);
            if need_key && !key {
                // waiting for a keyframe: skip
            } else {
                need_key = false;
                decodable.push((next_id, key, at.saturating_sub(cap)));
            }
            next_id += 1;
        }
        let stale = partial.get(&next_id).is_some_and(|f| now_us() - f.first_us > 50_000) || done.keys().next_back().is_some_and(|&l| l > next_id + 2);
        if stale && !done.contains_key(&next_id) {
            partial.remove(&next_id);
            next_id += 1;
            need_key = true;
        }
        if need_key && got_video && last_idr.elapsed() >= Duration::from_millis(250) {
            ph.send(host_addr, Type::Idr, &[]);
            last_idr = Instant::now();
            idrs += 1;
        }
    }
    eprintln!("packets lost on the way (before the simulated drop): {}", (max_seq + 1).saturating_sub(received));
    eprintln!("assembled {} of {} frames through FEC; keyframes at {:?}", done.len(), done.keys().next_back().map_or(0, |l| l + 1), done.iter().filter(|d| d.1 .0).map(|d| *d.0).collect::<Vec<_>>());
    let secs = first_frame_at.map(|t| t.elapsed().as_secs_f64()).unwrap_or(1.0);
    let mut lat: Vec<u64> = decodable.iter().map(|d| d.2).collect();
    lat.sort();
    let p95 = lat.get(lat.len() * 95 / 100).copied().unwrap_or(0);
    eprintln!(
        "loopback: {} decodable frames in {secs:.2}s ({:.1} fps), {idrs} IDR requests, {audio} audio packets, capture→receive p50 {} µs p95 {p95} µs, {w}x{h}",
        decodable.len(),
        decodable.len() as f64 / secs,
        lat.get(lat.len() / 2).copied().unwrap_or(0)
    );
    assert!(decodable.len() >= 120, "only {} frames", decodable.len());
    assert!(decodable[0].1, "decoding starts at a keyframe");
    // Desktop Duplication only produces frames when the screen changes, so a still desktop gives
    // fewer than the display rate: this checks frames keep flowing through FEC, not the PC's activity.
    assert!(done.len() as f64 / secs > 20.0, "frame rate through FEC: {:.1} fps", done.len() as f64 / secs);
    assert!(p95 < 50_000, "capture→receive p95 {p95} µs");
    drop(session);
}

impl Phone {
    fn addr(&self) -> SocketAddr {
        self.sock.local_addr().unwrap()
    }

    /// PROBEs received until 300 ms of silence.
    fn count_probes(&self) -> usize {
        std::iter::from_fn(|| self.recv()).filter(|(h, p)| h.typ == Type::Probe && p[8..] == [0; 8]).count()
    }
}

fn short_punch(sh: &mut Shared) {
    sh.punch_every = Duration::from_millis(50);
    sh.punch_for = Duration::from_millis(400);
}

#[test]
fn punches_candidates_until_the_first_probe() {
    let (b, c) = (Phone::new(&KEY, 42), Phone::new(&KEY, 42));
    let (sh, host_addr, _, t) = host_with(TIMEOUT, short_punch);
    sh.punch([b.addr(), b.addr()]);
    let n = b.count_probes();
    assert!((5..=10).contains(&n), "~400/50 PROBEs in the window, got {n}");
    assert_eq!(b.count_probes(), 0, "stops after the window");

    // stream.cand: a new candidate restarts the window
    sh.punch([c.addr()]);
    std::thread::sleep(Duration::from_millis(120));
    let mut a = Phone::new(&KEY, 42);
    a.probe(host_addr, 5); // the phone got through from another address
    assert!(a.recv().is_some());
    assert_eq!(sh.peer(), Some(a.addr()));
    let n = c.count_probes();
    assert!((1..=4).contains(&n), "punching stops at the first authenticated PROBE, got {n}");
    b.count_probes(); // b was punched again too
    assert_eq!(c.count_probes() + b.count_probes(), 0);
    stop(&sh, t);
}

#[test]
fn punching_keeps_the_stream_alive_past_the_timeout() {
    let b = Phone::new(&KEY, 42);
    let (sh, _, _, t) = host_with(Duration::from_millis(150), |sh| {
        short_punch(sh);
        sh.punch_for = Duration::from_millis(600);
    });
    sh.punch([b.addr()]);
    std::thread::sleep(Duration::from_millis(400));
    assert!(!t.is_finished(), "still punching");
    std::thread::sleep(Duration::from_millis(400));
    assert!(t.is_finished(), "gave up after the window");
    assert!(sh.stopped());
}

#[test]
fn ready_candidates_in_order() {
    let lan = ["192.168.0.126".to_string(), "10.0.0.2".to_string()];
    let pub_ip = Some(IpAddr::from([203, 0, 113, 7]));
    let refl = Some("203.0.113.7:48998".parse().unwrap());
    assert_eq!(host_cands(&lan, STREAM_PORT, pub_ip, refl), ["192.168.0.126:48998", "10.0.0.2:48998", "203.0.113.7:48998"]);
    let refl = Some("198.51.100.4:6100".parse().unwrap());
    assert_eq!(host_cands(&lan[..1], 50000, pub_ip, refl), ["192.168.0.126:50000", "198.51.100.4:6100"], "UPnP maps only 48998");
}

/// Measures the installed package's build profile without capture or a service restart.
#[test]
#[ignore]
fn packetization_throughput_diagnostic() {
    for size in [16_384, 65_536, 196_608] {
        let data=vec![123u8;size];
        let header=FrameHeader {len:size as u32,capture_us:0,keyframe:false,codec:Codec::Hevc};
        let start=Instant::now();
        for i in 0..60 { std::hint::black_box(packetize(i,&header,&data,15)); }
        eprintln!("host packetize bytes={size} mean_ms={:.3}",start.elapsed().as_secs_f64()*1000.0/60.0);
    }
}

/// `stream.size` mid-stream: the video thread reopens the encoder at the new size and announces it
/// with a fresh `stream.ready`, without the session ending.
#[test]
fn resize_mid_stream_announces_the_new_size() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::<(u32, u32)>::new()));
    let log = seen.clone();
    let notify: Notify = Arc::new(move |m| {
        if let ServerMsg::StreamReady { w, h, .. } = m {
            log.lock().unwrap().push((w, h));
        }
    });
    let p = Params { w: 1280, h: 720, fps: 60, codecs: vec!["h264".into()], max_kbps: 10000, audio: false, cands: vec![], public: None, display: String::new(), gate: None };
    let (ready, session) = match StreamSession::start(p, IpAddr::from([127, 0, 0, 1]), notify, Box::new(|_| {})) {
        Ok(r) => r,
        Err(e) => return eprintln!("no stream on this PC: {e:?}"),
    };
    let ServerMsg::StreamReady { w, h, .. } = ready else { panic!("{ready:?}") };
    session.resize(640, 360);
    let start = Instant::now();
    let got = loop {
        let now = seen.lock().unwrap().iter().copied().find(|&(sw, sh)| (sw, sh) != (w, h) && sw <= 640 && sh <= 360);
        if now.is_some() || start.elapsed() > Duration::from_secs(8) {
            break now;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(got.is_some(), "no stream.ready at the new size (started at {w}x{h}, saw {:?})", seen.lock().unwrap());
    assert!(session.alive(), "the session keeps running");
    // and back to the display's native size
    session.resize(0, 0);
    let start = Instant::now();
    let native = loop {
        let now = seen.lock().unwrap().iter().skip(1).any(|&(sw, _)| sw > 640);
        if now || start.elapsed() > Duration::from_secs(8) {
            break now;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(native, "no stream.ready back at native size, saw {:?}", seen.lock().unwrap());
    assert!(session.alive());
}

#[test]
fn source_clock_follows_near_equal_display_rates_without_speeding_up_capture() {
    assert_eq!(stream_interval(60,Some((60000,1001))),Duration::from_nanos(16_683_333));
    assert_eq!(stream_interval(30,Some((60000,1001))),Duration::from_nanos(33_366_666));
    assert_eq!(stream_interval(60,Some((144,1))),Duration::from_nanos(16_666_666));
    assert_eq!(stream_interval(60,None),Duration::from_nanos(16_666_666));
    assert_eq!(stream_interval(60,Some((60,0))),Duration::from_nanos(16_666_666));
}
#[test]
fn source_clock_anchors_to_capture_instead_of_thread_wakeup() {
    let now=Instant::now();let budget=Duration::from_millis(10);
    let first=source_deadline(1_000_000,1_001_000,now,budget).unwrap();
    let later=source_deadline(1_000_000,1_004_000,now+Duration::from_millis(3),budget).unwrap();
    assert_eq!(first,later);
    assert!(source_deadline(1_000_000,1_020_000,now,budget).is_none(),"old still image cannot choose the clock phase");
    assert!(source_deadline(1_020_000,1_000_000,now,budget).is_none(),"future timestamp cannot choose the clock phase");
}
#[test]
fn source_clock_samples_the_new_frame_when_capture_is_faster_than_the_stream() {
    let now=Instant::now();let slot=now+Duration::from_millis(10);
    assert!(!source_in_slot(1_000_000,1_000_000,now,slot),"do not encode the old image early");
    assert!(source_in_slot(1_010_000,1_000_000,now,slot));
    assert!(source_in_slot(1_008_500,1_011_000,now+Duration::from_millis(11),slot),"small compositor variation is allowed");
    assert!(!source_in_slot(1_000_000,1_011_000,now+Duration::from_millis(11),slot));
}
