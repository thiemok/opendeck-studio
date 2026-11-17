//! CRC-32 (IEEE 802.3, reflected), as the config record and PNG chunks use it.
//!
//! Table-driven, because a PNG key image runs several kilobytes through it per line.

const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// A running CRC-32 over data fed in pieces.
#[derive(Debug, Clone, Copy)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub const fn new() -> Self {
        Self(!0)
    }

    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.0 = TABLE[usize::from((self.0 as u8) ^ byte)] ^ (self.0 >> 8);
        }
    }

    pub fn finish(&self) -> u32 {
        !self.0
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(data);
    crc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value_and_empty_input() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn pieces_give_the_same_crc_as_one_buffer() {
        let mut crc = Crc32::new();
        crc.update(b"IEN");
        crc.update(b"D");
        // The CRC every PNG IEND chunk carries.
        assert_eq!(crc.finish(), 0xAE42_6082);
    }
}
