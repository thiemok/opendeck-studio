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

use crate::crc32::crc32;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Loads, saves and clears the record at the start of a NOR flash region (the
/// `deckcfg` partition on the device). Saving is erase-then-write of one sector:
/// if a write is interrupted, the record reads as "no host" and the operator
/// types the host again (KTD14).
pub mod store {
    use super::{HostConfig, RECORD_LEN};
    use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};

    /// The saved host, or `None` for a blank or unreadable record.
    pub fn load<F: ReadNorFlash>(flash: &mut F) -> Option<HostConfig> {
        let mut buf = [0u8; PADDED_LEN];
        flash.read(0, &mut buf[..padded_len(F::READ_SIZE)]).ok()?;
        HostConfig::decode(&buf)
    }

    pub fn save<F: NorFlash>(flash: &mut F, config: &HostConfig) -> Result<(), F::Error> {
        clear(flash)?;
        let mut buf = [0xffu8; PADDED_LEN];
        buf[..RECORD_LEN].copy_from_slice(&config.encode());
        flash.write(0, &buf[..padded_len(F::WRITE_SIZE)])
    }

    pub fn clear<F: NorFlash>(flash: &mut F) -> Result<(), F::Error> {
        flash.erase(0, F::ERASE_SIZE as u32)
    }

    /// Room for the record rounded up to any read/write granularity up to 32 bytes.
    const PADDED_LEN: usize = RECORD_LEN.div_ceil(32) * 32;

    /// The record length rounded up to the flash's access granularity.
    fn padded_len(granularity: usize) -> usize {
        RECORD_LEN.div_ceil(granularity) * granularity
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use core::convert::Infallible;
        use embedded_storage::nor_flash::ErrorType;

        /// RAM flash with NOR rules: erase sets 0xFF, writes can only clear bits.
        struct RamFlash(Vec<u8>);

        impl RamFlash {
            fn blank() -> Self {
                Self(vec![0xff; 2 * 4096])
            }
        }

        impl ErrorType for RamFlash {
            type Error = Infallible;
        }

        impl ReadNorFlash for RamFlash {
            const READ_SIZE: usize = 1;
            fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Infallible> {
                let o = offset as usize;
                bytes.copy_from_slice(&self.0[o..o + bytes.len()]);
                Ok(())
            }
            fn capacity(&self) -> usize {
                self.0.len()
            }
        }

        impl NorFlash for RamFlash {
            const WRITE_SIZE: usize = 4;
            const ERASE_SIZE: usize = 4096;
            fn erase(&mut self, from: u32, to: u32) -> Result<(), Infallible> {
                assert_eq!(from as usize % Self::ERASE_SIZE, 0);
                assert_eq!(to as usize % Self::ERASE_SIZE, 0);
                self.0[from as usize..to as usize].fill(0xff);
                Ok(())
            }
            fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Infallible> {
                assert_eq!(offset as usize % Self::WRITE_SIZE, 0);
                assert_eq!(bytes.len() % Self::WRITE_SIZE, 0);
                for (cell, &b) in self.0[offset as usize..].iter_mut().zip(bytes) {
                    *cell &= b;
                }
                Ok(())
            }
        }

        fn host(name: &str, port: u16) -> HostConfig {
            HostConfig::new(name, port).unwrap()
        }

        #[test]
        fn save_then_load_returns_the_same_host() {
            let mut flash = RamFlash::blank();
            save(&mut flash, &host("companion.lan", 16700)).unwrap();
            let back = load(&mut flash).unwrap();
            assert_eq!((back.host(), back.port), ("companion.lan", 16700));
        }

        #[test]
        fn clear_then_load_returns_none() {
            let mut flash = RamFlash::blank();
            save(&mut flash, &host("10.0.0.2", 16622)).unwrap();
            clear(&mut flash).unwrap();
            assert_eq!(load(&mut flash), None);
        }

        #[test]
        fn blank_flash_loads_as_none() {
            assert_eq!(load(&mut RamFlash::blank()), None);
        }

        #[test]
        fn a_second_save_replaces_the_first() {
            let mut flash = RamFlash::blank();
            save(&mut flash, &host("first.lan", 1)).unwrap();
            save(&mut flash, &host("second.lan", 2)).unwrap();
            let back = load(&mut flash).unwrap();
            assert_eq!((back.host(), back.port), ("second.lan", 2));
        }

        #[test]
        fn a_corrupted_record_loads_as_none() {
            let mut flash = RamFlash::blank();
            save(&mut flash, &host("10.0.0.2", 16622)).unwrap();
            flash.0[12] ^= 0x04;
            assert_eq!(load(&mut flash), None);
        }
    }
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
