//! Frame PCM against capture timestamps, preserving real pauses in the host clock.
use std::collections::VecDeque;
pub const RATE: u64 = 48_000;
pub const FRAME: usize = 480;
pub const MAX_TIMESTAMP_SKEW_US: u64 = 2_000;

#[derive(Default)]
pub struct AudioFrames {
    pcm: VecDeque<f32>,
    first_us: u64,
}
impl AudioFrames {
    pub fn push(&mut self, samples: &[f32], capture_us: u64) {
        assert_eq!(samples.len() % 2, 0, "Interleaved stereo PCM required");
        if samples.is_empty() { return; }
        let expected_us = self.first_us.saturating_add(self.pcm.len() as u64 / 2 * 1_000_000 / RATE);
        if self.pcm.is_empty() || expected_us.abs_diff(capture_us) > MAX_TIMESTAMP_SKEW_US {
            // Joining a partial old frame to resumed PCM would retain the old clock
            // forever. Drop only that incomplete frame and anchor to fresh capture.
            self.pcm.clear();
            self.first_us = capture_us;
        }
        self.pcm.extend(samples.iter().copied());
    }
    pub fn next_frame(&mut self) -> Option<(Vec<f32>, u64)> {
        if self.pcm.len() < FRAME * 2 { return None; }
        let capture_us = self.first_us;
        self.first_us += 10_000;
        Some((self.pcm.drain(..FRAME * 2).collect(), capture_us))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_capture_batches_keep_all_samples_and_contiguous_timestamps() {
        let mut frames = AudioFrames::default();
        frames.push(&vec![0.1; 1200 * 2], 1_000_000);
        assert_eq!(frames.next_frame().unwrap().1, 1_000_000);
        assert_eq!(frames.next_frame().unwrap().1, 1_010_000);
        assert!(frames.next_frame().is_none());
        frames.push(&vec![0.2; 240 * 2], 1_025_000);
        let (pcm, t) = frames.next_frame().unwrap();
        assert_eq!(t, 1_020_000);
        assert!(pcm[..480].iter().all(|n| *n == 0.1));
        assert!(pcm[480..].iter().all(|n| *n == 0.2));
    }
    #[test]
    fn repeated_pauses_and_capture_drops_never_reuse_old_pcm_or_timestamps() {
        let mut frames = AudioFrames::default();
        for t in [1_000_000, 6_000_000, 6_500_000, 12_000_000] {
            frames.push(&vec![t as f32; 1200 * 2], t);
            for offset in [0, 10_000] {
                let (pcm, capture) = frames.next_frame().unwrap();
                assert_eq!(capture, t + offset);
                assert!(pcm.iter().all(|n| *n == t as f32));
            }
            assert!(frames.next_frame().is_none());
        }
    }
    #[test]
    fn small_clock_jitter_preserves_the_partial_frame_but_backward_jumps_rebase() {
        let mut frames = AudioFrames::default();
        frames.push(&vec![0.1; 240 * 2], 1_000_000);
        frames.push(&vec![0.2; 240 * 2], 1_005_500);
        assert_eq!(frames.next_frame().unwrap().1, 1_000_000);
        frames.push(&vec![0.3; 240 * 2], 2_000_000);
        frames.push(&vec![0.4; 480 * 2], 1_500_000);
        let (pcm, t) = frames.next_frame().unwrap();
        assert_eq!(t, 1_500_000);
        assert!(pcm.iter().all(|n| *n == 0.4));
    }
}
