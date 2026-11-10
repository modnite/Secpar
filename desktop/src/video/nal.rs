//! Just enough bitstream reading to tell whether an access unit can start decoding.
use secpar_stream::frame::Codec;

/// Header byte(s) of every Annex B NAL unit (start code 00 00 01 or 00 00 00 01).
fn headers(data: &[u8]) -> impl Iterator<Item = u8> + '_ {
    data.windows(4).filter(|w| w[..3] == [0, 0, 1]).map(|w| w[3])
}

/// NAL unit types in order: H.264 uses 5 bits, HEVC 6 bits.
pub fn nal_types(codec: Codec, data: &[u8]) -> Vec<u8> {
    match codec {
        Codec::H264 => headers(data).map(|h| h & 0x1F).collect(),
        Codec::Hevc => headers(data).map(|h| (h >> 1) & 0x3F).collect(),
        Codec::Av1 => Vec::new(),
    }
}

/// True when the unit carries the parameter sets a decoder needs to start (SPS and PPS for
/// H.264, VPS/SPS/PPS for HEVC). AV1 carries its sequence header in-band, so it always passes.
pub fn has_parameter_sets(codec: Codec, data: &[u8]) -> bool {
    let types = nal_types(codec, data);
    match codec {
        Codec::H264 => types.contains(&7) && types.contains(&8),
        Codec::Hevc => types.contains(&33) && types.contains(&34),
        Codec::Av1 => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_h264_unit_types_with_both_start_code_lengths() {
        let data = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4];
        assert_eq!(nal_types(Codec::H264, &data), [7, 8, 5]);
        assert!(has_parameter_sets(Codec::H264, &data));
    }
    #[test]
    fn a_keyframe_without_parameter_sets_cannot_start_decoding() {
        let data = [0, 0, 0, 1, 0x65, 1, 2, 3];
        assert!(!has_parameter_sets(Codec::H264, &data));
        assert!(!has_parameter_sets(Codec::H264, &[0, 0, 0, 1, 0x67, 9, 9]));
        assert!(!has_parameter_sets(Codec::H264, &[]));
    }
    #[test]
    fn reads_hevc_unit_types() {
        let data = [0, 0, 1, 0x40, 1, 0, 0, 1, 0x42, 1, 0, 0, 1, 0x44, 1, 0, 0, 1, 0x26, 1];
        assert_eq!(nal_types(Codec::Hevc, &data), [32, 33, 34, 19]);
        assert!(has_parameter_sets(Codec::Hevc, &data));
    }
    #[test]
    fn truncated_start_codes_are_ignored() {
        assert!(nal_types(Codec::H264, &[0, 0, 1]).is_empty());
        assert!(nal_types(Codec::H264, &[0, 0]).is_empty());
    }
}
