//! The Satellite connection policy as a state machine (KTD12, KTD16, KTD18, KTD20).
//!
//! The firmware owns the socket and the clock; this owns the decisions. Every
//! input carries `now` in milliseconds, outgoing lines are appended to a
//! [`LineWriter`] for the firmware to send, and the return value says what the
//! firmware must do with the key or the socket.

use crate::line::{Base64Sink, LineWriter, Message, Status};

/// Native ScreenKey size; Companion renders the keys at exactly this size.
pub const KEY_SIZE: u16 = 128;
/// The encoder control's OLED, which Companion renders at exactly this size (KTD20).
pub const OLED_WIDTH: u16 = 128;
pub const OLED_HEIGHT: u16 = 64;
pub const PING_INTERVAL_MS: u64 = 2_000;
/// A healthy link always has a `PONG` within this window.
pub const SILENCE_TIMEOUT_MS: u64 = 5_000;
pub const BACKOFF_MIN_MS: u64 = 1_000;
pub const BACKOFF_MAX_MS: u64 = 5_000;

const PRODUCT_NAME: &str = "opendeck";
/// Two keys and the encoder in row 0; column 0 is left to Companion's page buttons (KTD12, KTD20).
const MANIFEST_JSON: &str = r#"{"stylePresets":{"default":{"bitmap":{"w":128,"h":128}},"encoder":{"bitmap":{"w":128,"h":64}}},"controls":{"0/1":{"row":0,"column":1,"stylePreset":"default"},"0/2":{"row":0,"column":2,"stylePreset":"default"},"0/3":{"row":0,"column":3,"stylePreset":"encoder"}}}"#;
/// A bench surface is rows of this many keys from `0/0` (THI-76).
pub const BENCH_COLUMNS: u8 = 8;
pub const MAX_BENCH_KEYS: u8 = 32;
/// `CONTROLID`s of the bench keys, row by row.
const BENCH_IDS: [&str; MAX_BENCH_KEYS as usize] = [
    "0/0", "0/1", "0/2", "0/3", "0/4", "0/5", "0/6", "0/7", //
    "1/0", "1/1", "1/2", "1/3", "1/4", "1/5", "1/6", "1/7", //
    "2/0", "2/1", "2/2", "2/3", "2/4", "2/5", "2/6", "2/7", //
    "3/0", "3/1", "3/2", "3/3", "3/4", "3/5", "3/6", "3/7",
];

/// A control the deck registers with Companion (KTD20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    KeyA,
    KeyB,
    /// Encoder rotation and push, shown on the OLED.
    Encoder,
    /// Key `n` of a bench surface, row by row from `0/0` (THI-76).
    Bench(u8),
}

impl Control {
    /// The `CONTROLID` Companion knows this control by.
    pub fn id(self) -> &'static str {
        match self {
            Control::KeyA => "0/1",
            Control::KeyB => "0/2",
            Control::Encoder => "0/3",
            Control::Bench(n) => BENCH_IDS.get(usize::from(n)).copied().unwrap_or(""),
        }
    }

    /// The control `id` names on a surface with this layout.
    pub fn from_id(id: &str, layout: Layout) -> Option<Self> {
        match layout.bench_keys() {
            0 => [Control::KeyA, Control::KeyB, Control::Encoder]
                .into_iter()
                .find(|c| c.id() == id),
            keys => BENCH_IDS[..usize::from(keys)]
                .iter()
                .position(|&b| b == id)
                .map(|n| Control::Bench(n as u8)),
        }
    }
}

/// Which controls the deck registers (THI-76).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// The PoC surface: two keys and the encoder.
    Normal,
    /// 16 keys (2×8) or 32 keys (4×8), no encoder.
    Bench16,
    Bench32,
}

impl Layout {
    /// How many bench keys the surface has; 0 for the normal surface.
    pub fn bench_keys(self) -> u8 {
        match self {
            Layout::Normal => 0,
            Layout::Bench16 => 16,
            Layout::Bench32 => 32,
        }
    }

    /// The control a physical key or the encoder push stands for on this surface. On a bench
    /// surface the two keys are bench keys `0/0` and `0/1` (page up and down on the bench
    /// pages), and the encoder isn't registered.
    pub fn physical(self, control: Control) -> Option<Control> {
        match (self, control) {
            (Layout::Normal, c) => Some(c),
            (_, Control::KeyA) => Some(Control::Bench(0)),
            (_, Control::KeyB) => Some(Control::Bench(1)),
            _ => None,
        }
    }
}

