//! A decoded frame kept as 8-bit NV12 (full-resolution luma, interleaved half-resolution
//! chroma). The graphics card converts it to RGB when drawing, so the CPU never
//! touches a 1080p RGB image per frame.
use eframe::egui::ColorImage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
}

#[derive(Clone)]
pub struct Picture {
    pub size: [usize; 2],
    /// `size[0] * size[1]` bytes.
    pub y: Vec<u8>,
    /// `ceil(w/2) * 2 * ceil(h/2)` bytes, U then V per chroma sample.
    pub uv: Vec<u8>,
}

fn chroma(size: [usize; 2]) -> [usize; 2] {
    [size[0].div_ceil(2), size[1].div_ceil(2)]
}

fn valid(w: usize, h: usize) -> bool {
    w > 0 && h > 0 && w <= 8192 && h <= 8192 && w * h <= 4096 * 2160
}

impl Picture {
    /// Rows may be padded: pass the stride of each plane in bytes.
    pub fn from_i420(
        size: [usize; 2],
        y: (&[u8], usize),
        u: (&[u8], usize),
        v: (&[u8], usize),
    ) -> Option<Self> {
        let [w, h] = size;
        let [cw, ch] = chroma(size);
        if !valid(w, h) || y.1 < w || u.1 < cw || v.1 < cw {
            return None;
        }
        if y.0.len() < y.1 * (h - 1) + w || u.0.len() < u.1 * (ch - 1) + cw || v.0.len() < v.1 * (ch - 1) + cw {
            return None;
        }
        let mut luma = Vec::with_capacity(w * h);
        for row in 0..h {
            luma.extend_from_slice(&y.0[row * y.1..row * y.1 + w]);
        }
        let mut uv = Vec::with_capacity(cw * ch * 2);
        for row in 0..ch {
            let (ur, vr) = (&u.0[row * u.1..row * u.1 + cw], &v.0[row * v.1..row * v.1 + cw]);
            for x in 0..cw {
                uv.push(ur[x]);
                uv.push(vr[x]);
            }
        }
        Some(Self { size, y: luma, uv })
    }

    pub fn from_nv12(size: [usize; 2], y: (&[u8], usize), uv: (&[u8], usize)) -> Option<Self> {
        let [w, h] = size;
        let [cw, ch] = chroma(size);
        if !valid(w, h) || y.1 < w || uv.1 < cw * 2 {
            return None;
        }
        if y.0.len() < y.1 * (h - 1) + w || uv.0.len() < uv.1 * (ch - 1) + cw * 2 {
            return None;
        }
        let mut luma = Vec::with_capacity(w * h);
        for row in 0..h {
            luma.extend_from_slice(&y.0[row * y.1..row * y.1 + w]);
        }
        let mut chroma_plane = Vec::with_capacity(cw * ch * 2);
        for row in 0..ch {
            chroma_plane.extend_from_slice(&uv.0[row * uv.1..row * uv.1 + cw * 2]);
        }
        Some(Self { size, y: luma, uv: chroma_plane })
    }

    /// Review-only: eight limited-range BT.709 colour bars for checking the shader.
    #[cfg(feature = "ui-review")]
    pub fn bars(w: usize, h: usize) -> Self {
        const BARS: [[f32; 3]; 8] = [
            [1., 1., 1.], [1., 1., 0.], [0., 1., 1.], [0., 1., 0.],
            [1., 0., 1.], [1., 0., 0.], [0., 0., 1.], [0., 0., 0.],
        ];
        let at = |x: usize| BARS[(x * 8 / w).min(7)];
        let yuv = |[r, g, b]: [f32; 3]| {
            (
                16. + 46.559 * r + 156.629 * g + 15.812 * b,
                128. - 25.664 * r - 86.336 * g + 112. * b,
                128. + 112. * r - 101.730 * g - 10.270 * b,
            )
        };
        let y = (0..w * h).map(|i| yuv(at(i % w)).0.round() as u8).collect();
        let mut uv = Vec::new();
        for _ in 0..h.div_ceil(2) {
            for cx in 0..w.div_ceil(2) {
                let (_, u, v) = yuv(at(cx * 2));
                uv.push(u.round() as u8);
                uv.push(v.round() as u8);
            }
        }
        Self { size: [w, h], y, uv }
    }

    pub fn chroma_size(&self) -> [usize; 2] {
        chroma(self.size)
    }

    /// Limited-range YCbCr to 8-bit RGB. Used by tests and the no-GPU fallback.
    pub fn rgb_at(&self, x: usize, y: usize, matrix: Matrix) -> [u8; 3] {
        let [cw, _] = self.chroma_size();
        let luma = self.y[y * self.size[0] + x] as f32;
        let at = ((y / 2) * cw + x / 2) * 2;
        let (u, v) = (self.uv[at] as f32 - 128.0, self.uv[at + 1] as f32 - 128.0);
        let l = 1.164_383 * (luma - 16.0);
        let (r, g, b) = match matrix {
            Matrix::Bt709 => (l + 1.792_741 * v, l - 0.213_249 * u - 0.532_909 * v, l + 2.112_402 * u),
            Matrix::Bt601 => (l + 1.596_027 * v, l - 0.391_762 * u - 0.812_968 * v, l + 2.017_232 * u),
        };
        [r, g, b].map(|c| c.round().clamp(0.0, 255.0) as u8)
    }

    pub fn to_color_image(&self, matrix: Matrix) -> ColorImage {
        let [w, h] = self.size;
        let mut rgb = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                rgb.extend_from_slice(&self.rgb_at(x, y, matrix));
            }
        }
        ColorImage::from_rgb(self.size, &rgb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_i420_rows_are_packed_and_chroma_is_interleaved() {
        let y: Vec<u8> = (0..4 * 2).map(|i| (i % 4) as u8 + 100).chain(0..0).collect();
        // 4x2 luma with stride 4, chroma 2x1 with stride 3 (one padding byte).
        let picture = Picture::from_i420([4, 2], (&y, 4), (&[10, 11, 99], 3), (&[20, 21, 99], 3)).unwrap();
        assert_eq!(picture.y.len(), 8);
        assert_eq!(picture.uv, vec![10, 20, 11, 21]);
    }

    #[test]
    fn short_or_oversized_planes_are_refused() {
        assert!(Picture::from_i420([4, 2], (&[0; 7], 4), (&[0; 2], 2), (&[0; 2], 2)).is_none());
        assert!(Picture::from_i420([0, 2], (&[], 0), (&[], 0), (&[], 0)).is_none());
        assert!(Picture::from_nv12([9000, 2], (&[], 9000), (&[], 9000)).is_none());
    }

    #[test]
    fn matrices_differ_and_neutral_grey_is_exact() {
        let grey = Picture::from_nv12([2, 2], (&[126; 4], 2), (&[128, 128], 2)).unwrap();
        for matrix in [Matrix::Bt601, Matrix::Bt709] {
            assert_eq!(grey.rgb_at(0, 0, matrix), [128, 128, 128]);
        }
        let red = Picture::from_nv12([2, 2], (&[81; 4], 2), (&[90, 240], 2)).unwrap();
        assert_ne!(red.rgb_at(0, 0, Matrix::Bt601), red.rgb_at(0, 0, Matrix::Bt709));
    }
}
