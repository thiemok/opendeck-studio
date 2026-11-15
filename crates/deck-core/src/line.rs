//! Satellite line framing, parsing and formatting.
//!
//! Wire rules (see `docs/notes/satellite-protocol-firmware-notes.md#messages`):
//! newline-terminated ASCII lines, `COMMAND [OK|ERROR] KEY=value KEY="quoted" …`,
//! a backslash escapes the next character inside quoted values, and a key is split
//! from its value on the first `=` only (base64 padding contains `=`).

/// What [`LineFramer::feed`] reports for each completed line.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame<'a> {
    /// One complete line, without its `\n` (and without a trailing `\r`).
    Line(&'a [u8]),
    /// A line was longer than the buffer and was skipped up to its newline.
    Overflow,
}

/// Collects bytes into a caller-owned buffer and yields complete lines.
pub struct LineFramer<'b> {
    buf: &'b mut [u8],
    len: usize,
    discarding: bool,
}

impl<'b> LineFramer<'b> {
    pub fn new(buf: &'b mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            discarding: false,
        }
    }

    /// Feeds received bytes, calling `on_frame` for every line they complete.
    pub fn feed(&mut self, mut data: &[u8], mut on_frame: impl FnMut(Frame<'_>)) {
        while !data.is_empty() {
            let newline = data.iter().position(|&b| b == b'\n');
            let (chunk, complete) = match newline {
                Some(i) => (&data[..i], true),
                None => (data, false),
            };
            if !self.discarding {
                if self.len + chunk.len() > self.buf.len() {
                    self.discarding = true;
                    self.len = 0;
                } else {
                    self.buf[self.len..self.len + chunk.len()].copy_from_slice(chunk);
                    self.len += chunk.len();
                }
            }
            if !complete {
                return;
            }
            if self.discarding {
                on_frame(Frame::Overflow);
            } else {
                let mut line = &self.buf[..self.len];
                if let [rest @ .., b'\r'] = line {
                    line = rest;
                }
                on_frame(Frame::Line(line));
            }
            self.reset();
            data = &data[chunk.len() + 1..];
        }
    }

    /// Whether no partial line is buffered, i.e. the next byte starts a new line.
    pub fn at_line_start(&self) -> bool {
        self.len == 0 && !self.discarding
    }

    /// Drops any partial line, e.g. after the connection closes.
    pub fn reset(&mut self) {
        self.len = 0;
        self.discarding = false;
    }
}

/// `OK` or `ERROR` right after the command name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Error,
}

/// A parameter value as it appears on the wire, without surrounding quotes.
/// Escapes are left in place; [`Value::has_escapes`] tells whether any exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Value<'a> {
    raw: &'a str,
}

impl<'a> Value<'a> {
    pub fn raw(&self) -> &'a str {
        self.raw
    }

    pub fn has_escapes(&self) -> bool {
        self.raw.contains('\\')
    }

    /// `1`/`true` and `0`/`false`, as Companion writes booleans.
    pub fn as_bool(&self) -> Option<bool> {
        match self.raw {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        }
    }
}

/// One parsed line. Parameters are found lazily, so a 65 KB `BITMAP` is never copied.
#[derive(Debug, Clone, Copy)]
pub struct Message<'a> {
    pub command: &'a str,
    pub status: Option<Status>,
    params: &'a str,
}

/// Why a line could not be parsed.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    NotUtf8,
}

impl<'a> Message<'a> {
    pub fn parse(line: &'a [u8]) -> Result<Self, ParseError> {
        let line = core::str::from_utf8(line).map_err(|_| ParseError::NotUtf8)?;
        let line = line.trim_start_matches(' ');
        let (command, rest) = split_token(line);
        if command.is_empty() {
            return Err(ParseError::Empty);
        }
        let (status_token, after_status) = split_token(rest);
        let (status, params) = match status_token {
            "OK" => (Some(Status::Ok), after_status),
            "ERROR" => (Some(Status::Error), after_status),
            _ => (None, rest),
        };
        Ok(Self {
            command,
            status,
            params: params.trim_end_matches(' '),
        })
    }

    /// Everything after the command (and status), e.g. a `PING` payload.
    pub fn rest(&self) -> &'a str {
        self.params
    }

    /// The value of `key`, if present.
    pub fn get(&self, key: &str) -> Option<Value<'a>> {
        Params { rest: self.params }
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }
}

/// Splits off the first space-separated token.
fn split_token(s: &str) -> (&str, &str) {
    match s.find(' ') {
        Some(i) => (&s[..i], s[i + 1..].trim_start_matches(' ')),
        None => (s, ""),
    }
}

