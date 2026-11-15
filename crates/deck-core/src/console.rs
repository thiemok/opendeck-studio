//! The USB serial console's command language (KTD13).
//!
//! ```text
//! host <hostname-or-ip>[:port]   set the Companion host (port defaults to 16622)
//! host clear                     forget the host
//! show                           print the saved host
//! help                           list the commands
//! ```

use crate::config::{DEFAULT_PORT, valid_host};

#[derive(Debug, PartialEq, Eq)]
pub enum Command<'a> {
    SetHost { host: &'a str, port: u16 },
    ClearHost,
    Show,
    Help,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConsoleError {
    /// Nothing typed.
    Empty,
    /// `host` with no argument.
    MissingHost,
    /// More words than the command takes.
    TooManyArgs,
    /// The host is longer than 63 characters or has characters outside a hostname/IPv4 address.
    BadHost,
    /// The port is not a number from 1 to 65535.
    BadPort,
    /// Not a command; the reply suggests `help`.
    Unknown,
}

impl ConsoleError {
    /// A one-line reason for the console reply.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Empty => "",
            Self::MissingHost => "usage: host <hostname-or-ip>[:port] | host clear",
            Self::TooManyArgs => "too many arguments; try `help`",
            Self::BadHost => "host must be a hostname or IPv4 address of at most 63 characters",
            Self::BadPort => "port must be a number from 1 to 65535",
            Self::Unknown => "unknown command; try `help`",
        }
    }
}

pub const HELP: &str = "commands: host <hostname-or-ip>[:port] | host clear | show | help";

pub fn parse(line: &str) -> Result<Command<'_>, ConsoleError> {
    let mut words = line.split_whitespace();
    let Some(command) = words.next() else {
        return Err(ConsoleError::Empty);
    };
    let arg = words.next();
    if words.next().is_some() {
        return Err(ConsoleError::TooManyArgs);
    }
    match (command, arg) {
        ("host", None) => Err(ConsoleError::MissingHost),
        ("host", Some("clear")) => Ok(Command::ClearHost),
        ("host", Some(target)) => parse_target(target),
        ("show", None) => Ok(Command::Show),
        ("help", None) => Ok(Command::Help),
        ("show" | "help", Some(_)) => Err(ConsoleError::TooManyArgs),
        _ => Err(ConsoleError::Unknown),
    }
}

/// `<host>` or `<host>:<port>`.
fn parse_target(target: &str) -> Result<Command<'_>, ConsoleError> {
    let (host, port) = match target.split_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>()
                .ok()
                .filter(|&p| p != 0)
                .ok_or(ConsoleError::BadPort)?,
        ),
        None => (target, DEFAULT_PORT),
    };
    if !valid_host(host) {
        return Err(ConsoleError::BadHost);
    }
    Ok(Command::SetHost { host, port })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_with_an_ip_uses_the_default_port() {
        assert_eq!(
            parse("host 192.168.1.10"),
            Ok(Command::SetHost {
                host: "192.168.1.10",
                port: 16622
            })
        );
    }

    #[test]
    fn host_with_a_name_and_port() {
        assert_eq!(
            parse("host companion.lan:16700"),
            Ok(Command::SetHost {
                host: "companion.lan",
                port: 16700
            })
        );
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        assert_eq!(parse("  show \r"), Ok(Command::Show));
    }

    #[test]
    fn bad_host_commands_are_errors() {
        let long = format!("host {}", "a".repeat(64));
        for (line, want) in [
            ("host", ConsoleError::MissingHost),
            ("host a b", ConsoleError::TooManyArgs),
            ("host x:0", ConsoleError::BadPort),
            ("host x:70000", ConsoleError::BadPort),
            ("host x:", ConsoleError::BadPort),
            ("host bad_host!", ConsoleError::BadHost),
            ("host :16622", ConsoleError::BadHost),
            (long.as_str(), ConsoleError::BadHost),
        ] {
            assert_eq!(parse(line), Err(want), "{line}");
        }
    }

    #[test]
    fn a_63_character_host_is_accepted() {
        let line = format!("host {}", "a".repeat(63));
        assert!(matches!(parse(&line), Ok(Command::SetHost { .. })));
    }

    #[test]
    fn other_commands_parse() {
        assert_eq!(parse("host clear"), Ok(Command::ClearHost));
        assert_eq!(parse("show"), Ok(Command::Show));
        assert_eq!(parse("help"), Ok(Command::Help));
        assert_eq!(parse("show extra"), Err(ConsoleError::TooManyArgs));
    }

    #[test]
    fn unknown_and_empty_lines() {
        assert_eq!(parse("reboot"), Err(ConsoleError::Unknown));
        assert!(ConsoleError::Unknown.reason().contains("help"));
        assert_eq!(parse("   "), Err(ConsoleError::Empty));
    }
}
