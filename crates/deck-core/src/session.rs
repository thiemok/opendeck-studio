//! The Satellite connection policy as a state machine (KTD12, KTD16, KTD18).
//!
//! The firmware owns the socket and the clock; this owns the decisions. Every
//! input carries `now` in milliseconds, outgoing lines are appended to a
//! [`LineWriter`] for the firmware to send, and the return value says what the
//! firmware must do with the key or the socket.

use crate::base64;
use crate::line::{LineWriter, Message, Status};

/// The one LCD key P2 registers: row 0, column 1 (KTD12).
pub const KEY_CONTROL: &str = "0/1";
/// Native ScreenKey size; Companion renders the key at exactly this size.
pub const KEY_SIZE: u16 = 128;
pub const PING_INTERVAL_MS: u64 = 2_000;
/// A healthy link always has a `PONG` within this window.
pub const SILENCE_TIMEOUT_MS: u64 = 5_000;
pub const BACKOFF_MIN_MS: u64 = 1_000;
pub const BACKOFF_MAX_MS: u64 = 5_000;

const PRODUCT_NAME: &str = "opendeck";
const MANIFEST_JSON: &str = r#"{"stylePresets":{"default":{"bitmap":{"w":128,"h":128}}},"controls":{"0/1":{"row":0,"column":1,"stylePreset":"default"}}}"#;

/// `DEVICEID` and `SERIAL`, derived from the Wi-Fi MAC so Companion remembers
/// the surface across reboots.
#[derive(Debug, Clone)]
pub struct DeviceIds {
    device_id: [u8; 21],
    serial: [u8; 21],
}

impl DeviceIds {
    pub fn from_mac(mac: [u8; 6]) -> Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut device_id = *b"opendeck-000000000000";
        for (i, byte) in mac.iter().enumerate() {
            device_id[9 + 2 * i] = HEX[usize::from(byte >> 4)];
            device_id[10 + 2 * i] = HEX[usize::from(byte & 0xf)];
        }
        let mut serial = device_id;
        serial[8] = b':';
        Self { device_id, serial }
    }

    pub fn device_id(&self) -> &str {
        core::str::from_utf8(&self.device_id).unwrap_or("")
    }

    pub fn serial(&self) -> &str {
        core::str::from_utf8(&self.serial).unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Disconnected,
    Handshake,
    Registered,
}

