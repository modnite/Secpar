use secpar_stream::{
    fec,
    frame::{self, FrameHeader},
    Crypt, Dir, Header, Type, SHARD,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const MAX_SHARDS: usize = 4096;
const MAX_PENDING: usize = 4;
const FRAME_TIMEOUT: Duration = Duration::from_millis(60);

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
}

#[derive(Default)]
pub struct Assembler {
    pending: HashMap<u32, Pending>,
    last: Option<u32>,
    need_key: bool,
    pub dropped: u64,
}
fn newer(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}
impl Assembler {
    pub fn tick(&mut self, now: Instant) {
        let stale: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, f)| now.saturating_duration_since(f.first) > FRAME_TIMEOUT)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.pending.remove(&id);
            self.drop_frame(id);
        }
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
    pub fn push(&mut self, h: Header, data: Vec<u8>, now: Instant) -> Option<VideoFrame> {
        self.tick(now);
        if !matches!(h.typ, Type::Video | Type::Fec)
            || data.len() != SHARD
            || !(5..=50).contains(&h.flags)
            || h.k == 0
            || h.count as usize > MAX_SHARDS
            || h.frag >= h.count
            || self.last.is_some_and(|last| !newer(h.frame_id, last))
        {
            return None;
        }
        let (k, p) = (h.k as usize, h.flags as usize);
        let layout = fec::layout(k, p);
        let count: usize = layout.iter().map(|(k, m)| k + m).sum();
        if count != h.count as usize {
            return None;
        }
        if !self.pending.contains_key(&h.frame_id) && self.pending.len() >= MAX_PENDING {
            let id = *self.pending.iter().min_by_key(|(_, f)| f.first)?.0;
            self.pending.remove(&id);
            self.drop_frame(id);
            if self.last.is_some_and(|last| !newer(h.frame_id, last)) {
                return None;
            }
        }
        let f = self.pending.entry(h.frame_id).or_insert_with(|| Pending {
            k,
            p,
            shards: vec![None; count],
            first: now,
        });
        if f.k != k || f.p != p || f.shards[h.frag as usize].is_some() {
            return None;
        }
        f.shards[h.frag as usize] = Some(data);
        let mut at = 0;
        for (kb, mb) in layout {
            if f.shards[at..at + kb + mb]
                .iter()
                .filter(|s| s.is_some())
                .count()
                < kb
            {
                return None;
            }
            at += kb + mb;
        }
        let f = self.pending.remove(&h.frame_id)?;
        let older: Vec<_> = self
            .pending
            .keys()
            .filter(|id| newer(h.frame_id, **id))
            .copied()
            .collect();
        for id in older {
            self.pending.remove(&id);
            self.drop_frame(id);
        }
        self.last = Some(h.frame_id);
        let Some((header, data)) = frame::assemble(k, p, f.shards) else {
            self.drop_frame(h.frame_id);
            return None;
        };
        if self.need_key && !header.keyframe {
            return None;
        }
        self.need_key = false;
        Some(VideoFrame {
            id: h.frame_id,
            header,
            data,
        })
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
                flags: 0,
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
    fn timeout_requires_keyframe_and_bounds_pending_memory() {
        let mut a = Assembler::default();
        let now = Instant::now();
        for id in 0..20 {
            let (h, b) = packets(id, false).remove(0);
            a.push(h, b, now);
            assert!(a.pending.len() <= MAX_PENDING);
        }
        a.tick(now + Duration::from_millis(61));
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
