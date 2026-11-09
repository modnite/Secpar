//! Pointer geometry for the portal session, kept free of D-Bus so it can be tested anywhere.
//!
//! Three coordinate spaces meet here:
//! - normalised: 0..1 across the picture, what a touch client sends in `mouse.pos`;
//! - logical: the compositor's own pixels, what the portal takes for absolute and relative motion
//!   (the size the portal reports for the monitor, which is not the frame size under HiDPI scaling);
//! - stream: pixels of the encoded picture, what the client uses to place the cursor it draws.

/// How the host pointer reaches the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorMode {
    /// The compositor paints the real pointer into every frame. It is always the true pointer at the
    /// true position and shape, so the client must not draw one of its own.
    Embedded,
    /// The frames carry no pointer. The client draws one at the position the host tracks.
    Hidden,
}

/// Embedded when the compositor offers it. `forced` is the `SECPAR_CURSOR` override
/// (`hidden` or `embedded`), for diagnosing a compositor.
pub fn pick_cursor_mode(embedded_offered: bool, hidden_offered: bool, forced: Option<&str>) -> CursorMode {
    match forced.map(str::trim) {
        Some(s) if s.eq_ignore_ascii_case("hidden") && hidden_offered => CursorMode::Hidden,
        Some(s) if s.eq_ignore_ascii_case("embedded") && embedded_offered => CursorMode::Embedded,
        _ if embedded_offered => CursorMode::Embedded,
        _ => CursorMode::Hidden,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    pub logical: (u32, u32),
    pub stream: (u32, u32),
}

fn last(n: u32) -> f64 {
    n.saturating_sub(1) as f64
}

/// A normalised picture position in compositor pixels, for absolute portal motion.
pub fn absolute(nx: f64, ny: f64, logical: (u32, u32)) -> (f64, f64) {
    let unit = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    (unit(nx) * last(logical.0), unit(ny) * last(logical.1))
}

/// A compositor position in stream pixels, the unit of a `cursor` packet.
pub fn to_stream(pos: (f64, f64), space: &Space) -> (i32, i32) {
    let axis = |v: f64, logical: u32, stream: u32| {
        let span = last(logical);
        if span <= 0.0 {
            0
        } else {
            (v.clamp(0.0, span) / span * last(stream)).round() as i32
        }
    };
    (axis(pos.0, space.logical.0, space.stream.0), axis(pos.1, space.logical.1, space.stream.1))
}

/// Where the pointer is, in compositor pixels, as far as this host can know without the compositor
/// reporting it. Only used when the frames do not carry the pointer.
#[derive(Debug)]
pub struct Tracker {
    logical: (u32, u32),
    pos: (f64, f64),
    /// Whether `pos` is the compositor's real pointer position. False until an absolute move landed.
    known: bool,
    moved: bool,
}

impl Tracker {
    pub fn new(logical: (u32, u32)) -> Self {
        Self { logical, pos: (0.0, 0.0), known: false, moved: false }
    }
    pub fn center(&self) -> (f64, f64) {
        (last(self.logical.0) / 2.0, last(self.logical.1) / 2.0)
    }
    /// A relative move can only be followed from a known place. Until then the caller moves the real
    /// pointer to the centre first and tells us with `set`.
    pub fn known(&self) -> bool {
        self.known
    }
    /// The real pointer was put at `pos` by an absolute move.
    pub fn set(&mut self, pos: (f64, f64)) {
        self.pos = (pos.0.clamp(0.0, last(self.logical.0)), pos.1.clamp(0.0, last(self.logical.1)));
        self.known = true;
        self.moved = true;
    }
    /// A relative move, clamped to the monitor like the compositor does.
    pub fn relative(&mut self, dx: i32, dy: i32) {
        self.pos = (
            (self.pos.0 + dx as f64).clamp(0.0, last(self.logical.0)),
            (self.pos.1 + dy as f64).clamp(0.0, last(self.logical.1)),
        );
        self.moved = true;
    }
    /// The position once after it changed.
    pub fn take(&mut self) -> Option<(f64, f64)> {
        std::mem::take(&mut self.moved).then_some(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: [(u32, u32); 6] = [(1366, 768), (1920, 1080), (2560, 1440), (1093, 614), (800, 600), (3840, 2160)];

    #[test]
    fn embedded_wins_when_offered() {
        assert_eq!(pick_cursor_mode(true, true, None), CursorMode::Embedded);
        assert_eq!(pick_cursor_mode(true, false, None), CursorMode::Embedded);
        assert_eq!(pick_cursor_mode(false, true, None), CursorMode::Hidden);
        assert_eq!(pick_cursor_mode(false, false, None), CursorMode::Hidden);
    }

    #[test]
    fn override_is_honoured_only_when_the_compositor_offers_it() {
        assert_eq!(pick_cursor_mode(true, true, Some("hidden")), CursorMode::Hidden);
        assert_eq!(pick_cursor_mode(true, true, Some(" Hidden ")), CursorMode::Hidden);
        assert_eq!(pick_cursor_mode(true, false, Some("hidden")), CursorMode::Embedded);
        assert_eq!(pick_cursor_mode(false, true, Some("embedded")), CursorMode::Hidden);
        assert_eq!(pick_cursor_mode(true, true, Some("nonsense")), CursorMode::Embedded);
    }

    #[test]
    fn absolute_hits_the_corners_and_the_middle() {
        for (w, h) in SIZES {
            assert_eq!(absolute(0.0, 0.0, (w, h)), (0.0, 0.0));
            assert_eq!(absolute(1.0, 1.0, (w, h)), ((w - 1) as f64, (h - 1) as f64));
            let (x, y) = absolute(0.5, 0.5, (w, h));
            assert!((x - (w - 1) as f64 / 2.0).abs() < 1e-9 && (y - (h - 1) as f64 / 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn absolute_survives_garbage() {
        assert_eq!(absolute(-3.0, 7.0, (1366, 768)), (0.0, 767.0));
        assert_eq!(absolute(f64::NAN, f64::INFINITY, (1366, 768)), (0.0, 0.0));
        assert_eq!(absolute(0.5, 0.5, (0, 0)), (0.0, 0.0));
    }

    /// The cursor packet must land on the pixel of the picture the finger touched, for every pairing
    /// of compositor size and stream size: native, scaled down by the quality cap, HiDPI frames that
    /// are larger than the logical size, and other aspect ratios.
    #[test]
    fn touched_pixel_and_drawn_cursor_agree_in_every_scale() {
        let streams = [(1366, 768), (1280, 720), (854, 480), (1920, 1080), (2732, 1536), (640, 360)];
        for logical in SIZES {
            for stream in streams {
                let space = Space { logical, stream };
                for (nx, ny) in [(0.0f64, 0.0f64), (1.0, 1.0), (0.5, 0.5), (0.25, 0.75), (0.9, 0.1)] {
                    let pos = absolute(nx, ny, logical);
                    let (sx, sy) = to_stream(pos, &space);
                    let want = (nx * (stream.0 - 1) as f64, ny * (stream.1 - 1) as f64);
                    assert!((sx as f64 - want.0).abs() <= 0.5 + 1e-9, "{logical:?} {stream:?} x {sx} vs {}", want.0);
                    assert!((sy as f64 - want.1).abs() <= 0.5 + 1e-9, "{logical:?} {stream:?} y {sy} vs {}", want.1);
                }
            }
        }
    }

    #[test]
    fn stream_coordinates_stay_inside_the_picture() {
        let space = Space { logical: (1366, 768), stream: (1280, 720) };
        assert_eq!(to_stream((-50.0, -50.0), &space), (0, 0));
        assert_eq!(to_stream((9999.0, 9999.0), &space), (1279, 719));
        assert_eq!(to_stream((5.0, 5.0), &Space { logical: (1, 1), stream: (1280, 720) }), (0, 0));
    }

    #[test]
    fn relative_moves_follow_from_a_known_centre_and_clamp() {
        let mut t = Tracker::new((1366, 768));
        assert!(!t.known());
        assert_eq!(t.take(), None);
        let c = t.center();
        assert_eq!(c, (682.5, 383.5));
        t.set(c);
        assert!(t.known());
        assert_eq!(t.take(), Some(c));
        assert_eq!(t.take(), None);
        t.relative(10, -4);
        assert_eq!(t.take(), Some((692.5, 379.5)));
        t.relative(-5000, 5000);
        assert_eq!(t.take(), Some((0.0, 767.0)));
        t.relative(0, 0);
        assert_eq!(t.take(), Some((0.0, 767.0)), "a zero move still reports the place");
    }

    #[test]
    fn an_absolute_move_replaces_whatever_was_tracked() {
        let mut t = Tracker::new((1920, 1080));
        t.relative(300, 300);
        t.set(absolute(0.5, 0.5, (1920, 1080)));
        assert_eq!(t.take(), Some((959.5, 539.5)));
        t.relative(1, 1);
        assert_eq!(t.take(), Some((960.5, 540.5)));
    }
}
