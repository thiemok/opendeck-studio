//! Standard base64 (RFC 4648, with padding), as Satellite uses for bitmaps and the manifest.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Length of the padded encoding of `n` bytes.
pub const fn encoded_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

/// Marks a byte outside the alphabet in [`SEXTETS`].
pub(crate) const INVALID: u8 = 0xff;

/// Base64 character to its 6-bit value, or [`INVALID`]. A table because the decode
/// loops run for every character of a 65 KB line.
pub(crate) const SEXTETS: [u8; 256] = {
    let mut table = [INVALID; 256];
    let mut i = 0;
    while i < ALPHABET.len() {
        table[ALPHABET[i] as usize] = i as u8;
        i += 1;
    }
    table
};

/// Decodes whole quads of `text` into `out`, returning the number of bytes written.
///
/// `text.len()` must be a multiple of 4 and `out` at least `text.len() / 4 * 3` long.
/// Padding is accepted only in the last quad, and only when `last` says no text follows.
/// `None` means a character outside the alphabet, misplaced padding or a short `out`.
pub fn decode_quads(text: &[u8], out: &mut [u8], last: bool) -> Option<usize> {
    if !text.len().is_multiple_of(4) || out.len() < text.len() / 4 * 3 {
        return None;
    }
    let padding = if last {
        text.iter()
            .rev()
            .take(2)
            .take_while(|&&c| c == b'=')
            .count()
    } else {
        0
    };
    let mut written = 0;
    let quads = text.len() / 4;
    for (i, quad) in text.as_chunks::<4>().0.iter().enumerate() {
        let pad = if i + 1 == quads { padding } else { 0 };
        let mut n = 0u32;
        for (j, &c) in quad.iter().enumerate() {
            let sextet = if j >= 4 - pad {
                0
            } else {
                SEXTETS[usize::from(c)]
            };
            if sextet == INVALID {
                return None;
            }
            n = (n << 6) | u32::from(sextet);
        }
        let bytes = n.to_be_bytes();
        let len = 3 - pad;
        out[written..written + len].copy_from_slice(&bytes[1..=len]);
        written += len;
    }
    Some(written)
}

/// Encodes `input` into `out`, returning the encoded text, or `None` if `out` is too small.
pub fn encode<'o>(input: &[u8], out: &'o mut [u8]) -> Option<&'o str> {
    let len = encoded_len(input.len());
    let out = out.get_mut(..len)?;
    for (chunk, dst) in input.chunks(3).zip(out.chunks_mut(4)) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, d) in dst.iter_mut().enumerate() {
            *d = if i <= chunk.len() {
                ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize]
            } else {
                b'='
            };
        }
    }
    core::str::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_rfc4648_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            let mut out = [0u8; 16];
            assert_eq!(encode(input.as_bytes(), &mut out), Some(want), "{input}");
        }
    }

    #[test]
    fn decode_quads_inverts_encode() {
        for want in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let mut text = [0u8; 16];
            let text = encode(want.as_bytes(), &mut text).unwrap();
            let mut out = [0u8; 12];
            let n = decode_quads(text.as_bytes(), &mut out, true).unwrap();
            assert_eq!(&out[..n], want.as_bytes(), "{text}");
        }
    }

    #[test]
    fn decode_quads_rejects_bad_input() {
        let mut out = [0u8; 12];
        assert_eq!(
            decode_quads(b"Zm9", &mut out, true),
            None,
            "not whole quads"
        );
        assert_eq!(
            decode_quads(b"Zm*v", &mut out, true),
            None,
            "outside the alphabet"
        );
        assert_eq!(
            decode_quads(b"Zg==Zm9v", &mut out, true),
            None,
            "padding mid-text"
        );
        assert_eq!(
            decode_quads(b"Zg==", &mut out, false),
            None,
            "padding before the end"
        );
        assert_eq!(
            decode_quads(b"Zm9vYmFy", &mut out[..5], true),
            None,
            "short out"
        );
    }

    #[test]
    fn encode_refuses_a_short_buffer() {
        let mut out = [0u8; 3];
        assert_eq!(encode(b"foo", &mut out), None);
    }
}
