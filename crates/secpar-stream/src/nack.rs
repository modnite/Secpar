//! Asking for lost video packets again (`Type::Nack`, client → host).
//!
//! Payload: the frame id (u32), then the fragment indices (u16 each) the client is missing.
//! Hosts keep the sealed packets of their last few frames in a [`History`] and send the asked ones
//! again unchanged: a packet opens the same way twice, and the assembler ignores pieces it has.

use std::collections::VecDeque;

/// Fragment indices one NACK may carry (fits one packet's plaintext).
pub const MAX_FRAGS: usize = 400;
/// Frames a host keeps for resending: about 400 ms at 60 fps.
pub const HISTORY_FRAMES: usize = 24;

pub fn encode(frame_id: u32, frags: &[u16]) -> Vec<u8> {
    let mut b = Vec::with_capacity(4 + 2 * frags.len().min(MAX_FRAGS));
    b.extend_from_slice(&frame_id.to_be_bytes());
    for f in frags.iter().take(MAX_FRAGS) {
        b.extend_from_slice(&f.to_be_bytes());
    }
    b
}

pub fn decode(b: &[u8]) -> Option<(u32, Vec<u16>)> {
    if b.len() < 4 || (b.len() - 4) % 2 != 0 {
        return None;
    }
    let id = u32::from_be_bytes(b[..4].try_into().ok()?);
    let frags = b[4..].chunks_exact(2).take(MAX_FRAGS).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    Some((id, frags))
}

/// The sealed packets of the last [`HISTORY_FRAMES`] frames, by frame id and fragment index.
#[derive(Default)]
pub struct History {
    frames: VecDeque<(u32, Vec<Option<Vec<u8>>>)>,
}

impl History {
    /// Records one sent video or parity packet (`count` packets in its frame).
    pub fn record(&mut self, frame_id: u32, frag: u16, count: u16, sealed: &[u8]) {
        let i = match self.frames.iter().rposition(|(id, _)| *id == frame_id) {
            Some(i) => i,
            None => {
                if self.frames.len() >= HISTORY_FRAMES {
                    self.frames.pop_front();
                }
                self.frames.push_back((frame_id, vec![None; count as usize]));
                self.frames.len() - 1
            }
        };
        if let Some(slot) = self.frames[i].1.get_mut(frag as usize) {
            *slot = Some(sealed.to_vec());
        }
    }

    /// The packets to send again; unknown frames and fragments are skipped.
    pub fn take(&self, frame_id: u32, frags: &[u16]) -> Vec<Vec<u8>> {
        let Some((_, pkts)) = self.frames.iter().find(|(id, _)| *id == frame_id) else { return vec![] };
        frags.iter().filter_map(|f| pkts.get(*f as usize).cloned().flatten()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bad_payloads() {
        assert_eq!(decode(&encode(7, &[1, 5, 9])), Some((7, vec![1, 5, 9])));
        assert_eq!(decode(&encode(3, &[])), Some((3, vec![])));
        assert_eq!(decode(&[0, 0, 0]), None);
        assert_eq!(decode(&[0, 0, 0, 1, 2]), None);
    }

    #[test]
    fn history_keeps_recent_frames_only() {
        let mut h = History::default();
        h.record(1, 0, 3, b"a");
        h.record(1, 2, 3, b"c");
        assert_eq!(h.take(1, &[0, 1, 2, 9]), vec![b"a".to_vec(), b"c".to_vec()]);
        for id in 2..2 + HISTORY_FRAMES as u32 {
            h.record(id, 0, 1, b"x");
        }
        assert!(h.take(1, &[0]).is_empty(), "frame 1 aged out");
        assert_eq!(h.take(2 + HISTORY_FRAMES as u32 - 1, &[0]), vec![b"x".to_vec()]);
    }
}
