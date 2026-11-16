//! The Companion link: connect, register, draw, send input, recover (KTD15, KTD16, KTD17,
//! KTD20, KTD23, KTD26, KTD27).
//!
//! `deck_core::session::Session` decides; this task owns the socket, the clock and the
//! displays.

use crate::display::{KeyDisplay, Status};
use crate::inputs;
use crate::oled::Oled;
use crate::{HOST, HOST_CHANGED, encoder};
use deck_core::bitmap::BitmapError;
use deck_core::config::HostConfig;
use deck_core::latency::{RoundTrips, WINDOW};
use deck_core::line::{Frame, LineFramer, LineWriter, Message};
use deck_core::session::{Control, Event, SILENCE_TIMEOUT_MS, Session};
use embassy_futures::select::{Either, Either4, select, select4};
use embassy_net::Stack;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::Write;
use log::{debug, info, warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(100);
/// Longest a send may wait for room in the TX buffer. Unacknowledged data from a host that
/// vanished silently fills it, and `write_all` would otherwise wait forever, starving the
/// silence watchdog and `HOST_CHANGED`. Same bound as the silence watchdog.
const WRITE_TIMEOUT: Duration = Duration::from_millis(SILENCE_TIMEOUT_MS);
/// smoltcp closes the socket when nothing arrives for this long. A live link answers the
/// 2 s `PING` with a `PONG`, so this only trips on a dead peer; backstop for any wait
/// other than the write.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);
/// Most `KEY-ROTATE` lines sent in one write (KTD23).
const ROTATIONS_PER_SEND: u32 = 8;

/// Buffers the link needs, all static so nothing large sits on a stack.
pub struct Buffers {
    pub line: &'static mut [u8],
    pub rx: &'static mut [u8],
    pub tx: &'static mut [u8],
    pub chunk: &'static mut [u8],
    pub out: &'static mut [u8],
}

/// Every display the surface has (KTD20).
pub struct Displays {
    pub key_a: KeyDisplay,
    pub key_b: KeyDisplay,
    pub oled: Oled,
}

impl Displays {
    /// Shows a link state on every display and sends the OLED's frame (KTD26).
    async fn status(&mut self, status: Status<'_>) {
        self.key_a.status(status);
        self.key_b.status(status);
        self.oled.status(status);
        self.flush_oled().await;
    }

    /// Draws a Companion bitmap. The OLED's frame changes here; [`Self::flush_oled`] sends it.
    fn bitmap(&mut self, control: Control, b64: &str) -> Result<(), BitmapError> {
        match control {
            Control::KeyA => self.key_a.bitmap(b64),
            Control::KeyB => self.key_b.bitmap(b64),
            Control::Encoder => self.oled.bitmap(b64),
        }
    }

    fn blank(&mut self) {
        self.key_a.blank();
        self.key_b.blank();
        self.oled.blank();
    }

    async fn flush_oled(&mut self) {
        if let Err(e) = self.oled.flush().await {
            warn!("OLED write failed: {e:?}");
        }
    }
}

/// The socket's receive, transmit and read-chunk buffers, lent to each connection attempt.
struct SocketBuffers {
    rx: &'static mut [u8],
    tx: &'static mut [u8],
    chunk: &'static mut [u8],
}

/// What the displays currently show when not registered, so they are redrawn only on change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    NoWifi,
    NoHost,
    Connecting(HostConfig),
}

/// Why a connection ended.
#[derive(Debug, PartialEq, Eq)]
enum Ended {
    Closed,
    Silence,
    Rejected,
    HostChanged,
}