/// What the firmware must do after an input.
#[derive(Debug, PartialEq, Eq)]
pub enum Event<'a> {
    None,
    /// `ADD-DEVICE OK`: the key now shows Companion's images.
    Registered,
    /// Draw this base64 `rgb` bitmap on the key.
    Draw(&'a str),
    /// `KEYS-CLEAR`: blank the key.
    Blank,
    /// Close the socket (silence watchdog). Then call [`Session::closed`].
    Close,
    /// `ADD-DEVICE ERROR`: log the message, close the socket, then call [`Session::closed`].
    Rejected(&'a str),
    /// A line the session understood but chose to ignore, with the reason for the log.
    Ignored(&'static str),
}

pub struct Session {
    ids: DeviceIds,
    state: LinkState,
    last_rx_ms: u64,
    last_ping_ms: u64,
    retry_at_ms: u64,
    backoff_ms: u64,
}

impl Session {
    pub fn new(ids: DeviceIds) -> Self {
        Self {
            ids,
            state: LinkState::Disconnected,
            last_rx_ms: 0,
            last_ping_ms: 0,
            retry_at_ms: 0,
            backoff_ms: BACKOFF_MIN_MS,
        }
    }

    pub fn state(&self) -> LinkState {
        self.state
    }

    pub fn is_registered(&self) -> bool {
        self.state == LinkState::Registered
    }

    /// Whether a new connection attempt may start now.
    pub fn connect_due(&self, now_ms: u64) -> bool {
        self.state == LinkState::Disconnected && now_ms >= self.retry_at_ms
    }

    /// The TCP connection opened.
    pub fn connected(&mut self, now_ms: u64) {
        self.state = LinkState::Handshake;
        self.last_rx_ms = now_ms;
        self.last_ping_ms = now_ms;
    }

    /// The connection closed or failed to open; schedules the next attempt.
    pub fn closed(&mut self, now_ms: u64) {
        self.state = LinkState::Disconnected;
        self.retry_at_ms = now_ms + self.backoff_ms;
        self.backoff_ms = (self.backoff_ms * 2).min(BACKOFF_MAX_MS);
    }

    /// The host changed: the next attempt may start immediately.
    pub fn reconnect_now(&mut self) {
        self.retry_at_ms = 0;
        self.backoff_ms = BACKOFF_MIN_MS;
    }

    /// One line from Companion.
    pub fn line<'a>(
        &mut self,
        now_ms: u64,
        msg: &Message<'a>,
        out: &mut LineWriter<'_>,
    ) -> Event<'a> {
        self.last_rx_ms = now_ms;
        match (msg.command, msg.status) {
            ("CAPS", None) if self.state == LinkState::Handshake => {
                self.write_add_device(out);
                Event::None
            }
            ("ADD-DEVICE", Some(Status::Ok)) => {
                self.state = LinkState::Registered;
                self.backoff_ms = BACKOFF_MIN_MS;
                Event::Registered
            }
            ("ADD-DEVICE", Some(Status::Error)) => {
                Event::Rejected(msg.get("MESSAGE").map_or("", |v| v.raw()))
            }
            ("PING", None) => {
                out.command("PONG").raw(msg.rest()).end();
                Event::None
            }
            ("KEY-STATE", None) => match (msg.get("CONTROLID"), msg.get("BITMAP")) {
                (Some(id), Some(bitmap)) if id.raw() == KEY_CONTROL => Event::Draw(bitmap.raw()),
                (Some(id), _) if id.raw() == KEY_CONTROL => {
                    Event::Ignored("KEY-STATE without BITMAP")
                }
                _ => Event::Ignored("KEY-STATE for an unknown control"),
            },
            ("KEYS-CLEAR", None) => Event::Blank,
            _ => Event::None,
        }
    }

    /// Periodic tick: sends `PING` and runs the silence watchdog.
    pub fn tick(&mut self, now_ms: u64, out: &mut LineWriter<'_>) -> Event<'static> {
        if self.state == LinkState::Disconnected {
            return Event::None;
        }
        if now_ms.saturating_sub(self.last_rx_ms) >= SILENCE_TIMEOUT_MS {
            return Event::Close;
        }
        if self.state == LinkState::Registered
            && now_ms.saturating_sub(self.last_ping_ms) >= PING_INTERVAL_MS
        {
            self.last_ping_ms = now_ms;
            let mut payload = [0u8; 20];
            out.command("PING").raw(decimal(now_ms, &mut payload)).end();
        }
        Event::None
    }

    /// A debounced key edge. Sent only while registered and never queued (KTD18).
    pub fn key(&mut self, pressed: bool, out: &mut LineWriter<'_>) {
        if self.is_registered() {
            out.command("KEY-PRESS")
                .quoted("DEVICEID", self.ids.device_id())
                .quoted("CONTROLID", KEY_CONTROL)
                .bare("PRESSED", if pressed { "1" } else { "0" })
                .end();
        }
    }

    fn write_add_device(&self, out: &mut LineWriter<'_>) {
        let mut manifest = [0u8; base64::encoded_len(MANIFEST_JSON.len())];
        let manifest = base64::encode(MANIFEST_JSON.as_bytes(), &mut manifest).unwrap_or("");
        out.command("ADD-DEVICE")
            .quoted("DEVICEID", self.ids.device_id())
            .quoted("SERIAL", self.ids.serial())
            .quoted("PRODUCT_NAME", PRODUCT_NAME)
            .quoted("LAYOUT_MANIFEST", manifest)
            .end();
    }
}

/// Formats `n` in decimal into `buf`.
fn decimal(mut n: u64, buf: &mut [u8; 20]) -> &str {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("0")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; 6] = [0xa0, 0xb7, 0x65, 0x01, 0x02, 0xff];

    struct Harness {
        session: Session,
        buf: Vec<u8>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                session: Session::new(DeviceIds::from_mac(MAC)),
                buf: vec![0u8; 4096],
            }
        }

        /// Feeds one line; returns the event (as text) and the lines sent.
        fn line(&mut self, now: u64, line: &str) -> (String, Vec<String>) {
            let mut w = LineWriter::new(&mut self.buf);
            let msg = Message::parse(line.as_bytes()).unwrap();
            let event = format!("{:?}", self.session.line(now, &msg, &mut w));
            (event, sent(&w))
        }

        fn tick(&mut self, now: u64) -> (String, Vec<String>) {
            let mut w = LineWriter::new(&mut self.buf);
            let event = format!("{:?}", self.session.tick(now, &mut w));
            (event, sent(&w))
        }

        fn key(&mut self, pressed: bool) -> Vec<String> {
            let mut w = LineWriter::new(&mut self.buf);
            self.session.key(pressed, &mut w);
            sent(&w)
        }

        fn register(&mut self, now: u64) {
            self.session.connected(now);
            self.line(now, r#"BEGIN CompanionVersion="5.1.3" ApiVersion="1.14.0""#);
            self.line(
                now,
                r#"CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ROTARY_AMOUNT=1"#,
            );
            let (event, _) = self.line(now, r#"ADD-DEVICE OK DEVICEID="opendeck-a0b7650102ff""#);
            assert_eq!(event, "Registered");
        }
    }

    fn sent(w: &LineWriter<'_>) -> Vec<String> {
        let text = core::str::from_utf8(w.bytes().expect("outbox overflow")).unwrap();
        text.lines().map(str::to_owned).collect()
    }

    fn decode_manifest(add_device: &str) -> String {
        let msg = Message::parse(add_device.as_bytes()).unwrap();
        base64_decode(msg.get("LAYOUT_MANIFEST").unwrap().raw())
    }

    fn base64_decode(s: &str) -> String {
        // Test-only decoder: invert the encoder over the alphabet.
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bits = 0u32;
        let mut n = 0;
        let mut out = Vec::new();
        for c in s.bytes().filter(|&c| c != b'=') {
            bits = (bits << 6) | alphabet.iter().position(|&a| a == c).unwrap() as u32;
            n += 6;
            if n >= 8 {
                n -= 8;
                out.push((bits >> n) as u8);
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn device_ids_come_from_the_mac() {
        let ids = DeviceIds::from_mac(MAC);
        assert_eq!(ids.device_id(), "opendeck-a0b7650102ff");
        assert_eq!(ids.serial(), "opendeck:a0b7650102ff");
    }

    #[test]
    fn caps_triggers_exactly_one_add_device() {
        let mut h = Harness::new();
        h.session.connected(0);
        let (_, begin_sent) = h.line(0, r#"BEGIN CompanionVersion="5.1.3" ApiVersion="1.14.0""#);
        assert!(begin_sent.is_empty());
        let (_, sent) = h.line(
            10,
            r#"CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ROTARY_AMOUNT=1"#,
        );
        assert_eq!(sent.len(), 1);
        let add = &sent[0];
        let msg = Message::parse(add.as_bytes()).unwrap();
        assert_eq!(msg.command, "ADD-DEVICE");
        assert_eq!(msg.get("DEVICEID").unwrap().raw(), "opendeck-a0b7650102ff");
        assert_eq!(msg.get("SERIAL").unwrap().raw(), "opendeck:a0b7650102ff");
        assert_eq!(msg.get("BITMAP_FORMAT"), None);
        assert_eq!(h.session.state(), LinkState::Handshake);
    }

    #[test]
    fn manifest_has_the_default_preset_and_one_key_at_row0_col1() {
        let mut h = Harness::new();
        h.session.connected(0);
        let (_, sent) = h.line(0, "CAPS NONSQUARE=1");
        let json = decode_manifest(&sent[0]);
        assert!(
            json.contains(r#""default":{"bitmap":{"w":128,"h":128}}"#),
            "{json}"
        );
        assert!(
            json.contains(r#""0/1":{"row":0,"column":1,"stylePreset":"default"}"#),
            "{json}"
        );
        assert_eq!(json.matches("\"row\"").count(), 1, "exactly one control");
    }

    #[test]
    fn server_ping_gets_a_pong_with_the_same_payload() {
        let mut h = Harness::new();
        h.register(0);
        let (_, sent) = h.line(100, "PING abc");
        assert_eq!(sent, vec!["PONG abc"]);
    }

    #[test]
    fn registered_link_pings_every_two_seconds() {
        let mut h = Harness::new();
        h.register(0);
        assert!(h.tick(1_999).1.is_empty());
        let (_, sent) = h.tick(2_000);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].starts_with("PING "), "{sent:?}");
        h.line(2_050, "PONG x");
        assert!(h.tick(3_000).1.is_empty());
        assert_eq!(h.tick(4_000).1.len(), 1);
    }

    #[test]
    fn five_seconds_of_silence_closes_the_link() {
        // Covers AE1: a half-open link is detected without a FIN.
        let mut h = Harness::new();
        h.register(0);
        assert_eq!(h.tick(4_999).0, "None");
        assert_eq!(h.tick(5_000).0, "Close");
    }

    #[test]
    fn handshake_silence_also_closes() {
        let mut h = Harness::new();
        h.session.connected(0);
        assert_eq!(h.tick(5_000).0, "Close");
    }

    #[test]
    fn backoff_doubles_to_a_five_second_cap_and_resets_on_registration() {
        // Covers AE1: recovery is automatic and bounded.
        let mut h = Harness::new();
        let mut now = 0;
        let mut delays = Vec::new();
        for _ in 0..5 {
            h.session.closed(now);
            let mut t = now;
            while !h.session.connect_due(t) {
                t += 100;
            }
            delays.push(t - now);
            now = t;
            h.session.connected(now);
        }
        assert_eq!(delays, vec![1_000, 2_000, 4_000, 5_000, 5_000]);

        h.line(now, "CAPS NONSQUARE=1");
        h.line(now, r#"ADD-DEVICE OK DEVICEID="opendeck-a0b7650102ff""#);
        h.session.closed(now);
        assert!(!h.session.connect_due(now + 999));
        assert!(h.session.connect_due(now + 1_000));
    }

    #[test]
    fn add_device_error_is_rejected_and_backs_off() {
        let mut h = Harness::new();
        h.session.connected(0);
        h.line(0, "CAPS NONSQUARE=1");
        let (event, _) = h.line(
            10,
            r#"ADD-DEVICE ERROR DEVICEID="x" MESSAGE="Device exists elsewhere""#,
        );
        assert_eq!(event, r#"Rejected("Device exists elsewhere")"#);
        h.session.closed(10);
        assert!(!h.session.connect_due(10));
    }

    #[test]
    fn host_change_allows_an_immediate_reconnect() {
        let mut h = Harness::new();
        h.session.closed(0);
        h.session.closed(1_000);
        assert!(!h.session.connect_due(1_000));
        h.session.reconnect_now();
        assert!(h.session.connect_due(1_000));
    }

    #[test]
    fn key_edges_are_sent_only_while_registered() {
        let mut h = Harness::new();
        h.session.connected(0);
        assert!(h.key(true).is_empty(), "handshake: dropped");
        h.register(0);
        assert_eq!(
            h.key(true),
            vec![r#"KEY-PRESS DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/1" PRESSED=1"#]
        );
        assert_eq!(
            h.key(false),
            vec![r#"KEY-PRESS DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/1" PRESSED=0"#]
        );
        h.session.closed(10);
        assert!(h.key(true).is_empty(), "disconnected: dropped, not queued");
    }

    #[test]
    fn key_state_for_our_key_draws_its_bitmap() {
        let mut h = Harness::new();
        h.register(0);
        let (event, _) = h.line(1, r#"KEY-STATE DEVICEID="d" CONTROLID="0/1" LOCATION="1/0/1" PRESSED=0 TYPE="BUTTON" BITMAP="AAAA""#);
        assert_eq!(event, r#"Draw("AAAA")"#);
    }

    #[test]
    fn key_state_for_another_control_is_ignored() {
        let mut h = Harness::new();
        h.register(0);
        let (event, _) = h.line(1, r#"KEY-STATE DEVICEID="d" CONTROLID="0/2" BITMAP="AAAA""#);
        assert!(event.starts_with("Ignored"), "{event}");
    }

    #[test]
    fn keys_clear_blanks_the_key() {
        let mut h = Harness::new();
        h.register(0);
        assert_eq!(h.line(1, r#"KEYS-CLEAR DEVICEID="d""#).0, "Blank");
    }

    #[test]
    fn acknowledgements_and_unknown_lines_change_nothing() {
        let mut h = Harness::new();
        h.register(0);
        for line in [
            r#"KEY-PRESS OK DEVICEID="d""#,
            "FUTURE-THING A=1",
            r#"BRIGHTNESS DEVICEID="d" VALUE=100"#,
        ] {
            let (event, sent) = h.line(1, line);
            assert_eq!(event, "None", "{line}");
            assert!(sent.is_empty(), "{line}");
        }
        assert!(h.session.is_registered());
    }

    #[test]
    fn any_line_resets_the_silence_watchdog() {
        let mut h = Harness::new();
        h.register(0);
        h.line(4_000, r#"KEY-PRESS OK DEVICEID="d""#);
        assert_eq!(h.tick(8_999).0, "None");
        assert_eq!(h.tick(9_000).0, "Close");
    }
}
