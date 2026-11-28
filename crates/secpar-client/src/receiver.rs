use secpar_stream::{
    fec,
    frame::{self, FrameHeader},
    Crypt, Dir, Header, Type, SHARD,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const MAX_SHARDS: usize = 4096;
/// Incomplete frames waiting for their missing packets.
const MAX_PENDING: usize = 12;
/// Waiting for a frame's packets before asking for them again: reordering on Wi-Fi.
const NACK_WAIT: Duration = Duration::from_millis(3);
/// Asks per frame: the first NACK, then one more after about a round trip.
const MAX_NACKS: u8 = 2;

pub struct VideoFrame {
    pub id: u32,
    pub header: FrameHeader,
    pub data: Vec<u8>,
}
struct Pending {
    k: usize,
    p: usize,
    shards: Vec<Option<Vec<u8>>>,
    first: Instant,
    nacks: u8,
    nacked_at: Option<Instant>,
}

impl Pending {
    /// Missing fragments of the FEC blocks that can't be rebuilt yet.
    fn missing(&self) -> Vec<u16> {
        let mut out = vec![];
        let mut at = 0;
        for (kb, mb) in fec::layout(self.k, self.p) {
            let block = &self.shards[at..at + kb + mb];
            if block.iter().filter(|s| s.is_some()).count() < kb {
                out.extend(block.iter().enumerate().filter(|(_, s)| s.is_none()).map(|(i, _)| (at + i) as u16));
            }
            at += kb + mb;
        }
        out
    }
}

fn oldest_first(a: &u32, b: &u32) -> std::cmp::Ordering {
    if a == b {
        std::cmp::Ordering::Equal
    } else if newer(*a, *b) {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Less
    }
}

/// Frames from video packets, in order. A frame FEC can't rebuild is asked for again
/// ([`Assembler::nacks`], `Type::Nack`) and later frames wait behind it; only when the repair
/// window passes is it dropped, and then nothing but a keyframe is shown until one arrives.
pub struct Assembler {
    pending: HashMap<u32, Pending>,
    /// Complete frames waiting for an older incomplete one, with their first packet's arrival.
    held: Vec<(VideoFrame, Instant)>,
    /// (capture time on the host, µs; first packet's arrival) of each frame handed out, for the
    /// client's queueing-delay measurement. Drained by the session.
    pub arrivals: Vec<(u64, Instant)>,
    /// Frames ready to hand out, oldest first.
    ready: std::collections::VecDeque<VideoFrame>,
    /// The newest frame handed out or dropped: packets of older frames are late.
    last: Option<u32>,
    /// The newest frame any packet belonged to: an older incomplete frame has a gap.
    newest: Option<u32>,
    need_key: bool,
    pub dropped: u64,
    /// Round-trip time to the host, for when to ask again and when to give up.
    pub rtt: Duration,
}
impl Default for Assembler {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
            held: vec![],
            arrivals: vec![],
            ready: Default::default(),
            last: None,
            newest: None,
            need_key: false,
            dropped: 0,
            rtt: Duration::from_millis(50),
        }
    }
}
/// How long a frame waits for one before it that sent no packet at all, in case it is only late.
const GAP_WAIT: Duration = Duration::from_millis(30);
fn newer(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}
impl Assembler {
    /// How long a frame may wait for lost packets: two round trips and a margin.
    fn window(&self) -> Duration {
        (self.rtt * 2 + Duration::from_millis(40)).clamp(Duration::from_millis(80), Duration::from_millis(300))
    }
    pub fn tick(&mut self, now: Instant) {
        let window = self.window();
        let mut stale: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, f)| now.saturating_duration_since(f.first) > window)
            .map(|(id, _)| *id)
            .collect();
        stale.sort_by(oldest_first);
        for id in stale {
            self.pending.remove(&id);
            self.drop_frame(id);
        }
        self.release(now);
    }
    fn drop_frame(&mut self, id: u32) {
        if self.last.is_none_or(|last| newer(id, last)) {
            self.last = Some(id);
        }
        self.need_key = true;
        self.dropped += 1;
    }
    pub fn needs_keyframe(&self) -> bool {
        self.need_key
    }
    /// The next frame waiting to be handed out (after [`Assembler::push`] or [`Assembler::tick`]).
    pub fn pop(&mut self) -> Option<VideoFrame> {
        self.ready.pop_front()
    }
    /// Lost packets to ask for again now: (frame id, fragment indices).
    pub fn nacks(&mut self, now: Instant) -> Vec<(u32, Vec<u16>)> {
        let again = (self.rtt + self.rtt / 2).max(Duration::from_millis(20));
        let newest = self.newest;
        let mut out = vec![];
        for (id, f) in self.pending.iter_mut() {
            let age = now.saturating_duration_since(f.first);
            let gap = newest.is_some_and(|n| newer(n, *id)) || age > Duration::from_millis(20);
            let due = match f.nacked_at {
                None => gap && age >= NACK_WAIT,
                Some(at) => f.nacks < MAX_NACKS && now.saturating_duration_since(at) >= again,
            };
            if !due {
                continue;
            }
            let missing = f.missing();
            if missing.is_empty() {
                continue;
            }
            f.nacks += 1;
            f.nacked_at = Some(now);
            out.push((*id, missing));
        }
        out
    }
    /// Moves complete frames that no older incomplete frame blocks into `ready`, in order.
    fn release(&mut self, now: Instant) {
        self.held.sort_by(|a, b| oldest_first(&a.0.id, &b.0.id));
        while let Some((first, arrived)) = self.held.first() {
            if self.pending.keys().any(|id| newer(first.id, *id)) {
                break;
            }
            // A frame between the last one handed out and this one sent no packet at all: lost
            // whole (a small frame is one or two packets), or skipped by a host whose sending to
            // this device fell behind. This one would decode against a picture never received and
            // smear until the next key frame. Wait a moment for it, then count it lost.
            let (id, key, arrived) = (first.id, first.header.keyframe, *arrived);
            if let Some(last) = self.last.filter(|last| newer(id, last.wrapping_add(1))) {
                if !self.need_key && !key && now.saturating_duration_since(arrived) < GAP_WAIT {
                    break;
                }
                self.need_key = true;
                self.dropped += u64::from(id.wrapping_sub(last).wrapping_sub(1));
            }
            let (f, arrived) = self.held.remove(0);
            if self.arrivals.len() < 1024 {
                self.arrivals.push((f.header.capture_us, arrived));
            }
            if self.last.is_some_and(|last| !newer(f.id, last)) {
                continue; // an older drop already moved past it
            }
            self.last = Some(f.id);
            if self.need_key && !f.header.keyframe {
                continue;
            }
            self.need_key = false;
            self.ready.push_back(f);
        }
    }
    pub fn push(&mut self, h: Header, data: Vec<u8>, now: Instant) -> Option<VideoFrame> {
        self.tick(now);
        if !matches!(h.typ, Type::Video | Type::Fec)
            || data.len() != SHARD
            || !(5..=50).contains(&h.flags)
            || h.k == 0
            || h.count as usize > MAX_SHARDS
            || h.frag >= h.count
            || self.last.is_some_and(|last| !newer(h.frame_id, last))
            || self.held.iter().any(|(f, _)| f.id == h.frame_id)
        {
            return self.pop();
        }
        let (k, p) = (h.k as usize, h.flags as usize);
        let layout = fec::layout(k, p);
        let count: usize = layout.iter().map(|(k, m)| k + m).sum();
        if count != h.count as usize {
            return self.pop();
        }
        if self.newest.is_none_or(|n| newer(h.frame_id, n)) {
            self.newest = Some(h.frame_id);
        }
        if !self.pending.contains_key(&h.frame_id) && self.pending.len() >= MAX_PENDING {
            let oldest = self.pending.keys().copied().min_by(oldest_first);
            if let Some(id) = oldest {
                self.pending.remove(&id);
                self.drop_frame(id);
                self.release(now);
            }
            if self.last.is_some_and(|last| !newer(h.frame_id, last)) {
                return self.pop();
            }
        }
        let f = self.pending.entry(h.frame_id).or_insert_with(|| Pending {
            k,
            p,
            shards: vec![None; count],
            first: now,
            nacks: 0,
            nacked_at: None,
        });
        if f.k != k || f.p != p || f.shards[h.frag as usize].is_some() {
            return self.pop();
        }
        f.shards[h.frag as usize] = Some(data);
        let mut at = 0;
        for (kb, mb) in layout {
            if f.shards[at..at + kb + mb].iter().filter(|s| s.is_some()).count() < kb {
                return self.pop();
            }
            at += kb + mb;
        }
        let f = self.pending.remove(&h.frame_id)?;
        match frame::assemble(k, p, f.shards) {
            Some((header, data)) => self.held.push((VideoFrame { id: h.frame_id, header, data }, f.first)),
            None => self.drop_frame(h.frame_id),
        }
        self.release(now);
        self.pop()
    }
}