#[allow(
    clippy::large_stack_frames,
    reason = "about 1-1.6 KB, mostly log formatting; the main task stack has far more room"
)]
pub async fn run(
    stack: Stack<'static>,
    mut displays: Displays,
    mut session: Session,
    bufs: Buffers,
) -> ! {
    let Buffers {
        line,
        rx,
        tx,
        chunk,
        out,
    } = bufs;
    let mut framer = LineFramer::new(line);
    let mut out = LineWriter::new(out);
    let mut sockets = SocketBuffers { rx, tx, chunk };
    let mut round_trips = RoundTrips::new();
    let mut shown: Option<Shown> = None;

    loop {
        let host = HOST.lock(|h| h.get());
        let want = match (stack.is_config_up(), host) {
            (false, _) => Shown::NoWifi,
            (true, None) => Shown::NoHost,
            (true, Some(config)) => Shown::Connecting(config),
        };
        if shown != Some(want) {
            displays
                .status(match &want {
                    Shown::NoWifi => Status::NoWifi,
                    Shown::NoHost => Status::NoHost,
                    Shown::Connecting(config) => Status::Connecting {
                        host: config.host(),
                    },
                })
                .await;
            shown = Some(want);
        }

        let Shown::Connecting(config) = want else {
            wait_for_change(Duration::from_millis(500), &mut session).await;
            continue;
        };
        if !session.connect_due(now_ms()) {
            wait_for_change(TICK, &mut session).await;
            continue;
        }

        let ended = connect_and_serve(
            stack,
            &config,
            &mut session,
            &mut Link {
                displays: &mut displays,
                round_trips: &mut round_trips,
                framer: &mut framer,
                out: &mut out,
            },
            &mut sockets,
        )
        .await;
        session.closed(now_ms());
        let Some(ended) = ended else {
            continue;
        };
        info!("Companion link ended: {ended:?}");
        if ended == Ended::HostChanged {
            session.reconnect_now();
        }
        shown = None;
    }
}

/// What one connection works on, borrowed from [`run`].
struct Link<'a, 'b, 'c> {
    displays: &'a mut Displays,
    round_trips: &'a mut RoundTrips,
    framer: &'a mut LineFramer<'b>,
    out: &'a mut LineWriter<'c>,
}

/// Waits up to `timeout`; a host change allows an immediate reconnect.
async fn wait_for_change(timeout: Duration, session: &mut Session) {
    if let Either::First(()) = select(HOST_CHANGED.wait(), Timer::after(timeout)).await {
        session.reconnect_now();
    }
}

/// One connection attempt. `None` means it never connected.
#[allow(
    clippy::large_stack_frames,
    reason = "about 1-1.6 KB, mostly log formatting; the main task stack has far more room"
)]
async fn connect_and_serve(
    stack: Stack<'static>,
    config: &HostConfig,
    session: &mut Session,
    link: &mut Link<'_, '_, '_>,
    sockets: &mut SocketBuffers,
) -> Option<Ended> {
    // Resolved on every attempt; an IP literal returns without a query.
    let address = match stack.dns_query(config.host(), DnsQueryType::A).await {
        Ok(addrs) if !addrs.is_empty() => addrs[0],
        other => {
            warn!("cannot resolve {}: {other:?}", config.host());
            return None;
        }
    };

    let mut socket = TcpSocket::new(stack, sockets.rx, sockets.tx);
    socket.set_nagle_enabled(false);
    socket.set_timeout(Some(SOCKET_TIMEOUT));
    match with_timeout(CONNECT_TIMEOUT, socket.connect((address, config.port))).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            debug!("connect to {}:{} failed: {e:?}", config.host(), config.port);
            socket.abort();
            return None;
        }
        Err(_) => {
            debug!("connect to {}:{} timed out", config.host(), config.port);
            socket.abort();
            return None;
        }
    }
    info!("connected to {}:{}", config.host(), config.port);
    session.connected(now_ms());
    link.framer.reset();
    link.out.clear();

    let ended = serve(&mut socket, session, link, sockets.chunk).await;
    socket.abort();
    Some(ended)
}

