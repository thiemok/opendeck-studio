//! Standard base64 (RFC 4648, with padding), as Satellite uses for bitmaps and the manifest.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Length of the padded encoding of `n` bytes.
pub const fn encoded_len(n: usize) -> usize {
    n.div_ceil(3) * 4
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
    fn encode_refuses_a_short_buffer() {
        let mut out = [0u8; 3];
        assert_eq!(encode(b"foo", &mut out), None);
    }
}
