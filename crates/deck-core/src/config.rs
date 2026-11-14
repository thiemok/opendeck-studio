//! The saved Companion host and its on-flash record (KTD14).
//!
//! Record layout, little-endian:
//!
//! | bytes | field |
//! |---|---|
//! | 0..4 | magic `ODCF` |
//! | 4 | format version (1) |
//! | 5 | host length |
//! | 6..8 | port |
//! | 8..71 | host, zero-padded |
//! | 71..75 | CRC-32 (IEEE) of bytes 0..71 |
//!
//! A blank (erased, `0xFF`) region, another version or a bad CRC all read as "no host".

pub const DEFAULT_PORT: u16 = 16622;
pub const MAX_HOST_LEN: usize = 63;
pub const RECORD_LEN: usize = 75;

const MAGIC: &[u8; 4] = b"ODCF";
const VERSION: u8 = 1;

/// A hostname or IPv4 address: 1–63 characters of ASCII letters, digits, `-` and `.`.
pub fn valid_host(host: &str) -> bool {
    (1..=MAX_HOST_LEN).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    host: [u8; MAX_HOST_LEN],
    host_len: u8,
    pub port: u16,
}

impl HostConfig {
    /// `None` unless `host` passes [`valid_host`].
    pub fn new(host: &str, port: u16) -> Option<Self> {
        if !valid_host(host) {
            return None;
        }
        let mut buf = [0u8; MAX_HOST_LEN];
        buf[..host.len()].copy_from_slice(host.as_bytes());
        Some(Self {
            host: buf,
            host_len: host.len() as u8,
            port,
        })
    }

    pub fn host(&self) -> &str {
        core::str::from_utf8(&self.host[..usize::from(self.host_len)]).unwrap_or("")
    }

    pub fn encode(&self) -> [u8; RECORD_LEN] {
        let mut rec = [0u8; RECORD_LEN];
        rec[0..4].copy_from_slice(MAGIC);
        rec[4] = VERSION;
        rec[5] = self.host_len;
        rec[6..8].copy_from_slice(&self.port.to_le_bytes());
        rec[8..8 + MAX_HOST_LEN].copy_from_slice(&self.host);
        let crc = crc32(&rec[..RECORD_LEN - 4]);
        rec[RECORD_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
        rec
    }

    pub fn decode(record: &[u8]) -> Option<Self> {
        let rec = record.get(..RECORD_LEN)?;
        if &rec[0..4] != MAGIC || rec[4] != VERSION {
            return None;
        }
        let stored_crc = u32::from_le_bytes(rec[RECORD_LEN - 4..].try_into().ok()?);
        if crc32(&rec[..RECORD_LEN - 4]) != stored_crc {
            return None;
        }
        let host = rec[8..8 + MAX_HOST_LEN].get(..usize::from(rec[5]))?;
        let port = u16::from_le_bytes([rec[6], rec[7]]);
        Self::new(core::str::from_utf8(host).ok()?, port)
    }
}

/// CRC-32 (IEEE 802.3, reflected), bitwise: the record is 71 bytes, so a table isn't worth it.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_round_trips() {
        let cfg = HostConfig::new("companion.lan", 16700).unwrap();
        let back = HostConfig::decode(&cfg.encode()).unwrap();
        assert_eq!((back.host(), back.port), ("companion.lan", 16700));
    }

    #[test]
    fn blank_flash_reads_as_no_host() {
        assert_eq!(HostConfig::decode(&[0xff; RECORD_LEN]), None);
    }

    #[test]
    fn an_unknown_version_reads_as_no_host() {
        let mut rec = HostConfig::new("10.0.0.2", DEFAULT_PORT).unwrap().encode();
        rec[4] = 2;
        assert_eq!(HostConfig::decode(&rec), None);
    }

    #[test]
    fn a_corrupted_record_reads_as_no_host() {
        let mut rec = HostConfig::new("10.0.0.2", DEFAULT_PORT).unwrap().encode();
        rec[10] ^= 0x01;
        assert_eq!(HostConfig::decode(&rec), None);
    }

    #[test]
    fn a_short_slice_reads_as_no_host() {
        let rec = HostConfig::new("10.0.0.2", DEFAULT_PORT).unwrap().encode();
        assert_eq!(HostConfig::decode(&rec[..RECORD_LEN - 1]), None);
    }

    #[test]
    fn crc_is_ieee_crc32() {
        // Pins the on-flash checksum to the standard check value.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn host_validation() {
        assert!(valid_host("192.168.1.10"));
        assert!(valid_host("companion-1.lan"));
        assert!(!valid_host(""));
        assert!(!valid_host("has space"));
        assert!(!valid_host("under_score"));
        assert!(!valid_host(&"a".repeat(64)));
        assert!(HostConfig::new("bad!", 1).is_none());
    }
}