/// Iterates `KEY=value` / `KEY="quoted value"` pairs, skipping bare tokens.
struct Params<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Params<'a> {
    type Item = (&'a str, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let s = self.rest.trim_start_matches(' ');
            if s.is_empty() {
                return None;
            }
            let token_end = s.find([' ', '=']).unwrap_or(s.len());
            if s.as_bytes().get(token_end) != Some(&b'=') {
                self.rest = &s[token_end..];
                continue;
            }
            let key = &s[..token_end];
            let value_start = &s[token_end + 1..];
            let (raw, rest) = if let Some(quoted) = value_start.strip_prefix('"') {
                let end = closing_quote(quoted);
                (&quoted[..end], quoted.get(end + 1..).unwrap_or(""))
            } else {
                let end = value_start.find(' ').unwrap_or(value_start.len());
                (&value_start[..end], &value_start[end..])
            };
            self.rest = rest;
            return Some((key, Value { raw }));
        }
    }
}

/// Index of the unescaped closing quote, or the end of the string if it is missing.
fn closing_quote(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Writes outgoing lines into a fixed byte buffer.
pub struct LineWriter<'b> {
    buf: &'b mut [u8],
    len: usize,
    overflowed: bool,
}

impl<'b> LineWriter<'b> {
    pub fn new(buf: &'b mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            overflowed: false,
        }
    }

    /// Starts a line with its command name.
    pub fn command(&mut self, name: &str) -> &mut Self {
        self.push(name.as_bytes())
    }

    /// Appends ` KEY="value"`, escaping `"` and `\`.
    pub fn quoted(&mut self, key: &str, value: &str) -> &mut Self {
        self.push(b" ").push(key.as_bytes()).push(b"=\"");
        for &b in value.as_bytes() {
            if b == b'"' || b == b'\\' {
                self.push(b"\\");
            }
            self.push(&[b]);
        }
        self.push(b"\"")
    }

    /// Appends ` KEY=value` unquoted (numbers and booleans).
    pub fn bare(&mut self, key: &str, value: &str) -> &mut Self {
        self.push(b" ")
            .push(key.as_bytes())
            .push(b"=")
            .push(value.as_bytes())
    }

    /// Appends ` text` verbatim, e.g. a `PING` payload.
    pub fn raw(&mut self, text: &str) -> &mut Self {
        self.push(b" ").push(text.as_bytes())
    }

    /// Ends the line with `\n`.
    pub fn end(&mut self) -> &mut Self {
        self.push(b"\n")
    }

    fn push(&mut self, bytes: &[u8]) -> &mut Self {
        match self.buf.get_mut(self.len..self.len + bytes.len()) {
            Some(dst) if !self.overflowed => {
                dst.copy_from_slice(bytes);
                self.len += bytes.len();
            }
            _ => self.overflowed = true,
        }
        self
    }

    /// The bytes written so far, or `None` if the buffer was too small.
    pub fn bytes(&self) -> Option<&[u8]> {
        (!self.overflowed).then(|| &self.buf[..self.len])
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.overflowed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(framer: &mut LineFramer<'_>, chunks: &[&[u8]]) -> Vec<Result<Vec<u8>, ()>> {
        let mut out = Vec::new();
        for chunk in chunks {
            framer.feed(chunk, |f| match f {
                Frame::Line(l) => out.push(Ok(l.to_vec())),
                Frame::Overflow => out.push(Err(())),
            });
        }
        out
    }

    #[test]
    fn framer_joins_a_line_split_across_chunks() {
        let mut buf = [0u8; 64];
        let mut framer = LineFramer::new(&mut buf);
        let got = frames(&mut framer, &[b"PING", b" ab", b"c\n"]);
        assert_eq!(got, vec![Ok(b"PING abc".to_vec())]);
    }

    #[test]
    fn framer_strips_trailing_carriage_return() {
        let mut buf = [0u8; 64];
        let mut framer = LineFramer::new(&mut buf);
        let got = frames(&mut framer, &[b"PONG x\r\nPING y\n"]);
        assert_eq!(got, vec![Ok(b"PONG x".to_vec()), Ok(b"PING y".to_vec())]);
    }

    #[test]
    fn framer_skips_an_overlong_line_and_keeps_the_next() {
        let mut buf = [0u8; 8];
        let mut framer = LineFramer::new(&mut buf);
        let got = frames(&mut framer, &[b"0123456789", b"abcdef\nPING 1\n"]);
        assert_eq!(got, vec![Err(()), Ok(b"PING 1".to_vec())]);
    }

    #[test]
    fn framer_reports_whether_a_line_is_in_progress() {
        let mut buf = [0u8; 64];
        let mut framer = LineFramer::new(&mut buf);
        assert!(framer.at_line_start());
        frames(&mut framer, &[b"KEY-ST"]);
        assert!(!framer.at_line_start());
        frames(&mut framer, &[b"ATE\n"]);
        assert!(framer.at_line_start());
    }

    #[test]
    fn framer_reset_drops_a_partial_line() {
        let mut buf = [0u8; 64];
        let mut framer = LineFramer::new(&mut buf);
        frames(&mut framer, &[b"KEY-ST"]);
        framer.reset();
        let got = frames(&mut framer, &[b"PING 2\n"]);
        assert_eq!(got, vec![Ok(b"PING 2".to_vec())]);
    }

    #[test]
    fn parses_key_state_keeping_base64_padding() {
        let line = br#"KEY-STATE DEVICEID="opendeck-1" CONTROLID="0/1" LOCATION="1/0/1" PRESSED=0 TYPE="BUTTON" BITMAP="AAAA/w==" "#;
        let msg = Message::parse(line).unwrap();
        assert_eq!(msg.command, "KEY-STATE");
        assert_eq!(msg.status, None);
        assert_eq!(msg.get("CONTROLID").unwrap().raw(), "0/1");
        assert_eq!(msg.get("BITMAP").unwrap().raw(), "AAAA/w==");
        assert_eq!(msg.get("PRESSED").unwrap().as_bool(), Some(false));
        assert_eq!(msg.get("MISSING"), None);
    }

    #[test]
    fn parses_an_error_with_its_message() {
        let msg =
            Message::parse(br#"ADD-DEVICE ERROR DEVICEID="x" MESSAGE="Device exists elsewhere""#)
                .unwrap();
        assert_eq!(msg.command, "ADD-DEVICE");
        assert_eq!(msg.status, Some(Status::Error));
        assert_eq!(msg.get("MESSAGE").unwrap().raw(), "Device exists elsewhere");
        assert_eq!(msg.get("DEVICEID").unwrap().raw(), "x");
    }

    #[test]
    fn parses_quoted_values_with_escapes_and_spaces() {
        let msg = Message::parse(br#"ERROR MESSAGE="say \"hi\" now" NEXT=1"#).unwrap();
        assert_eq!(msg.command, "ERROR");
        let v = msg.get("MESSAGE").unwrap();
        assert_eq!(v.raw(), r#"say \"hi\" now"#);
        assert!(v.has_escapes());
        assert_eq!(msg.get("NEXT").unwrap().raw(), "1");
    }

    #[test]
    fn parses_acknowledgements_and_unknown_commands() {
        let ok = Message::parse(br#"KEY-PRESS OK DEVICEID="d""#).unwrap();
        assert_eq!((ok.command, ok.status), ("KEY-PRESS", Some(Status::Ok)));
        let unknown = Message::parse(b"FUTURE-THING A=1").unwrap();
        assert_eq!(unknown.command, "FUTURE-THING");
        assert_eq!(unknown.get("A").unwrap().raw(), "1");
    }

    #[test]
    fn rest_carries_a_ping_payload() {
        let msg = Message::parse(b"PING abc").unwrap();
        assert_eq!((msg.command, msg.rest()), ("PING", "abc"));
    }

    #[test]
    fn booleans_normalise() {
        for (text, want) in [
            ("1", Some(true)),
            ("true", Some(true)),
            ("0", Some(false)),
            ("false", Some(false)),
            ("2", None),
        ] {
            let line = format!("X PRESSED={text}");
            let msg = Message::parse(line.as_bytes()).unwrap();
            assert_eq!(msg.get("PRESSED").unwrap().as_bool(), want, "{text}");
        }
    }

    #[test]
    fn rejects_empty_and_non_utf8_lines() {
        assert_eq!(Message::parse(b"").unwrap_err(), ParseError::Empty);
        assert_eq!(
            Message::parse(&[0xff, 0xfe]).unwrap_err(),
            ParseError::NotUtf8
        );
    }

    #[test]
    fn writer_quotes_and_escapes_values() {
        let mut buf = [0u8; 128];
        let mut w = LineWriter::new(&mut buf);
        w.command("KEY-PRESS")
            .quoted("DEVICEID", r#"a"b\c"#)
            .quoted("CONTROLID", "0/1")
            .bare("PRESSED", "1")
            .end();
        assert_eq!(
            w.bytes().unwrap(),
            b"KEY-PRESS DEVICEID=\"a\\\"b\\\\c\" CONTROLID=\"0/1\" PRESSED=1\n"
        );
    }

    #[test]
    fn writer_reports_overflow_instead_of_truncating() {
        let mut buf = [0u8; 8];
        let mut w = LineWriter::new(&mut buf);
        w.command("PING").raw("0123456789").end();
        assert_eq!(w.bytes(), None);
    }
}
