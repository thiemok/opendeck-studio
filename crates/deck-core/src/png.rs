//! Streams a Companion `png` bitmap from a `KEY-STATE` line as rows of RGB (THI-76, KTD3).
//!
//! `BITMAP` holds `data:image/png;base64,…`. The text is base64-decoded in small blocks,
//! the PNG chunks are parsed as they arrive, `IDAT` is inflated into a 32 KB wrapping window,
//! and each scanline is unfiltered against the previous one and handed to the caller. No
//! buffer ever holds the whole image. Only 8-bit, non-interlaced RGB and RGBA images are
//! supported, which is what Companion sends.
//!
//! Unlike the `rgb` path, a fault in the image data is found only when it is reached, so
//! rows before it have already been delivered.

use crate::base64;
use crate::crc32::Crc32;
use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::inflate_flags::{
    TINFL_FLAG_COMPUTE_ADLER32, TINFL_FLAG_HAS_MORE_INPUT, TINFL_FLAG_PARSE_ZLIB_HEADER,
};
use miniz_oxide::inflate::core::{DecompressorOxide, decompress};

/// What Companion puts before the base64 PNG in `BITMAP`.
pub const DATA_URL_PREFIX: &str = "data:image/png;base64,";
/// The inflate window: deflate may refer back up to 32 KB.
pub const WINDOW_SIZE: usize = 32 * 1024;
/// The widest image the workspace holds a scanline for.
pub const MAX_WIDTH: usize = 128;
const MAX_STRIDE: usize = MAX_WIDTH * 4;
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
/// Base64 characters decoded per block; 192 bytes of PNG.
const BLOCK_CHARS: usize = 256;

/// Why a PNG was not (fully) drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngError {
    /// No `data:image/png;base64,` prefix, or no PNG signature.
    NotPng,
    /// Characters outside the base64 alphabet, or misplaced padding.
    InvalidBase64,
    /// The image is not the size the control asked for.
    WrongSize {
        expected: (u16, u16),
        actual: (u32, u32),
    },
    /// Not 8-bit, non-interlaced RGB or RGBA.
    Unsupported {
        color_type: u8,
        bit_depth: u8,
        interlaced: bool,
    },
    /// A chunk's CRC does not match.
    BadCrc,
    /// The zlib stream is corrupt, or its Adler-32 does not match.
    Inflate,
    /// A scanline names a filter type other than 0–4.
    BadFilter(u8),
    /// Chunks out of order, or an `IHDR` of the wrong length.
    Malformed,
    /// The data ended before `IEND`, or before every row.
    Truncated,
}

/// Working memory for one decode at a time: about 11 KB of inflate state, the 32 KB window
/// and three scanlines.
///
/// All-zero bytes are a valid value, so the firmware can allocate it zeroed on the heap
/// rather than build it on a stack (KTD4). miniz_oxide documents its decompressor's zeroed
/// state as its initial one.
pub struct PngWorkspace {
    inflater: DecompressorOxide,
    window: [u8; WINDOW_SIZE],
    rows: [[u8; MAX_STRIDE]; 2],
    rgb: [u8; MAX_WIDTH * 3],
}

impl Default for PngWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl PngWorkspace {
    pub fn new() -> Self {
        Self {
            inflater: DecompressorOxide::new(),
            window: [0; WINDOW_SIZE],
            rows: [[0; MAX_STRIDE]; 2],
            rgb: [0; MAX_WIDTH * 3],
        }
    }
}

/// Decodes a `data:image/png;base64,…` bitmap that must be `width × height`, calling `row`
/// with each row's index and its `width × 3` RGB bytes, top to bottom.
pub fn decode(
    data_url: &str,
    width: u16,
    height: u16,
    ws: &mut PngWorkspace,
    mut row: impl FnMut(u16, &[u8]),
) -> Result<(), PngError> {
    let text = data_url
        .strip_prefix(DATA_URL_PREFIX)
        .ok_or(PngError::NotPng)?
        .as_bytes();
    if !text.len().is_multiple_of(4) {
        return Err(PngError::InvalidBase64);
    }
    if usize::from(width) > MAX_WIDTH {
        return Err(PngError::WrongSize {
            expected: (width, height),
            actual: (0, 0),
        });
    }
    ws.inflater.init();
    let mut decoder = Decoder::new(width, height);
    let mut block = [0u8; BLOCK_CHARS / 4 * 3];
    let mut chunks = text.chunks(BLOCK_CHARS).peekable();
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        let n = base64::decode_quads(chunk, &mut block, last).ok_or(PngError::InvalidBase64)?;
        decoder.feed(&block[..n], ws, &mut row)?;
        if decoder.stage == Stage::End {
            break;
        }
    }
    decoder.finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Signature,
    Length,
    Type,
    Data,
    Crc,
    End,
}

