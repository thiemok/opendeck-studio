//! The Companion link: connect, register, draw, send presses, recover (KTD15, KTD16, KTD17).
//!
//! `deck_core::session::Session` decides; this task owns the socket, the clock and the display.

use crate::display::{KeyDisplay, Status};
use crate::{HOST, HOST_CHANGED, key};
use deck_core::config::HostConfig;
use deck_core::line::{Frame, LineFramer, LineWriter, Message};
use deck_core::session::{Event, Session};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_net::Stack;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::Write;
use log::{debug, info, warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(100);

/// Buffers the link needs, all static so nothing large sits on a stack.
pub struct Buffers {
    pub line: &'static mut [u8],
    pub rx: &'static mut [u8],
    pub tx: &'static mut [u8],
    pub chunk: &'static mut [u8],
    pub out: &'static mut [u8],
}

/// What the key currently shows when not registered, so it is redrawn only on change.
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
    mut display: KeyDisplay,
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
    let mut shown: Option<Shown> = None;

    loop {
        let host = HOST.lock(|h| h.get());
        let want = match (stack.is_config_up(), host) {
            (false, _) => Shown::NoWifi,
            (true, None) => Shown::NoHost,
            (true, Some(config)) => Shown::Connecting(config),
        };
        if shown != Some(want) {
            display.status(match &want {
                Shown::NoWifi => Status::NoWifi,
                Shown::NoHost => Status::NoHost,
                Shown::Connecting(config) => Status::Connecting {
                    host: config.host(),
                },
            });
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
            &mut display,
            &mut framer,
            &mut out,
            rx,
            tx,
            chunk,
        )
        .await;
        let Some(ended) = ended else {
            session.closed(now_ms());
            continue;
        };
        info!("Companion link ended: {ended:?}");
        session.closed(now_ms());
        if ended == Ended::HostChanged {
            session.reconnect_now();
        }
        // Back to a status screen until the next `ADD-DEVICE OK` (R7).
        shown = None;
    }
}

/// Waits up to `timeout`; a host change allows an immediate reconnect.
async fn wait_for_change(timeout: Duration, session: &mut Session) {
    if let Either::First(()) = select(HOST_CHANGED.wait(), Timer::after(timeout)).await {
        session.reconnect_now();
    }
}

/// One connection attempt. `None` means it never connected.
#[allow(
    clippy::too_many_arguments,
    reason = "the buffers are borrowed per attempt"
)]
#[allow(
    clippy::large_stack_frames,
    reason = "about 1-1.6 KB, mostly log formatting; the main task stack has far more room"
)]
async fn connect_and_serve(
    stack: Stack<'static>,
    config: &HostConfig,
    session: &mut Session,
    display: &mut KeyDisplay,
    framer: &mut LineFramer<'_>,
    out: &mut LineWriter<'_>,
    rx: &mut [u8],
    tx: &mut [u8],
    chunk: &mut [u8],
) -> Option<Ended> {
    // Resolved on every attempt; an IP literal returns without a query.
    let address = match stack.dns_query(config.host(), DnsQueryType::A).await {
        Ok(addrs) if !addrs.is_empty() => addrs[0],
        other => {
            warn!("cannot resolve {}: {other:?}", config.host());
            return None;
        }
    };

    let mut socket = TcpSocket::new(stack, rx, tx);
    socket.set_nagle_enabled(false);
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
    framer.reset();
    out.clear();

    let ended = serve(&mut socket, session, display, framer, out, chunk).await;
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
    display: &mut KeyDisplay,
    framer: &mut LineFramer<'_>,
    out: &mut LineWriter<'_>,
    chunk: &mut [u8],
) -> Ended {
    let mut line_started = Instant::now();
    loop {
        let input = select3(
            socket.read(chunk),
            Timer::after(TICK),
            select(key::EDGES.receive(), HOST_CHANGED.wait()),
        )
        .await;
        match input {
            Either3::First(Ok(0) | Err(_)) => return Ended::Closed,
            Either3::First(Ok(n)) => {
                if framer.at_line_start() {
                    line_started = Instant::now();
                }
                let mut rejected = false;
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
                            Event::Draw(b64) => {
                                let received = line_started.elapsed();
                                let draw_start = Instant::now();
                                match display.bitmap(b64) {
                                    Ok(()) => info!(
                                        "bitmap {} B received in {} ms, drawn in {} ms",
                                        bytes.len(),
                                        received.as_millis(),
                                        draw_start.elapsed().as_millis()
                                    ),
                                    Err(e) => warn!("bitmap not drawn: {e:?}"),
                                }
                            }
                            Event::Blank => display.blank(),
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
                if rejected {
                    return Ended::Rejected;
                }
            }
            Either3::Second(()) => {
                if session.tick(now_ms(), out) == Event::Close {
                    return Ended::Silence;
                }
            }
            Either3::Third(Either::First(pressed)) => session.key(pressed, out),
            Either3::Third(Either::Second(())) => return Ended::HostChanged,
        }

        match out.bytes() {
            Some([]) => {}
            Some(bytes) => {
                if socket.write_all(bytes).await.is_err() {
                    return Ended::Closed;
                }
            }
            None => warn!("outgoing line buffer overflowed; dropped"),
        }
        out.clear();
    }
}

fn now_ms() -> u64 {
    Instant::now().as_millis()
}