/// The bitmap format of a registration. The deck asks for `png` whenever `CAPS` offers it
/// (THI-76: about 4× faster page changes than raw `rgb`) and falls back to `rgb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Rgb,
    Png,
}

impl ImageFormat {
    pub fn name(self) -> &'static str {
        match self {
            ImageFormat::Rgb => "rgb",
            ImageFormat::Png => "png",
        }
    }
}

/// What the deck registers as, set from the serial console and applied at the next
/// registration (THI-76, KTD1). Not saved: a reboot returns to [`SurfaceConfig::NORMAL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceConfig {
    pub layout: Layout,
    /// On a bench surface, also draw every key's image on a panel, not only decode it.
    pub draw: bool,
}

impl SurfaceConfig {
    pub const NORMAL: Self = Self {
        layout: Layout::Normal,
        draw: false,
    };
}

/// `DEVICEID` and `SERIAL`, derived from the device MAC so Companion remembers
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
    /// Draw this base64 `rgb` bitmap on the control's display.
    Draw {
        control: Control,
        bitmap: &'a str,
    },
    /// `png` was asked for but `CAPS` doesn't offer it, so the deck registers for `rgb`.
    PngNotOffered,
    /// `KEYS-CLEAR`: blank every display.
    Blank,
    /// Close the socket (silence watchdog). Then call [`Session::closed`].
    Close,
    /// `ADD-DEVICE ERROR`, or a Companion without `NONSQUARE`: log the message, close the
    /// socket, then call [`Session::closed`].
    Rejected(&'a str),
    /// A line the session understood but chose to ignore, with the reason for the log.
    Ignored(&'static str),
}

pub struct Session {
    ids: DeviceIds,
    config: SurfaceConfig,
    /// The format the current registration asked for.
    format: ImageFormat,
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
            config: SurfaceConfig::NORMAL,
            format: ImageFormat::Rgb,
            state: LinkState::Disconnected,
            last_rx_ms: 0,
            last_ping_ms: 0,
            retry_at_ms: 0,
            backoff_ms: BACKOFF_MIN_MS,
        }
    }

    /// Replaces the surface config; it applies at the next registration, so the caller
    /// reconnects for it to take effect.
    pub fn set_config(&mut self, config: SurfaceConfig) {
        self.config = config;
    }

    pub fn config(&self) -> SurfaceConfig {
        self.config
    }

    /// The bitmap format the current registration asked Companion for.
    pub fn image_format(&self) -> ImageFormat {
        self.format
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
                // The 128×64 OLED preset needs non-square bitmaps (KTD20).
                if msg.get("NONSQUARE").and_then(|v| v.as_bool()) != Some(true) {
                    return Event::Rejected("Companion lacks NONSQUARE; it needs Companion 5.0+");
                }
                let png_offered = msg
                    .get("BITMAP_FORMATS")
                    .is_some_and(|v| v.raw().split(',').any(|f| f == "png"));
                self.format = if png_offered {
                    ImageFormat::Png
                } else {
                    ImageFormat::Rgb
                };
                self.write_add_device(out);
                if !png_offered {
                    Event::PngNotOffered
                } else {
                    Event::None
                }
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
            ("KEY-STATE", None) => {
                let control = msg
                    .get("CONTROLID")
                    .and_then(|id| Control::from_id(id.raw(), self.config.layout));
                match (control, bitmap(msg)) {
                    (Some(control), Some(bitmap)) => Event::Draw { control, bitmap },
                    (Some(_), None) => Event::Ignored("KEY-STATE without BITMAP"),
                    (None, _) => Event::Ignored("KEY-STATE for an unknown control"),
                }
            }
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

    /// A debounced press edge on a key or the encoder push. Sent only while registered and
    /// never queued (KTD18).
    pub fn key(&mut self, control: Control, pressed: bool, out: &mut LineWriter<'_>) {
        if self.is_registered() {
            out.command("KEY-PRESS")
                .quoted("DEVICEID", self.ids.device_id())
                .quoted("CONTROLID", control.id())
                .bare("PRESSED", if pressed { "1" } else { "0" })
                .end();
        }
    }

    /// One encoder detent. Companion runs a rotate action once per message whatever the step
    /// count, so each detent is its own `KEY-ROTATE` (KTD23). Dropped unless registered.
    pub fn rotate(&mut self, clockwise: bool, out: &mut LineWriter<'_>) {
        // A bench surface has no encoder control.
        if self.is_registered() && self.config.layout == Layout::Normal {
            out.command("KEY-ROTATE")
                .quoted("DEVICEID", self.ids.device_id())
                .quoted("CONTROLID", Control::Encoder.id())
                .bare("DIRECTION", if clockwise { "1" } else { "-1" })
                .end();
        }
    }

    fn write_add_device(&self, out: &mut LineWriter<'_>) {
        let layout = self.config.layout;
        out.command("ADD-DEVICE")
            .quoted("DEVICEID", self.ids.device_id())
            .quoted("SERIAL", self.ids.serial())
            .quoted("PRODUCT_NAME", PRODUCT_NAME)
            .quoted_base64("LAYOUT_MANIFEST", |sink| write_manifest(layout, sink));
        if self.format == ImageFormat::Png {
            out.quoted("BITMAP_FORMAT", "png");
        }
        out.end();
    }
}