#[derive(Debug, Clone, Copy)]
struct Header {
    /// Bytes per pixel: 3 or 4.
    bpp: usize,
    stride: usize,
}

struct Decoder {
    width: u16,
    height: u16,
    stage: Stage,
    /// Bytes collected for a fixed-size field: signature, length, type or CRC.
    field: [u8; 8],
    have: usize,
    /// The `IHDR` chunk's data, kept until its CRC has been checked.
    ihdr: [u8; 13],
    ihdr_len: usize,
    chunk_type: [u8; 4],
    remaining: u32,
    crc: Crc32,
    header: Option<Header>,
    out_pos: usize,
    inflated: bool,
    /// Scanline assembly: the filter byte (`None` until read), bytes filled, which of the
    /// two rows is current, and the next row's index.
    filter: Option<u8>,
    filled: usize,
    current: usize,
    y: u16,
}

impl Decoder {
    fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            stage: Stage::Signature,
            field: [0; 8],
            have: 0,
            ihdr: [0; 13],
            ihdr_len: 0,
            chunk_type: [0; 4],
            remaining: 0,
            crc: Crc32::new(),
            header: None,
            out_pos: 0,
            inflated: false,
            filter: None,
            filled: 0,
            current: 0,
            y: 0,
        }
    }

    /// Copies from `data` into `field` until it holds `len` bytes; true once it does.
    fn collect(&mut self, data: &mut &[u8], len: usize) -> bool {
        let take = (len - self.have).min(data.len());
        self.field[self.have..self.have + take].copy_from_slice(&data[..take]);
        self.have += take;
        *data = &data[take..];
        if self.have == len {
            self.have = 0;
            true
        } else {
            false
        }
    }

    fn feed(
        &mut self,
        mut data: &[u8],
        ws: &mut PngWorkspace,
        row: &mut impl FnMut(u16, &[u8]),
    ) -> Result<(), PngError> {
        while !data.is_empty() {
            match self.stage {
                Stage::Signature => {
                    if self.collect(&mut data, 8) {
                        if self.field[..8] != SIGNATURE {
                            return Err(PngError::NotPng);
                        }
                        self.stage = Stage::Length;
                    }
                }
                Stage::Length => {
                    if self.collect(&mut data, 4) {
                        let len = be_u32(&self.field);
                        if len > i32::MAX as u32 {
                            return Err(PngError::Malformed);
                        }
                        self.remaining = len;
                        self.stage = Stage::Type;
                    }
                }
                Stage::Type => {
                    if self.collect(&mut data, 4) {
                        self.chunk_type.copy_from_slice(&self.field[..4]);
                        self.crc = Crc32::new();
                        self.crc.update(&self.chunk_type);
                        self.begin_chunk()?;
                        self.stage = if self.remaining == 0 {
                            Stage::Crc
                        } else {
                            Stage::Data
                        };
                    }
                }
                Stage::Data => {
                    let take = data.len().min(self.remaining as usize);
                    let (bytes, rest) = data.split_at(take);
                    data = rest;
                    self.crc.update(bytes);
                    self.remaining -= take as u32;
                    match &self.chunk_type {
                        b"IHDR" => {
                            // `begin_chunk` checked the length is exactly 13.
                            self.ihdr[self.ihdr_len..self.ihdr_len + bytes.len()]
                                .copy_from_slice(bytes);
                            self.ihdr_len += bytes.len();
                        }
                        b"IDAT" => self.inflate(bytes, ws, row)?,
                        _ => {}
                    }
                    if self.remaining == 0 {
                        self.stage = Stage::Crc;
                    }
                }
                Stage::Crc => {
                    if self.collect(&mut data, 4) {
                        let stored = be_u32(&self.field);
                        if stored != self.crc.finish() {
                            return Err(PngError::BadCrc);
                        }
                        self.end_chunk()?;
                    }
                }
                Stage::End => return Ok(()),
            }
        }
        Ok(())
    }

    /// Checks a chunk's place in the stream once its type is known.
    fn begin_chunk(&self) -> Result<(), PngError> {
        match &self.chunk_type {
            b"IHDR" if self.header.is_some() || self.remaining != 13 => Err(PngError::Malformed),
            b"IHDR" => Ok(()),
            _ if self.header.is_none() => Err(PngError::Malformed),
            _ => Ok(()),
        }
    }

    fn end_chunk(&mut self) -> Result<(), PngError> {
        match &self.chunk_type {
            b"IHDR" => self.header = Some(self.check_header()?),
            b"IEND" => {
                self.stage = Stage::End;
                return Ok(());
            }
            _ => {}
        }
        self.stage = Stage::Length;
        Ok(())
    }

    fn check_header(&self) -> Result<Header, PngError> {
        let f = &self.ihdr;
        let (width, height) = (be_u32(&f[..4]), be_u32(&f[4..8]));
        let (bit_depth, color_type, compression, filter, interlace) =
            (f[8], f[9], f[10], f[11], f[12]);
        if compression != 0 || filter != 0 {
            return Err(PngError::Malformed);
        }
        let bpp = match (bit_depth, color_type, interlace) {
            (8, 2, 0) => 3,
            (8, 6, 0) => 4,
            _ => {
                return Err(PngError::Unsupported {
                    color_type,
                    bit_depth,
                    interlaced: interlace != 0,
                });
            }
        };
        if (width, height) != (u32::from(self.width), u32::from(self.height)) {
            return Err(PngError::WrongSize {
                expected: (self.width, self.height),
                actual: (width, height),
            });
        }
        Ok(Header {
            bpp,
            stride: usize::from(self.width) * bpp,
        })
    }

    fn inflate(
        &mut self,
        mut input: &[u8],
        ws: &mut PngWorkspace,
        row: &mut impl FnMut(u16, &[u8]),
    ) -> Result<(), PngError> {
        let flags =
            TINFL_FLAG_PARSE_ZLIB_HEADER | TINFL_FLAG_HAS_MORE_INPUT | TINFL_FLAG_COMPUTE_ADLER32;
        while !self.inflated {
            let (status, consumed, written) =
                decompress(&mut ws.inflater, input, &mut ws.window, self.out_pos, flags);
            input = &input[consumed..];
            let out = &ws.window[self.out_pos..self.out_pos + written];
            self.scanlines(out, &mut ws.rows, &mut ws.rgb, row)?;
            self.out_pos = (self.out_pos + written) & (WINDOW_SIZE - 1);
            match status {
                TINFLStatus::Done => self.inflated = true,
                TINFLStatus::NeedsMoreInput => return Ok(()),
                TINFLStatus::HasMoreOutput => {}
                _ => return Err(PngError::Inflate),
            }
        }
        Ok(())
    }

    /// Fills scanlines from inflated bytes and delivers each completed one.
    fn scanlines(
        &mut self,
        mut data: &[u8],
        rows: &mut [[u8; MAX_STRIDE]; 2],
        rgb: &mut [u8; MAX_WIDTH * 3],
        row: &mut impl FnMut(u16, &[u8]),
    ) -> Result<(), PngError> {
        let Some(Header { bpp, stride }) = self.header else {
            return Err(PngError::Malformed);
        };
        while !data.is_empty() && self.y < self.height {
            let Some(filter) = self.filter else {
                if data[0] > 4 {
                    return Err(PngError::BadFilter(data[0]));
                }
                self.filter = Some(data[0]);
                data = &data[1..];
                continue;
            };
            let take = (stride - self.filled).min(data.len());
            rows[self.current][self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled < stride {
                continue;
            }

            let [a, b] = rows;
            let (cur, prev) = if self.current == 0 {
                (a, &*b)
            } else {
                (b, &*a)
            };
            // The row above the first one is all zeros.
            let prev: &[u8] = if self.y == 0 { &[] } else { &prev[..stride] };
            unfilter(filter, bpp, &mut cur[..stride], prev);
            for (px, out) in cur[..stride]
                .chunks_exact(bpp)
                .zip(rgb.as_chunks_mut::<3>().0)
            {
                out.copy_from_slice(&px[..3]);
            }
            row(self.y, &rgb[..usize::from(self.width) * 3]);

            self.y += 1;
            self.current ^= 1;
            self.filled = 0;
            self.filter = None;
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), PngError> {
        if self.stage != Stage::End || !self.inflated || self.y < self.height {
            return Err(PngError::Truncated);
        }
        Ok(())
    }
}

/// The big-endian `u32` in the first four bytes of `bytes`.
fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Reverses one scanline's filter in place. An empty `prev` stands for a row of zeros.
fn unfilter(filter: u8, bpp: usize, cur: &mut [u8], prev: &[u8]) {
    let up = |i: usize| prev.get(i).copied().unwrap_or(0);
    for i in 0..cur.len() {
        let left = if i >= bpp { cur[i - bpp] } else { 0 };
        let upper_left = if i >= bpp { up(i - bpp) } else { 0 };
        let predictor = match filter {
            1 => left,
            2 => up(i),
            3 => ((u16::from(left) + u16::from(up(i))) / 2) as u8,
            4 => paeth(left, up(i), upper_left),
            _ => 0,
        };
        cur[i] = cur[i].wrapping_add(predictor);
    }
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc32::crc32;

    fn data_url(png: &[u8]) -> String {
        let mut out = vec![0u8; base64::encoded_len(png.len())];
        format!(
            "{DATA_URL_PREFIX}{}",
            base64::encode(png, &mut out).unwrap()
        )
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = kind.to_vec();
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        out
    }

    fn ihdr(w: u32, h: u32, depth: u8, color: u8, interlace: u8) -> Vec<u8> {
        let mut d = w.to_be_bytes().to_vec();
        d.extend_from_slice(&h.to_be_bytes());
        d.extend_from_slice(&[depth, color, 0, 0, interlace]);
        chunk(b"IHDR", &d)
    }

    /// Filters `pixels` (rows of `w × bpp` bytes) with `filter` on every row: the encoder side,
    /// written from the PNG spec independently of [`unfilter`].
    fn filtered(pixels: &[u8], w: usize, bpp: usize, filter: u8) -> Vec<u8> {
        let stride = w * bpp;
        let mut out = Vec::new();
        for (y, line) in pixels.chunks(stride).enumerate() {
            out.push(filter);
            for x in 0..stride {
                let a = if x >= bpp { line[x - bpp] } else { 0 };
                let b = if y > 0 {
                    pixels[(y - 1) * stride + x]
                } else {
                    0
                };
                let c = if y > 0 && x >= bpp {
                    pixels[(y - 1) * stride + x - bpp]
                } else {
                    0
                };
                let pred = match filter {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) / 2) as u8,
                    _ => {
                        let p = a as i32 + b as i32 - c as i32;
                        let (pa, pb, pc) = (
                            (p - a as i32).abs(),
                            (p - b as i32).abs(),
                            (p - c as i32).abs(),
                        );
                        if pa <= pb && pa <= pc {
                            a
                        } else if pb <= pc {
                            b
                        } else {
                            c
                        }
                    }
                };
                out.push(line[x].wrapping_sub(pred));
            }
        }
        out
    }

    fn png(w: u32, h: u32, color: u8, idat_parts: &[&[u8]]) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        out.extend(ihdr(w, h, 8, color, 0));
        out.extend(chunk(b"tEXt", b"Software\0test"));
        for part in idat_parts {
            out.extend(chunk(b"IDAT", part));
        }
        out.extend(chunk(b"IEND", &[]));
        out
    }

    fn zlib(raw: &[u8]) -> Vec<u8> {
        miniz_oxide::deflate::compress_to_vec_zlib(raw, 6)
    }

    /// Decodes into one RGB buffer, or the error with the rows delivered before it.
    fn decode_all(url: &str, w: u16, h: u16) -> Result<Vec<u8>, (PngError, usize)> {
        let mut ws = PngWorkspace::new();
        let mut out = Vec::new();
        let mut rows = 0;
        decode(url, w, h, &mut ws, |y, rgb| {
            assert_eq!(usize::from(y), rows, "rows arrive in order");
            assert_eq!(rgb.len(), usize::from(w) * 3);
            rows += 1;
            out.extend_from_slice(rgb);
        })
        .map(|()| out)
        .map_err(|e| (e, rows))
    }

    /// A 4×3 RGB image whose rows differ, so every filter type changes the bytes.
    fn sample_rgb() -> Vec<u8> {
        (0..4 * 3 * 3).map(|i| (i * 37 % 251) as u8).collect()
    }

    fn rgba_of(rgb: &[u8]) -> Vec<u8> {
        rgb.chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 0x80])
            .collect()
    }

    #[test]
    fn every_filter_type_decodes_to_the_original_pixels() {
        let rgb = sample_rgb();
        for filter in 0..=4 {
            let raw = filtered(&rgb, 4, 3, filter);
            let url = data_url(&png(4, 3, 2, &[&zlib(&raw)]));
            assert_eq!(decode_all(&url, 4, 3), Ok(rgb.clone()), "filter {filter}");
        }
    }

    #[test]
    fn rgba_drops_alpha() {
        let rgb = sample_rgb();
        for filter in 0..=4 {
            let raw = filtered(&rgba_of(&rgb), 4, 4, filter);
            let url = data_url(&png(4, 3, 6, &[&zlib(&raw)]));
            assert_eq!(decode_all(&url, 4, 3), Ok(rgb.clone()), "filter {filter}");
        }
    }

    #[test]
    fn idat_split_across_chunks_decodes_the_same() {
        let rgb = sample_rgb();
        let z = zlib(&filtered(&rgb, 4, 3, 4));
        let parts: Vec<&[u8]> = z.chunks(3).collect();
        let url = data_url(&png(4, 3, 2, &parts));
        assert_eq!(decode_all(&url, 4, 3), Ok(rgb));
    }

    #[test]
    fn a_long_png_crosses_many_base64_blocks() {
        // Incompressible noise makes the PNG several base64 blocks long, so chunk
        // boundaries and block boundaries fall in different places.
        let mut seed = 7u32;
        let rgb: Vec<u8> = (0..32 * 32 * 3)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (seed >> 16) as u8
            })
            .collect();
        let z = zlib(&filtered(&rgb, 32, 3, 1));
        let parts: Vec<&[u8]> = z.chunks(1000).collect();
        let url = data_url(&png(32, 32, 2, &parts));
        assert!(url.len() > 4 * BLOCK_CHARS);
        assert_eq!(decode_all(&url, 32, 32), Ok(rgb));
    }

    #[test]
    fn companion_pngs_match_the_rgb_of_the_same_button() {
        // Captured from Companion 5.1.3 (THI-76 desktop run): the same button sent
        // once as `png` and once as `rgb`. Companion uses Sub, Up and Paeth filters.
        for (png_bytes, rgb, w, h) in [
            (
                &include_bytes!("../tests/fixtures/companion-128x128-key.png")[..],
                &include_bytes!("../tests/fixtures/companion-128x128-key.rgb")[..],
                128,
                128,
            ),
            (
                &include_bytes!("../tests/fixtures/companion-128x64-oled.png")[..],
                &include_bytes!("../tests/fixtures/companion-128x64-oled.rgb")[..],
                128,
                64,
            ),
        ] {
            assert_eq!(decode_all(&data_url(png_bytes), w, h).as_deref(), Ok(rgb));
        }
    }

    #[test]
    fn a_zeroed_workspace_decodes() {
        // The firmware allocates the workspace zeroed (KTD4).
        let mut ws: Box<PngWorkspace> = unsafe { Box::<PngWorkspace>::new_zeroed().assume_init() };
        let rgb = sample_rgb();
        let url = data_url(&png(4, 3, 2, &[&zlib(&filtered(&rgb, 4, 3, 4))]));
        let mut out = Vec::new();
        decode(&url, 4, 3, &mut ws, |_, row| out.extend_from_slice(row)).unwrap();
        assert_eq!(out, rgb);
        // And a workspace is reusable for the next image.
        out.clear();
        decode(&url, 4, 3, &mut ws, |_, row| out.extend_from_slice(row)).unwrap();
        assert_eq!(out, rgb);
    }

    #[test]
    fn the_workspace_stays_within_its_45_kb_budget() {
        // KTD3/KTD4 of the THI-76 plan: the window, the inflate state and three scanlines.
        let size = core::mem::size_of::<PngWorkspace>();
        assert!(size > WINDOW_SIZE && size <= 45 * 1024, "{size} B");
    }

    #[test]
    fn the_wrong_size_is_rejected_before_any_row() {
        let url = data_url(&png(4, 3, 2, &[&zlib(&filtered(&sample_rgb(), 4, 3, 0))]));
        assert_eq!(
            decode_all(&url, 4, 4),
            Err((
                PngError::WrongSize {
                    expected: (4, 4),
                    actual: (4, 3)
                },
                0
            ))
        );
    }

    #[test]
    fn unsupported_formats_are_rejected() {
        for (depth, color, interlace) in [(8, 3, 0), (16, 2, 0), (8, 2, 1), (8, 0, 0)] {
            let mut bytes = SIGNATURE.to_vec();
            bytes.extend(ihdr(4, 3, depth, color, interlace));
            bytes.extend(chunk(b"IEND", &[]));
            assert_eq!(
                decode_all(&data_url(&bytes), 4, 3),
                Err((
                    PngError::Unsupported {
                        color_type: color,
                        bit_depth: depth,
                        interlaced: interlace != 0
                    },
                    0
                ))
            );
        }
    }

    #[test]
    fn a_flipped_idat_byte_fails_the_crc() {
        let mut bytes = png(4, 3, 2, &[&zlib(&filtered(&sample_rgb(), 4, 3, 0))]);
        let idat = bytes.windows(4).position(|w| w == b"IDAT").unwrap();
        bytes[idat + 6] ^= 0x01;
        assert_eq!(
            decode_all(&data_url(&bytes), 4, 3).map_err(|e| e.0),
            Err(PngError::BadCrc)
        );
    }

    #[test]
    fn corrupt_zlib_data_with_a_valid_crc_fails_to_inflate() {
        let mut z = zlib(&filtered(&sample_rgb(), 4, 3, 0));
        let last = z.len() - 1;
        z[last] ^= 0xff; // the Adler-32
        let url = data_url(&png(4, 3, 2, &[&z]));
        assert_eq!(
            decode_all(&url, 4, 3).map_err(|e| e.0),
            Err(PngError::Inflate)
        );
    }

    #[test]
    fn a_missing_iend_is_truncated() {
        let mut bytes = png(4, 3, 2, &[&zlib(&filtered(&sample_rgb(), 4, 3, 0))]);
        bytes.truncate(bytes.len() - 12);
        assert_eq!(
            decode_all(&data_url(&bytes), 4, 3),
            Err((PngError::Truncated, 3))
        );
    }

    #[test]
    fn too_few_rows_is_truncated() {
        let rgb = sample_rgb();
        let z = zlib(&filtered(&rgb[..4 * 3 * 2], 4, 3, 0));
        let url = data_url(&png(4, 3, 2, &[&z]));
        assert_eq!(decode_all(&url, 4, 3), Err((PngError::Truncated, 2)));
    }

    #[test]
    fn a_bad_filter_type_is_reported() {
        let mut raw = filtered(&sample_rgb(), 4, 3, 0);
        raw[13] = 7; // second row's filter byte
        let url = data_url(&png(4, 3, 2, &[&zlib(&raw)]));
        assert_eq!(decode_all(&url, 4, 3), Err((PngError::BadFilter(7), 1)));
    }

    #[test]
    fn idat_before_ihdr_is_malformed() {
        let mut bytes = SIGNATURE.to_vec();
        bytes.extend(chunk(b"IDAT", &[0]));
        assert_eq!(
            decode_all(&data_url(&bytes), 4, 3).map_err(|e| e.0),
            Err(PngError::Malformed)
        );
    }

    #[test]
    fn a_missing_or_wrong_prefix_or_signature_is_not_png() {
        let bytes = png(4, 3, 2, &[&zlib(&filtered(&sample_rgb(), 4, 3, 0))]);
        let url = data_url(&bytes);
        let bare = url.strip_prefix(DATA_URL_PREFIX).unwrap();
        assert_eq!(
            decode_all(bare, 4, 3).map_err(|e| e.0),
            Err(PngError::NotPng)
        );
        let webp = url.replace("image/png", "image/webp");
        assert_eq!(
            decode_all(&webp, 4, 3).map_err(|e| e.0),
            Err(PngError::NotPng)
        );
        let mut not_png = bytes.clone();
        not_png[1] = b'X';
        assert_eq!(
            decode_all(&data_url(&not_png), 4, 3).map_err(|e| e.0),
            Err(PngError::NotPng)
        );
        assert_eq!(
            decode_all(&format!("{DATA_URL_PREFIX}iVBO*w=="), 4, 3).map_err(|e| e.0),
            Err(PngError::InvalidBase64)
        );
    }
}
