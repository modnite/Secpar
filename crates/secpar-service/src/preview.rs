//! This computer's screen as another computer's list shows it: the primary
//! display, scaled down, as one JPEG still.
//!
//! On Windows the picture is taken with GDI from the desktop as it is drawn now.
//! That needs no duplication session of its own, so it does not take the
//! output's duplication away from the hosting engine while a stream runs.
//!
//! Linux has no still yet. The session's screen is reached through KWin or the
//! desktop portal, and this part speaks neither, so Linux answers
//! `not_available` and the other computer keeps its monitor icon.
use secpar_hosting::{Previews, Still};

/// The primary display of this computer.
pub struct Screen;

impl Previews for Screen {
    fn still(&self, max_width: u32) -> Option<Still> {
        screen::still(max_width)
    }
}

#[cfg(windows)]
mod screen {
    use super::Still;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The quality a still is written at. Small enough for a tile, clear enough
    /// to see what is on the screen.
    const QUALITY: u8 = 65;
    /// The quality a still is written again at when the first one is too large.
    const SMALLER: u8 = 45;
    /// The largest JPEG that is answered with. A picture over it is written again
    /// at a lower quality, so the base64 answer stays well inside the app
    /// channel's frame.
    const MAX_JPEG: usize = 320 * 1024;

    pub fn still(max_width: u32) -> Option<Still> {
        let (bgra, width, height) = grab()?;
        let (scaled_width, scaled_height) = scaled_size(width, height, max_width);
        let rgb = downscale(&bgra, width, height, width as usize * 4, scaled_width, scaled_height);
        let mut picture = jpeg(&rgb, scaled_width, scaled_height, QUALITY)?;
        if picture.len() > MAX_JPEG {
            picture = jpeg(&rgb, scaled_width, scaled_height, SMALLER)?;
        }
        if picture.len() > MAX_JPEG {
            return None;
        }
        Some(Still { jpeg: picture, width: scaled_width, height: scaled_height, at: now() })
    }

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }

    /// The width and height of a still of a `width`×`height` screen that is at
    /// most `max_width` wide. A screen that is already narrower keeps its own
    /// size; the shape of the screen is kept.
    fn scaled_size(width: u32, height: u32, max_width: u32) -> (u32, u32) {
        let max_width = max_width.max(1);
        if width <= max_width {
            return (width.max(1), height.max(1));
        }
        let scaled = (height as f64 * max_width as f64 / width as f64).round() as u32;
        (max_width, scaled.max(1))
    }

    /// Averages a top-down BGRA picture (each row `pitch` bytes) down to
    /// `to_width`×`to_height` RGB, which is what the JPEG writer takes.
    fn downscale(src: &[u8], width: u32, height: u32, pitch: usize, to_width: u32, to_height: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((to_width * to_height * 3) as usize);
        for to_y in 0..to_height {
            let (y0, y1) = (to_y * height / to_height, ((to_y + 1) * height / to_height).max(to_y * height / to_height + 1));
            for to_x in 0..to_width {
                let (x0, x1) = (to_x * width / to_width, ((to_x + 1) * width / to_width).max(to_x * width / to_width + 1));
                let mut sum = [0u32; 3];
                for y in y0..y1.min(height) {
                    for x in x0..x1.min(width) {
                        let at = y as usize * pitch + x as usize * 4;
                        // BGRA in memory, RGB out.
                        sum[0] += src[at + 2] as u32;
                        sum[1] += src[at + 1] as u32;
                        sum[2] += src[at] as u32;
                    }
                }
                let count = ((y1.min(height) - y0) * (x1.min(width) - x0)).max(1);
                out.extend(sum.map(|sum| (sum / count) as u8));
            }
        }
        out
    }

    /// `rgb` as a JPEG at `quality`.
    fn jpeg(rgb: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
        let mut buffer = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, quality)
            .encode(rgb, width, height, image::ExtendedColorType::Rgb8)
            .ok()?;
        Some(buffer)
    }

    /// One picture of the primary display as top-down BGRA, with the size it was
    /// taken at. `None` when there is no desktop to take: a locked screen, a
    /// session without a visible display, or a screen the system refuses.
    fn grab() -> Option<(Vec<u8>, u32, u32)> {
        use windows_sys::Win32::Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        // SAFETY: every handle is created and released in this call. The desktop
        // is only read while `mem` holds the bitmap, and the bytes are copied.
        unsafe {
            let (width, height) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            if width <= 0 || height <= 0 {
                return None;
            }
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return None;
            }
            let mem = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let old = SelectObject(mem, bitmap);
            // CAPTUREBLT includes layered windows, so a window drawn by the
            // engine or a game is in the still as well.
            let copied = BitBlt(mem, 0, 0, width, height, screen, 0, 0, SRCCOPY | CAPTUREBLT) != 0;
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            // A negative height gives the rows top-down, the way they are read.
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut buffer = vec![0u8; width as usize * height as usize * 4];
            let lines = if copied {
                GetDIBits(mem, bitmap, 0, height as u32, buffer.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS)
            } else {
                0
            };
            SelectObject(mem, old);
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(mem);
            let _ = ReleaseDC(std::ptr::null_mut(), screen);
            (lines == height).then_some((buffer, width as u32, height as u32))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_still_keeps_the_screens_shape_and_never_grows() {
            assert_eq!(scaled_size(2560, 1440, 480), (480, 270));
            assert_eq!(scaled_size(1920, 1200, 480), (480, 300));
            assert_eq!(scaled_size(1366, 768, 480), (480, 270));
            // A screen narrower than the request is answered at its own size.
            assert_eq!(scaled_size(320, 200, 480), (320, 200));
        }

        #[test]
        fn downscaling_averages_bgra_and_drops_the_alpha() {
            // Two pixels of one row: red on the left, blue on the right.
            let src = vec![0, 0, 255, 255, 255, 0, 0, 255];
            assert_eq!(downscale(&src, 2, 1, 8, 1, 1), vec![127, 0, 127]);
            assert_eq!(downscale(&src, 2, 1, 8, 2, 1), vec![255, 0, 0, 0, 0, 255]);
        }

        #[test]
        fn a_still_is_a_jpeg_of_the_width_it_was_asked_for() {
            // A picture of this computer, if there is a desktop to take one of.
            let Some(still) = still(480) else {
                eprintln!("no still: the screen is locked or there is no display");
                return;
            };
            assert_eq!(still.width, 480);
            assert_eq!(&still.jpeg[..2], &[0xFF, 0xD8], "a JPEG starts with its marker");
            assert_eq!(&still.jpeg[still.jpeg.len() - 2..], &[0xFF, 0xD9]);
            assert!(still.jpeg.len() < MAX_JPEG);
            assert!(still.at > 0);
        }
    }
}

#[cfg(not(windows))]
mod screen {
    use super::Still;

    /// Nothing to capture here yet: Linux is reached through KWin or the desktop
    /// portal, which this part does not speak.
    pub fn still(_max_width: u32) -> Option<Still> {
        None
    }
}