/// How much later than usual video frames arrive: capture (host clock) to first packet (client
/// clock), minus the smallest such gap of the last 5-10 s. The clocks' offset cancels out; a
/// rising value is a queue filling somewhere on the path, before it overflows into loss.
pub struct QueueDelay {
    start: Instant,
    /// Smallest gap in the previous and the current 5 s bucket, µs.
    base: [Option<i64>; 2],
    bucket_at: Instant,
    sum: i64,
    n: i64,
}
impl QueueDelay {
    pub fn new(start: Instant) -> Self {
        Self { start, base: [None, None], bucket_at: start, sum: 0, n: 0 }
    }
    pub fn add(&mut self, capture_us: u64, arrived: Instant) {
        if capture_us == 0 {
            return;
        }
        let gap = arrived.saturating_duration_since(self.start).as_micros() as i64 - capture_us as i64;
        if arrived.saturating_duration_since(self.bucket_at) >= Duration::from_secs(5) {
            self.base = [self.base[1], None];
            self.bucket_at = arrived;
        }
        self.base[1] = Some(self.base[1].map_or(gap, |b| b.min(gap)));
        let base = self.base.iter().flatten().copied().min().unwrap_or(gap);
        self.sum += gap - base;
        self.n += 1;
    }
    /// The average since the last call, ms (0 without frames).
    pub fn take_ms(&mut self) -> u16 {
        let ms = if self.n == 0 { 0 } else { self.sum / self.n / 1000 };
        (self.sum, self.n) = (0, 0);
        ms.clamp(0, u16::MAX as i64) as u16
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    #[test]
    fn a_growing_queue_shows_and_a_steady_offset_does_not() {
        let start = Instant::now();
        let mut q = QueueDelay::new(start);
        // host clock 7 s ahead, 3 ms path, a frame every 16 ms
        for i in 0..60u64 {
            q.add(7_000_000 + i * 16_000, start + Duration::from_micros(i * 16_000 + 3_000));
        }
        assert_eq!(q.take_ms(), 0);
        // the path starts queueing: each frame 2 ms later than the one before
        for i in 60..80u64 {
            q.add(7_000_000 + i * 16_000, start + Duration::from_micros(i * 16_000 + 3_000 + (i - 59) * 2_000));
        }
        assert!(q.take_ms() >= 15);
        assert_eq!(q.take_ms(), 0, "reset after each report");
    }
}

/// Replay window is advanced only after packet authentication. Sessions must rekey before sequence exhaustion.
struct Replay {
    max: Option<u32>,
    seen: Vec<Option<u32>>,
}
impl Default for Replay {
    fn default() -> Self {
        Self {
            max: None,
            seen: vec![None; 4096],
        }
    }
}
impl Replay {
    fn accept(&mut self, seq: u32) -> bool {
        if self.max.is_some_and(|max| seq < max && max - seq >= 4096) {
            return false;
        }
        let index = seq as usize % self.seen.len();
        if self.seen[index] == Some(seq) {
            return false;
        }
        self.seen[index] = Some(seq);
        self.max = Some(self.max.map_or(seq, |max| max.max(seq)));
        true
    }
}

pub struct Receiver {
    crypt: Crypt,
    replay: Replay,
    pub assembler: Assembler,
    next: u32,
    exhausted: bool,
}
impl Receiver {
    pub fn new(key: &[u8; 32], session: u32) -> Self {
        Self {
            crypt: Crypt::new(key, session),
            replay: Replay::default(),
            assembler: Assembler::default(),
            next: 0,
            exhausted: false,
        }
    }
    pub fn open(&mut self, packet: &[u8]) -> Option<(Header, Vec<u8>)> {
        if packet.len() > 1200 {
            return None;
        }
        let (h, p) = self.crypt.open(Dir::HostToPhone, packet)?;
        self.replay.accept(h.seq).then_some((h, p))
    }
    pub fn seal(&mut self, typ: Type, plain: &[u8]) -> Option<Vec<u8>> {
        self.seal_flags(typ, 0, plain)
    }
    pub fn seal_flags(&mut self, typ: Type, flags: u8, plain: &[u8]) -> Option<Vec<u8>> {
        if self.exhausted || plain.len() > secpar_stream::MAX_PLAIN {
            return None;
        }
        let seq = self.next;
        if let Some(n) = self.next.checked_add(1) {
            self.next = n;
        } else {
            self.exhausted = true;
        }
        Some(self.crypt.seal(
            Dir::PhoneToHost,
            &Header {
                typ,
                flags,
                k: 0,
                seq,
                frame_id: 0,
                frag: 0,
                count: 0,
            },
            plain,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packets(id: u32, key: bool) -> Vec<(Header, Vec<u8>)> {
        let data = vec![37; 5000];
        frame::packetize(
            id,
            &FrameHeader {
                len: data.len() as u32,
                capture_us: 42,
                keyframe: key,
                codec: frame::Codec::H264,
            },
            &data,
            30,
        )
    }
    #[test]
    fn recovers_reordered_and_missing_packets_without_duplicate_frames() {
        let mut a = Assembler::default();
        let mut p = packets(1, true);
        let now = Instant::now();
        p.remove(0);
        p.reverse();
        let mut frames = vec![];
        for (h, b) in &p {
            if let Some(f) = a.push(*h, b.clone(), now) {
                frames.push(f);
            }
            assert!(a.push(*h, b.clone(), now).is_none());
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, vec![37; 5000]);
    }
    #[test]
    fn a_frame_that_never_arrived_holds_back_later_ones_until_a_key_frame() {
        let mut a = Assembler::default();
        let now = Instant::now();
        let whole = |a: &mut Assembler, id, key, at| packets(id, key).into_iter().filter_map(|(h, b)| a.push(h, b, at)).map(|f| f.id).collect::<Vec<_>>();
        assert_eq!(whole(&mut a, 1, true, now), vec![1]);
        // Frame 2 sends nothing; frame 3 waits for it a moment, then is held back.
        assert!(whole(&mut a, 3, false, now).is_empty(), "frame 3 must wait for frame 2");
        assert!(!a.needs_keyframe());
        a.tick(now + GAP_WAIT + Duration::from_millis(1));
        assert!(a.pop().is_none(), "frame 3 would decode into smears");
        assert!(a.needs_keyframe());
        assert!(whole(&mut a, 4, false, now + Duration::from_millis(40)).is_empty());
        assert_eq!(whole(&mut a, 5, true, now + Duration::from_millis(50)), vec![5]);
        assert!(!a.needs_keyframe());
        // A late frame that does arrive within the wait is handed out in order.
        assert!(whole(&mut a, 7, false, now + Duration::from_millis(60)).is_empty());
        assert_eq!(whole(&mut a, 6, false, now + Duration::from_millis(70)), vec![6, 7]);
    }
    #[test]
    fn timeout_requires_keyframe_and_bounds_pending_memory() {
        let mut a = Assembler::default();
        let now = Instant::now();
        for id in 0..20 {
            let (h, b) = packets(id, false).remove(0);
            a.push(h, b, now);
            assert!(a.pending.len() <= MAX_PENDING);
        }
        a.tick(now + Duration::from_millis(301));
        assert!(a.pending.is_empty());
        assert!(a.needs_keyframe());
        for (h, b) in packets(21, false) {
            assert!(a.push(h, b, now).is_none());
        }
        let mut got = false;
        for (h, b) in packets(22, true) {
            got |= a.push(h, b, now).is_some();
        }
        assert!(got);
        assert!(!a.needs_keyframe());
    }
    #[test]
    fn lost_packets_are_asked_for_and_later_frames_wait_in_order() {
        let mut a = Assembler::default();
        let now = Instant::now();
        let mut first = packets(1, true);
        // Frame 1 loses more than FEC can rebuild: every data packet of its first block.
        let lost: Vec<_> = first.iter().take_while(|(h, _)| h.typ == Type::Video).map(|(h, b)| (*h, b.clone())).collect();
        first.retain(|(h, _)| !lost.iter().any(|(l, _)| l.frag == h.frag));
        for (h, b) in first {
            assert!(a.push(h, b, now).is_none());
        }
        // Frame 2 arrives whole and waits behind frame 1.
        for (h, b) in packets(2, false) {
            assert!(a.push(h, b, now).is_none(), "frame 2 must wait for frame 1");
        }
        let asked = a.nacks(now + Duration::from_millis(5));
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0, 1);
        assert!(lost.iter().all(|(h, _)| asked[0].1.contains(&h.frag)));
        assert!(a.nacks(now + Duration::from_millis(6)).is_empty(), "not again before a round trip");
        // The resent packets complete frame 1; frame 2 follows it.
        let mut got = vec![];
        for (h, b) in lost {
            if let Some(f) = a.push(h, b, now + Duration::from_millis(30)) {
                got.push(f.id);
            }
        }
        while let Some(f) = a.pop() {
            got.push(f.id);
        }
        assert_eq!(got, vec![1, 2]);
        assert!(!a.needs_keyframe());
    }
    #[test]
    fn rejects_malformed_counts_before_allocating() {
        let mut a = Assembler::default();
        let (mut h, b) = packets(1, true).remove(0);
        h.k = u16::MAX;
        h.count = u16::MAX;
        assert!(a.push(h, b, Instant::now()).is_none());
        assert!(a.pending.is_empty());
    }
    #[test]
    fn authentication_precedes_replay_state() {
        let c = Crypt::new(&[7; 32], 123);
        let mut r = Receiver::new(&[7; 32], 123);
        let h = Header {
            typ: Type::Probe,
            flags: 0,
            k: 0,
            seq: 1,
            frame_id: 0,
            frag: 0,
            count: 0,
        };
        let p = c.seal(Dir::HostToPhone, &h, &[0; 16]);
        let mut bad = p.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(r.open(&bad).is_none());
        assert!(r.open(&p).is_some());
        assert!(r.open(&p).is_none());
        assert!(r
            .open(&c.seal(Dir::PhoneToHost, &Header { seq: 2, ..h }, &[]))
            .is_none());
        assert!(r
            .open(&c.seal(Dir::HostToPhone, &Header { seq: 2, ..h }, &[]))
            .is_some());
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    #[test]
    fn tolerates_large_frame_reordering_but_rejects_duplicates_and_stale_packets() {
        let mut replay = Replay::default();
        assert!(replay.accept(4000));
        assert!(replay.accept(1));
        assert!(!replay.accept(1));
        assert!(replay.accept(9000));
        assert!(!replay.accept(4000));
        assert!(replay.accept(8999));
        assert!(!replay.accept(8999));
    }
}
