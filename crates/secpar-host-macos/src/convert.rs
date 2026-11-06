//! Pure helpers shared by the native capture code, tested without a Mac.

/// Output size: the display scaled down to fit the request, aspect kept, even dimensions
/// (H.264 4:2:0 needs them). A zero request means the display's own size.
pub fn fit_size(display_w: u32, display_h: u32, want_w: u32, want_h: u32) -> (u32, u32) {
    let (mut w, mut h) = (display_w.max(2) as u64, display_h.max(2) as u64);
    if want_w > 0 && want_h > 0 && (w > want_w as u64 || h > want_h as u64) {
        // scale = min(want_w / w, want_h / h), done in integers
        if want_w as u64 * h <= want_h as u64 * w {
            h = (h * want_w as u64 / w).max(2);
            w = want_w as u64;
        } else {
            w = (w * want_h as u64 / h).max(2);
            h = want_h as u64;
        }
    }
    ((w as u32) & !1, (h as u32) & !1)
}
/// Little-endian f32 bytes, either one interleaved stereo buffer or two planar mono buffers,
/// to interleaved stereo samples. Mismatched or odd data yields None.
pub fn interleave_stereo(buffers: &[&[u8]]) -> Option<Vec<f32>> {
    let floats = |b: &[u8]| -> Option<Vec<f32>> {
        (b.len() % 4 == 0).then(|| b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    };
    match buffers {
        [one] => {
            let s = floats(one)?;
            (s.len() % 2 == 0).then_some(s)
        }
        [left, right] => {
            let (l, r) = (floats(left)?, floats(right)?);
            (l.len() == r.len()).then(|| l.into_iter().zip(r).flat_map(|(a, b)| [a, b]).collect())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_fits_keeps_aspect_and_is_even() {
        assert_eq!(fit_size(1920, 1080, 0, 0), (1920, 1080));
        assert_eq!(fit_size(1920, 1080, 1280, 720), (1280, 720));
        assert_eq!(fit_size(1920, 1080, 1000, 1000), (1000, 562));
        assert_eq!(fit_size(1440, 900, 3840, 2160), (1440, 900));
        assert_eq!(fit_size(1001, 701, 0, 0), (1000, 700));
    }
    #[test]
    fn planar_and_interleaved_audio_become_the_same_stereo_stream() {
        let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let (l, r, both) = (f(&[1.0, 2.0]), f(&[3.0, 4.0]), f(&[1.0, 3.0, 2.0, 4.0]));
        let planar = interleave_stereo(&[&l, &r]).unwrap();
        assert_eq!(planar, [1.0, 3.0, 2.0, 4.0]);
        assert_eq!(interleave_stereo(&[&both]).unwrap(), planar);
        assert!(interleave_stereo(&[&l, &f(&[1.0])]).is_none());
        assert!(interleave_stereo(&[&[0u8; 3]]).is_none());
        assert!(interleave_stereo(&[]).is_none());
    }
}