/// The layout manifest's JSON. A bench surface's is built here rather than stored: 32 keys
/// make about 1.6 KB of it.
fn write_manifest(layout: Layout, sink: &mut Base64Sink<'_, '_>) {
    let keys = layout.bench_keys();
    if keys == 0 {
        sink.write(MANIFEST_JSON.as_bytes());
        return;
    }
    sink.write(br#"{"stylePresets":{"default":{"bitmap":{"w":128,"h":128}}},"controls":{"#);
    for n in 0..keys {
        let (row, column) = (b'0' + n / BENCH_COLUMNS, b'0' + n % BENCH_COLUMNS);
        if n > 0 {
            sink.write(b",");
        }
        sink.write(&[b'"', row, b'/', column]);
        sink.write(br#"":{"row":"#);
        sink.write(&[row]);
        sink.write(br#","column":"#);
        sink.write(&[column]);
        sink.write(br#","stylePreset":"default"}"#);
    }
    sink.write(b"}}");
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

/// The `BITMAP` value of a `KEY-STATE` line. The link uses it again on a line it held for a
/// draw (THI-89 KTD3), so both read the field the same way.
pub fn bitmap<'a>(msg: &Message<'a>) -> Option<&'a str> {
    msg.get("BITMAP").map(|v| v.raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; 6] = [0xa0, 0xb7, 0x65, 0x01, 0x02, 0xff];

    #[test]
    fn bitmap_reads_the_bitmap_field_of_a_key_state_line() {
        let msg = Message::parse(br#"KEY-STATE CONTROLID="0/1" BITMAP="AAAA/w==""#).unwrap();
        assert_eq!(bitmap(&msg), Some("AAAA/w=="));
        let msg = Message::parse(br#"KEY-STATE CONTROLID="0/1""#).unwrap();
        assert_eq!(bitmap(&msg), None);
    }

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

        fn key(&mut self, control: Control, pressed: bool) -> Vec<String> {
            let mut w = LineWriter::new(&mut self.buf);
            self.session.key(control, pressed, &mut w);
            sent(&w)
        }

        fn rotate(&mut self, clockwise: bool) -> Vec<String> {
            let mut w = LineWriter::new(&mut self.buf);
            self.session.rotate(clockwise, &mut w);
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
        assert_eq!(msg.get("BITMAP_FORMAT").map(|v| v.raw()), Some("png"));
        assert_eq!(h.session.state(), LinkState::Handshake);
    }

    #[test]
    fn manifest_has_two_keys_and_the_encoder_in_row_0() {
        let mut h = Harness::new();
        h.session.connected(0);
        let (_, sent) = h.line(0, "CAPS NONSQUARE=1");
        let json = decode_manifest(&sent[0]);
        assert!(
            json.contains(r#""default":{"bitmap":{"w":128,"h":128}}"#),
            "{json}"
        );
        assert!(
            json.contains(r#""encoder":{"bitmap":{"w":128,"h":64}}"#),
            "{json}"
        );
        assert!(!json.contains("text"), "no TEXT requested: {json}");
        for control in [
            r#""0/1":{"row":0,"column":1,"stylePreset":"default"}"#,
            r#""0/2":{"row":0,"column":2,"stylePreset":"default"}"#,
            r#""0/3":{"row":0,"column":3,"stylePreset":"encoder"}"#,
        ] {
            assert!(json.contains(control), "{control} missing: {json}");
        }
        assert_eq!(json.matches("\"row\"").count(), 3, "exactly three controls");
    }

    #[test]
    fn caps_without_nonsquare_is_rejected_and_sends_nothing() {
        for caps in ["CAPS SUBSCRIPTIONS=0", "CAPS NONSQUARE=0"] {
            let mut h = Harness::new();
            h.session.connected(0);
            let (event, sent) = h.line(0, caps);
            assert!(event.starts_with("Rejected("), "{caps}: {event}");
            assert!(event.contains("NONSQUARE"), "{caps}: {event}");
            assert!(sent.is_empty(), "{caps}: {sent:?}");
            h.session.closed(10);
            assert!(!h.session.connect_due(10), "backs off");
        }
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
        assert!(h.key(Control::KeyA, true).is_empty(), "handshake: dropped");
        h.register(0);
        assert_eq!(
            h.key(Control::KeyA, true),
            vec![r#"KEY-PRESS DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/1" PRESSED=1"#]
        );
        assert_eq!(
            h.key(Control::KeyA, false),
            vec![r#"KEY-PRESS DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/1" PRESSED=0"#]
        );
        h.session.closed(10);
        assert!(
            h.key(Control::KeyA, true).is_empty(),
            "disconnected: dropped, not queued"
        );
    }

    #[test]
    fn presses_carry_their_own_control_id() {
        let mut h = Harness::new();
        h.register(0);
        assert_eq!(
            h.key(Control::KeyB, true),
            vec![r#"KEY-PRESS DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/2" PRESSED=1"#]
        );
        assert_eq!(
            h.key(Control::Encoder, false),
            vec![r#"KEY-PRESS DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/3" PRESSED=0"#]
        );
    }

    #[test]
    fn each_detent_is_one_key_rotate_on_the_encoder() {
        // Covers AE5 at protocol level.
        let mut h = Harness::new();
        h.register(0);
        assert_eq!(
            h.rotate(true),
            vec![r#"KEY-ROTATE DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/3" DIRECTION=1"#]
        );
        assert_eq!(
            h.rotate(false),
            vec![r#"KEY-ROTATE DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/3" DIRECTION=-1"#]
        );
        let mut w = LineWriter::new(&mut h.buf);
        for _ in 0..5 {
            h.session.rotate(true, &mut w);
        }
        assert_eq!(sent(&w).len(), 5);
    }

    #[test]
    fn rotations_are_dropped_unless_registered() {
        let mut h = Harness::new();
        assert!(h.rotate(true).is_empty(), "disconnected");
        h.session.connected(0);
        assert!(h.rotate(true).is_empty(), "handshake");
    }

    #[test]
    fn key_state_draws_on_the_control_it_names() {
        let mut h = Harness::new();
        h.register(0);
        for (id, control) in [("0/1", "KeyA"), ("0/2", "KeyB"), ("0/3", "Encoder")] {
            let line = format!(
                r#"KEY-STATE DEVICEID="d" CONTROLID="{id}" PRESSED=0 TYPE="BUTTON" BITMAP="AAAA""#
            );
            let (event, _) = h.line(1, &line);
            assert_eq!(
                event,
                format!(r#"Draw {{ control: {control}, bitmap: "AAAA" }}"#)
            );
        }
    }

    #[test]
    fn key_state_for_an_unknown_control_is_ignored() {
        let mut h = Harness::new();
        h.register(0);
        let (event, _) = h.line(1, r#"KEY-STATE DEVICEID="d" CONTROLID="0/4" BITMAP="AAAA""#);
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

    fn bench(layout: Layout) -> Harness {
        let mut h = Harness::new();
        h.session.set_config(SurfaceConfig {
            layout,
            draw: false,
        });
        h
    }

    const CAPS_ALL: &str =
        r#"CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ROTARY_AMOUNT=1"#;

    /// The `ADD-DEVICE` line sent in answer to `caps`, with the event it raised.
    fn add_device(h: &mut Harness, caps: &str) -> (String, String) {
        h.session.connected(0);
        let (event, sent) = h.line(0, caps);
        assert_eq!(sent.len(), 1, "{sent:?}");
        (event, sent[0].clone())
    }

    #[test]
    fn bench_manifests_have_rows_of_eight_keys_and_no_encoder() {
        for (layout, keys, last) in [(Layout::Bench16, 16, "1/7"), (Layout::Bench32, 32, "3/7")] {
            let mut h = bench(layout);
            let (_, add) = add_device(&mut h, CAPS_ALL);
            let json = decode_manifest(&add);
            assert_eq!(json.matches("\"row\"").count(), keys, "{json}");
            assert!(!json.contains("encoder"), "{json}");
            assert!(
                json.starts_with(r#"{"stylePresets":{"default":{"bitmap":{"w":128,"h":128}}},"controls":{"0/0":{"row":0,"column":0,"stylePreset":"default"},"#),
                "{json}"
            );
            let row = last.as_bytes()[0] - b'0';
            assert!(
                json.ends_with(&format!(
                    r#""{last}":{{"row":{row},"column":7,"stylePreset":"default"}}}}}}"#
                )),
                "{json}"
            );
        }
    }

    #[test]
    fn a_32_key_add_device_fits_a_4_kb_writer() {
        let mut h = bench(Layout::Bench32);
        h.session.connected(0);
        let mut buf = [0u8; 4096];
        let mut w = LineWriter::new(&mut buf);
        h.session
            .line(0, &Message::parse(CAPS_ALL.as_bytes()).unwrap(), &mut w);
        let len = w.bytes().expect("fits").len();
        assert!(len > 1024, "outgrows the old 1 KB buffer: {len}");
    }

    #[test]
    fn png_is_requested_whenever_caps_offers_it() {
        let mut h = bench(Layout::Normal);
        let (event, add) = add_device(&mut h, CAPS_ALL);
        assert_eq!(event, "None");
        let msg = Message::parse(add.as_bytes()).unwrap();
        assert_eq!(msg.get("BITMAP_FORMAT").map(|v| v.raw()), Some("png"));
        assert_eq!(h.session.image_format(), ImageFormat::Png);

        let mut h = bench(Layout::Normal);
        for caps in [
            r#"CAPS NONSQUARE=1 BITMAP_FORMATS="rgb""#,
            "CAPS NONSQUARE=1",
        ] {
            let (event, add) = add_device(&mut h, caps);
            assert_eq!(event, "PngNotOffered", "{caps}");
            assert_eq!(
                Message::parse(add.as_bytes()).unwrap().get("BITMAP_FORMAT"),
                None,
                "{caps}"
            );
            assert_eq!(h.session.image_format(), ImageFormat::Rgb, "{caps}");
            h.session.closed(0);
        }
    }

    #[test]
    fn bench_key_state_maps_to_its_index_within_the_layout() {
        let line = r#"KEY-STATE DEVICEID="d" CONTROLID="2/5" BITMAP="AAAA""#;
        let mut h = bench(Layout::Bench32);
        h.register(0);
        assert_eq!(
            h.line(1, line).0,
            r#"Draw { control: Bench(21), bitmap: "AAAA" }"#
        );
        let mut h = bench(Layout::Bench16);
        h.register(0);
        assert!(
            h.line(1, line).0.starts_with("Ignored"),
            "2/5 is not on a 16-key surface"
        );
        // The PoC's IDs mean other controls on a bench surface.
        assert_eq!(
            h.line(1, r#"KEY-STATE DEVICEID="d" CONTROLID="0/3" BITMAP="AAAA""#)
                .0,
            r#"Draw { control: Bench(3), bitmap: "AAAA" }"#
        );
    }

    #[test]
    fn bench_presses_use_their_own_control_id_and_rotation_sends_nothing() {
        let mut h = bench(Layout::Bench32);
        h.register(0);
        let key = Layout::Bench32.physical(Control::KeyB).unwrap();
        assert_eq!(
            h.key(key, true),
            vec![r#"KEY-PRESS DEVICEID="opendeck-a0b7650102ff" CONTROLID="0/1" PRESSED=1"#]
        );
        assert!(h.rotate(true).is_empty());
        assert_eq!(Layout::Bench32.physical(Control::Encoder), None);
        assert_eq!(Layout::Normal.physical(Control::KeyB), Some(Control::KeyB));
    }

    #[test]
    fn a_config_change_applies_at_the_next_registration() {
        let mut h = Harness::new();
        h.register(0);
        h.session.set_config(SurfaceConfig {
            layout: Layout::Bench16,
            draw: true,
        });
        h.session.closed(10);
        let (_, add) = add_device(&mut h, CAPS_ALL);
        assert_eq!(decode_manifest(&add).matches("\"row\"").count(), 16);
    }
}