#[allow(
    clippy::large_stack_frames,
    reason = "about 1-1.6 KB, mostly log formatting; the main task stack has far more room"
)]
async fn serve(
    socket: &mut TcpSocket<'_>,
    session: &mut Session,
    link: &mut Link<'_, '_, '_>,
    chunk: &mut [u8],
) -> Ended {
    let Link {
        displays,
        round_trips,
        framer,
        out,
    } = link;
    let mut line_started = Instant::now();
    loop {
        let input = select4(
            socket.read(chunk),
            Timer::after(TICK),
            inputs::EDGES.receive(),
            select(encoder::TURNED.wait(), HOST_CHANGED.wait()),
        )
        .await;
        match input {
            Either4::First(Ok(0) | Err(_)) => return Ended::Closed,
            Either4::First(Ok(n)) => {
                if framer.at_line_start() {
                    line_started = Instant::now();
                }
                let mut rejected = false;
                let mut oled_changed = false;
                framer.feed(&chunk[..n], |frame| match frame {
                    Frame::Overflow => warn!("skipped a line longer than the line buffer"),
                    Frame::Line(bytes) => {
                        let Ok(msg) = Message::parse(bytes) else {
                            warn!("unparseable line ({} B)", bytes.len());
                            return;
                        };
                        debug!("<- {} ({} B)", msg.command, bytes.len());
                        match session.line(now_ms(), &msg, out) {
                            Event::Registered => {
                                info!(
                                    "registered with Companion; free heap {} B",
                                    esp_alloc::HEAP.free()
                                );
                            }
                            Event::Draw { control, bitmap } => {
                                let received = line_started.elapsed();
                                let draw_start = Instant::now();
                                match displays.bitmap(control, bitmap) {
                                    Ok(()) => {
                                        info!(
                                            "{} bitmap {} B received in {} ms, drawn in {} ms",
                                            control.id(),
                                            bytes.len(),
                                            received.as_millis(),
                                            draw_start.elapsed().as_millis()
                                        );
                                        if control == Control::Encoder {
                                            oled_changed = true;
                                        } else {
                                            record_round_trip(round_trips, control);
                                        }
                                    }
                                    Err(e) => warn!("{} bitmap not drawn: {e:?}", control.id()),
                                }
                            }
                            Event::Blank => {
                                displays.blank();
                                oled_changed = true;
                            }
                            Event::Rejected(message) => {
                                warn!("Companion rejected the device: {message}");
                                rejected = true;
                            }
                            Event::Ignored(reason) => debug!("ignored: {reason}"),
                            Event::Close | Event::None => {}
                        }
                        line_started = Instant::now();
                    }
                });
                if oled_changed {
                    // The I²C write is async, so it runs after the line callback (KTD22).
                    displays.flush_oled().await;
                    record_round_trip(round_trips, Control::Encoder);
                }
                if rejected {
                    return Ended::Rejected;
                }
            }
            Either4::Second(()) => {
                if session.tick(now_ms(), out) == Event::Close {
                    return Ended::Silence;
                }
            }
            Either4::Third((button, pressed)) => {
                if let Some(control) = button.control() {
                    if pressed && session.is_registered() {
                        round_trips.sent(control, now_ms());
                    }
                    session.key(control, pressed, out);
                } else {
                    // BACK and CONFIRM stay on the device (KTD24); the inputs task logged it.
                    debug!("{button:?} not sent to Companion");
                }
            }
            Either4::Fourth(Either::First(())) => {
                // A `KEY-ROTATE` line is about 75 B, so a batch stays well inside the 1 KB
                // outgoing buffer; a longer spin is sent over several loops.
                let detents = encoder::take_pending(ROTATIONS_PER_SEND);
                // Dropped unless registered, like presses (KTD18, KTD23).
                if session.is_registered() && detents != 0 {
                    round_trips.sent(Control::Encoder, now_ms());
                    for _ in 0..detents.unsigned_abs() {
                        session.rotate(detents > 0, out);
                    }
                }
            }
            Either4::Fourth(Either::Second(())) => return Ended::HostChanged,
        }

        match out.bytes() {
            Some([]) => {}
            Some(bytes) => match with_timeout(WRITE_TIMEOUT, socket.write_all(bytes)).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Ended::Closed,
                Err(_) => {
                    warn!(
                        "send blocked for {} ms; dropping the link",
                        WRITE_TIMEOUT.as_millis()
                    );
                    return Ended::Silence;
                }
            },
            None => warn!("outgoing line buffer overflowed; dropped"),
        }
        out.clear();
    }
}

/// Logs a press-to-image sample when this draw answers a send, and a summary every so
/// often (KTD27).
fn record_round_trip(round_trips: &mut RoundTrips, control: Control) {
    let Some(ms) = round_trips.drawn(control, now_ms()) else {
        return;
    };
    info!("round trip {} {ms} ms", control.id());
    if round_trips.summary_due()
        && let (Some(median), Some(max)) = (round_trips.median(), round_trips.max())
    {
        info!("round trips over the last {WINDOW}: median {median} ms, max {max} ms");
    }
}

fn now_ms() -> u64 {
    Instant::now().as_millis()
}
