//! The OLED's 1-bit frame (KTD22).
//!
//! Laid out the way SH1106-family controllers take it: eight pages of 128 bytes, each byte a
//! column of eight vertical pixels with the top pixel in bit 0. Companion's 128×64 `rgb`
//! bitmap becomes on/off pixels by a fixed luma threshold, which P0 found keeps Companion's
//! white-on-black text legible.

use crate::bitmap::{self, BitmapError};
use crate::session::{OLED_HEIGHT, OLED_WIDTH};

const WIDTH: usize = OLED_WIDTH as usize;
const HEIGHT: usize = OLED_HEIGHT as usize;
/// Rows per page.
const PAGE_ROWS: usize = 8;
pub const PAGES: usize = HEIGHT / PAGE_ROWS;
pub const FRAME_BYTES: usize = WIDTH * PAGES;
/// A pixel lights when its BT.601 luma (0–255) is at least this.
pub const THRESHOLD: u32 = 128;

#[derive(Clone)]
pub struct MonoFrame {
    bytes: [u8; FRAME_BYTES],
}

impl Default for MonoFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl MonoFrame {
    /// All pixels off.
    pub const fn new() -> Self {
        Self {
            bytes: [0; FRAME_BYTES],
        }
    }

    pub fn clear(&mut self) {
        self.bytes.fill(0);
    }

    /// Sets one pixel; coordinates outside the panel are ignored.
    pub fn set(&mut self, x: usize, y: usize, on: bool) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let byte = &mut self.bytes[(y / PAGE_ROWS) * WIDTH + x];
        let bit = 1 << (y % PAGE_ROWS);
        if on {
            *byte |= bit;
        } else {
            *byte &= !bit;
        }
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        x < WIDTH
            && y < HEIGHT
            && self.bytes[(y / PAGE_ROWS) * WIDTH + x] & (1 << (y % PAGE_ROWS)) != 0
    }

    /// The 128 bytes of one page, as the controller takes them.
    pub fn page(&self, page: usize) -> &[u8] {
        &self.bytes[page * WIDTH..(page + 1) * WIDTH]
    }

    /// Replaces the frame with a Companion `rgb` bitmap. A bad bitmap is reported and the
    /// frame is left as it was.
    pub fn fill_from_rgb(&mut self, b64: &str) -> Result<(), BitmapError> {
        let pixels = bitmap::rgb_pixels(b64, OLED_WIDTH, OLED_HEIGHT)?;
        for (i, [r, g, b]) in pixels.enumerate() {
            self.set(i % WIDTH, i / WIDTH, luma([r, g, b]) >= THRESHOLD);
        }
        Ok(())
    }
}

/// BT.601 luma in integer arithmetic, 0–255.
fn luma([r, g, b]: [u8; 3]) -> u32 {
    (299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b)) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base64;

    fn encode(bytes: &[u8]) -> String {
        let mut out = vec![0u8; base64::encoded_len(bytes.len())];
        base64::encode(bytes, &mut out).unwrap().to_owned()
    }

    /// A black 128×64 RGB image with the given pixels set to `rgb`.
    fn image(lit: &[(usize, usize)], rgb: [u8; 3]) -> String {
        let mut bytes = vec![0u8; WIDTH * HEIGHT * 3];
        for &(x, y) in lit {
            let i = (y * WIDTH + x) * 3;
            bytes[i..i + 3].copy_from_slice(&rgb);
        }
        encode(&bytes)
    }

    fn all(frame: &MonoFrame) -> Vec<u8> {
        (0..PAGES).flat_map(|p| frame.page(p).to_vec()).collect()
    }

    #[test]
    fn white_and_black_fill_every_byte() {
        let mut frame = MonoFrame::new();
        frame
            .fill_from_rgb(&encode(&[0xff; WIDTH * HEIGHT * 3]))
            .unwrap();
        assert!(all(&frame).iter().all(|&b| b == 0xff));
        frame.fill_from_rgb(&image(&[], [0; 3])).unwrap();
        assert!(all(&frame).iter().all(|&b| b == 0x00));
    }

    #[test]
    fn pixels_land_in_page_major_column_bytes() {
        let mut frame = MonoFrame::new();
        frame.fill_from_rgb(&image(&[(0, 0)], [0xff; 3])).unwrap();
        assert_eq!(frame.page(0)[0], 0b0000_0001);
        assert_eq!(all(&frame).iter().filter(|&&b| b != 0).count(), 1);

        frame.fill_from_rgb(&image(&[(5, 9)], [0xff; 3])).unwrap();
        assert_eq!(frame.page(1)[5], 0b0000_0010);
        assert_eq!(frame.page(0)[0], 0, "the previous image is gone");
    }

    #[test]
    fn luma_128_lights_and_127_does_not() {
        let mut frame = MonoFrame::new();
        frame.fill_from_rgb(&image(&[(1, 1)], [127; 3])).unwrap();
        assert!(!frame.get(1, 1));
        frame.fill_from_rgb(&image(&[(1, 1)], [128; 3])).unwrap();
        assert!(frame.get(1, 1));
    }

    #[test]
    fn saturated_red_stays_dark() {
        // P0's caveat: red has a luma of about 76.
        let mut frame = MonoFrame::new();
        frame
            .fill_from_rgb(&image(&[(3, 3)], [0xff, 0, 0]))
            .unwrap();
        assert!(!frame.get(3, 3));
    }

    #[test]
    fn a_bad_bitmap_leaves_the_frame_unchanged() {
        let mut frame = MonoFrame::new();
        frame.set(10, 10, true);
        assert_eq!(
            frame.fill_from_rgb(&encode(&[0xff; 128 * 128 * 3])).err(),
            Some(BitmapError::WrongLength {
                expected: 24_576,
                actual: 49_152
            })
        );
        assert_eq!(
            frame.fill_from_rgb("AA*A").err(),
            Some(BitmapError::InvalidBase64)
        );
        assert!(frame.get(10, 10));
        assert_eq!(all(&frame).iter().filter(|&&b| b != 0).count(), 1);
    }

    #[test]
    fn set_get_and_clear() {
        let mut frame = MonoFrame::new();
        frame.set(127, 63, true);
        assert!(frame.get(127, 63));
        assert_eq!(frame.page(7)[127], 0b1000_0000);
        frame.set(128, 0, true);
        frame.set(0, 64, true);
        assert_eq!(
            all(&frame).iter().filter(|&&b| b != 0).count(),
            1,
            "out of range ignored"
        );
        frame.set(127, 63, false);
        assert!(!frame.get(127, 63));
        frame.set(1, 1, true);
        frame.clear();
        assert!(all(&frame).iter().all(|&b| b == 0));
    }
}
