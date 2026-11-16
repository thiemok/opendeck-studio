//! Streams a base64 `rgb` bitmap from a `KEY-STATE` line as RGB565 pixels (KTD15).
//!
//! The whole bitmap is checked before the first pixel is yielded, so a bad
//! image is skipped rather than half-drawn.

use crate::base64::{INVALID, SEXTETS};

/// Why a bitmap was not drawn.
#[derive(Debug, PartialEq, Eq)]
pub enum BitmapError {
    /// The decoded size is not `width × height × 3` bytes.
    WrongLength { expected: usize, actual: usize },
    /// The text contains characters outside the base64 alphabet, or misplaced padding.
    InvalidBase64,
}

/// Checks `b64` against `width × height` and returns its RGB565 pixels in row-major order.
pub fn rgb565_pixels(
    b64: &str,
    width: u16,
    height: u16,
) -> Result<impl Iterator<Item = u16> + '_, BitmapError> {
    Ok(rgb_pixels(b64, width, height)?.map(rgb565))
}

/// One RGB888 pixel as RGB565, by truncation.
pub fn rgb565([r, g, b]: [u8; 3]) -> u16 {
    let (r, g, b) = (u16::from(r), u16::from(g), u16::from(b));
    ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)
}

/// Checks `b64` against `width × height` and returns its `[r, g, b]` pixels in row-major order.
pub fn rgb_pixels(b64: &str, width: u16, height: u16) -> Result<Pixels<'_>, BitmapError> {
    let text = b64.as_bytes();
    let padding = validate(text)?;
    let expected = usize::from(width) * usize::from(height) * 3;
    let actual = text.len() / 4 * 3 - padding;
    if actual != expected {
        return Err(BitmapError::WrongLength { expected, actual });
    }
    Ok(Pixels { text })
}

/// Checks the alphabet and padding placement; returns the number of padding characters.
fn validate(text: &[u8]) -> Result<usize, BitmapError> {
    if !text.len().is_multiple_of(4) {
        return Err(BitmapError::InvalidBase64);
    }
    let padding = text
        .iter()
        .rev()
        .take(2)
        .take_while(|&&c| c == b'=')
        .count();
    let body = &text[..text.len() - padding];
    if body.iter().all(|&c| SEXTETS[usize::from(c)] != INVALID) {
        Ok(padding)
    } else {
        Err(BitmapError::InvalidBase64)
    }
}

/// RGB pixels decoded on demand, four base64 characters per pixel.
pub struct Pixels<'a> {
    text: &'a [u8],
}

impl Iterator for Pixels<'_> {
    type Item = [u8; 3];

    fn next(&mut self) -> Option<[u8; 3]> {
        let (quad, rest) = self.text.split_first_chunk::<4>()?;
        self.text = rest;
        let n = quad.iter().fold(0u32, |acc, &c| {
            (acc << 6) | u32::from(SEXTETS[usize::from(c)])
        });
        let [_, r, g, b] = n.to_be_bytes();
        Some([r, g, b])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.text.len() / 4;
        (n, Some(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base64;

    fn encode(bytes: &[u8]) -> String {
        let mut out = vec![0u8; base64::encoded_len(bytes.len())];
        base64::encode(bytes, &mut out).unwrap().to_owned()
    }

    #[test]
    fn red_and_blue_become_rgb565() {
        let b64 = encode(&[0xff, 0x00, 0x00, 0x00, 0x00, 0xff]);
        let pixels: Vec<u16> = rgb565_pixels(&b64, 2, 1).unwrap().collect();
        assert_eq!(pixels, vec![0xF800, 0x001F]);
    }

    #[test]
    fn white_green_and_mid_grey_convert_by_truncation() {
        let b64 = encode(&[0xff, 0xff, 0xff, 0x00, 0xff, 0x00, 0x80, 0x80, 0x80]);
        let pixels: Vec<u16> = rgb565_pixels(&b64, 3, 1).unwrap().collect();
        assert_eq!(pixels, vec![0xFFFF, 0x07E0, 0x8410]);
    }

    #[test]
    fn a_full_key_yields_16384_pixels() {
        let b64 = encode(&vec![0x11; 128 * 128 * 3]);
        assert_eq!(rgb565_pixels(&b64, 128, 128).unwrap().count(), 16_384);
    }

    #[test]
    fn a_short_bitmap_is_rejected_before_any_pixel() {
        let b64 = encode(&vec![0x11; 128 * 128 * 3 - 1]);
        assert_eq!(
            rgb565_pixels(&b64, 128, 128).err(),
            Some(BitmapError::WrongLength {
                expected: 49_152,
                actual: 49_151
            })
        );
    }

    #[test]
    fn invalid_characters_are_rejected() {
        let mut b64 = encode(&[0x10; 6]);
        b64.replace_range(2..3, "*");
        assert_eq!(
            rgb565_pixels(&b64, 2, 1).err(),
            Some(BitmapError::InvalidBase64)
        );
    }

    #[test]
    fn padding_in_the_middle_is_rejected() {
        assert_eq!(
            rgb565_pixels("AA==AAAA", 2, 1).err(),
            Some(BitmapError::InvalidBase64)
        );
    }
}
