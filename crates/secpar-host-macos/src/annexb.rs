//! VideoToolbox emits length-prefixed (AVCC) NAL units; the PRS stream carries Annex-B.
const START: [u8; 4] = [0, 0, 0, 1];

/// `nal_len_size` is the prefix width reported by the format description (4 for VideoToolbox).
/// `parameter_sets` (SPS, PPS) are prepended on keyframes so every keyframe is decodable alone.
pub fn avcc_to_annexb(data: &[u8], nal_len_size: usize, parameter_sets: &[Vec<u8>], key: bool) -> Option<Vec<u8>> {
    if !(1..=4).contains(&nal_len_size) {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() + 64);
    if key {
        for set in parameter_sets {
            out.extend_from_slice(&START);
            out.extend_from_slice(set);
        }
    }
    let mut at = 0;
    while at < data.len() {
        let prefix = data.get(at..at + nal_len_size)?;
        let len = prefix.iter().fold(0usize, |n, b| n << 8 | *b as usize);
        at += nal_len_size;
        let nal = data.get(at..at.checked_add(len)?)?;
        if !nal.is_empty() {
            out.extend_from_slice(&START);
            out.extend_from_slice(nal);
        }
        at += len;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converts_prefixes_and_adds_parameter_sets_only_on_keyframes() {
        let avcc = [0, 0, 0, 2, 0x65, 0xaa, 0, 0, 0, 1, 0x41];
        let sets = vec![vec![0x67, 1], vec![0x68, 2]];
        assert_eq!(
            avcc_to_annexb(&avcc, 4, &sets, true).unwrap(),
            [0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x68, 2, 0, 0, 0, 1, 0x65, 0xaa, 0, 0, 0, 1, 0x41]
        );
        assert_eq!(avcc_to_annexb(&avcc, 4, &sets, false).unwrap(), [0, 0, 0, 1, 0x65, 0xaa, 0, 0, 0, 1, 0x41]);
    }
    #[test]
    fn truncated_or_oversized_lengths_are_rejected_not_read_past_the_end() {
        assert!(avcc_to_annexb(&[0, 0, 0, 9, 1], 4, &[], false).is_none());
        assert!(avcc_to_annexb(&[0, 0], 4, &[], false).is_none());
        assert!(avcc_to_annexb(&[0xff, 0xff, 0xff, 0xff, 1], 4, &[], false).is_none());
        assert!(avcc_to_annexb(&[1], 5, &[], false).is_none());
    }
}
