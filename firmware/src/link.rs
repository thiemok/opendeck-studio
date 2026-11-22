//! The Companion link: connect, register, draw, send input, recover (KTD15, KTD16, KTD17,
//! KTD20, KTD23, KTD26, KTD27).
//!
//! `deck_core::session::Session` decides; this task owns the socket, the clock and the
//! displays.

use crate::console::Describe;
use crate::display::{DrawError, KeyDisplay, Status, decode_only};
use crate::expander::Expander;
use crate::inputs;
use crate::oled::Oled;
use crate::{HOST, HOST_CHANGED, SURFACE, encoder};
use alloc::boxed::Box;
use deck_core::config::HostConfig;
use deck_core::expander::{Key, key_for};
use deck_core::latency::{RoundTrips, WINDOW};
use deck_core::line::{Flow, Frame, LineFramer, LineWriter, Message};
use deck_core::pagetime::{Drawn, PageChange, PageTimer};
use deck_core::png::PngWorkspace;
use deck_core::session::{self, Control, Event, ImageFormat, Layout, SILENCE_TIMEOUT_MS, Session};
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

/// Every display the surface has (KTD20). A key is `None` when the MCP23017 that selects
/// it is missing; its images are then dropped (THI-89 KTD6).
pub struct Displays {
    pub key_a: Option<KeyDisplay>,
    pub key_b: Option<KeyDisplay>,
    pub oled: Oled,
    pub expander: &'static Expander,
}

impl Displays {
    /// Shows a link state on every display and sends the OLED's frame (KTD26).
    async fn status(&mut self, status: Status<'_>) {
        for key in [Key::A, Key::B] {
            self.on_key(key, |panel| panel.status(status)).await;
        }
        self.oled.status(status);
        self.flush_oled().await;
    }

    /// Draws a Companion bitmap on a key, selecting its panel around the synchronous draw
    /// (THI-89 KTD3). [`key_for`] says which controls are drawn on a key. A key missing
    /// since boot drops its images quietly; boot already logged why (KTD6).
    async fn draw_key(
        &mut self,
        key: Key,
        bitmap: &str,
        png: Option<&mut PngWorkspace>,
    ) -> Result<(), DrawError> {
        if !self.has_key(key) {
            return Ok(());
        }
        self.on_key(key, |panel| panel.bitmap(bitmap, png))
            .await
            .unwrap_or(Err(DrawError::NotSelected))
    }

    /// The key to pause and draw `control`'s image on. A bench key without a panel gets
    /// none, so it is decoded instead and page-change times don't count it as free.
    fn panel_for(&self, control: Control, draw: bool) -> Option<Key> {
        key_for(control, draw)
            .filter(|&key| !matches!(control, Control::Bench(_)) || self.has_key(key))
    }

    fn has_key(&self, key: Key) -> bool {
        match key {
            Key::A => self.key_a.is_some(),
            Key::B => self.key_b.is_some(),
        }
    }

    /// Blanks both keys and the OLED's frame; [`Self::flush_oled`] sends it.
    async fn blank(&mut self) {
        for key in [Key::A, Key::B] {
            self.on_key(key, KeyDisplay::blank).await;
        }
        self.oled.blank();
    }

    /// Runs `draw` on `key`'s panel with its CS held low. `None` when the key is missing or
    /// the expander didn't take the select.
    async fn on_key<R>(&mut self, key: Key, draw: impl FnOnce(&mut KeyDisplay) -> R) -> Option<R> {
        let panel = match key {
            Key::A => self.key_a.as_mut()?,
            Key::B => self.key_b.as_mut()?,
        };
        if !self.expander.select(key).await {
            return None;
        }
        let result = draw(panel);
        self.expander.deselect().await;
        Some(result)
    }

    async fn flush_oled(&mut self) {
        if let Err(e) = self.oled.flush().await {
            warn!("OLED write failed: {e:?}");
        }
    }
}

/// A line the framer paused on, handled once the callback has returned so the expander can
/// be awaited (THI-89 KTD3).
enum Paused {
    Draw {
        control: Control,
        key: Key,
        received: Duration,
    },
    Blank,
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
    let mut png: Option<Box<PngWorkspace>> = None;
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
                png: &mut png,
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
    /// The PNG decoder's memory, taken from the heap at the first `png` registration and kept
    /// (KTD4 of the THI-76 plan).
    png: &'a mut Option<Box<PngWorkspace>>,
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
    session.set_config(SURFACE.lock(|s| s.get()));
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
        png,
        framer,
        out,
    } = link;
    let surface = session.config();
    let mut page_timer: Option<PageTimer> = None;
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
                let mut data = &chunk[..n];
                let mut rejected = false;
                let mut oled_changed = false;
                while !data.is_empty() {
                    if framer.at_line_start() {
                        line_started = Instant::now();
                    }
                    let mut paused = None;
                    let used = framer.feed(data, |frame| {
                        let Frame::Line(bytes) = frame else {
                            warn!("skipped a line longer than the line buffer");
                            return Flow::Continue;
                        };
                        let Ok(msg) = Message::parse(bytes) else {
                            warn!("unparseable line ({} B)", bytes.len());
                            return Flow::Continue;
                        };
                        debug!("<- {} ({} B)", msg.command, bytes.len());
                        let mut flow = Flow::Continue;
                        match session.line(now_ms(), &msg, out) {
                            Event::Registered => {
                                let format = session.image_format();
                                if format == ImageFormat::Png && png.is_none() {
                                    **png = alloc_png_workspace();
                                }
                                info!(
                                    "registered with Companion as {} ({} images); free heap {} B",
                                    Describe(surface),
                                    format.name(),
                                    esp_alloc::HEAP.free()
                                );
                                let keys = surface.layout.bench_keys();
                                if keys > 0 {
                                    page_timer = Some(PageTimer::new(keys));
                                    displays.oled.bench(
                                        if keys == 16 { "16 keys" } else { "32 keys" },
                                        if surface.draw {
                                            "decode + draw"
                                        } else {
                                            "decode only"
                                        },
                                    );
                                    oled_changed = true;
                                }
                            }
                            Event::PngNotOffered => {
                                warn!("Companion doesn't offer png; registering for rgb");
                            }
                            Event::Draw { control, bitmap } => {
                                let received = line_started.elapsed();
                                if let Some(key) = displays.panel_for(control, surface.draw) {
                                    paused = Some(Paused::Draw {
                                        control,
                                        key,
                                        received,
                                    });
                                    flow = Flow::Pause;
                                } else {
                                    let draw_start = Instant::now();
                                    let result = if control == Control::Encoder {
                                        displays.oled.bitmap(bitmap, png.as_deref_mut())
                                    } else {
                                        decode_only(bitmap, png.as_deref_mut())
                                            .map(|sum| debug!("checksum {sum:08x}"))
                                    };
                                    oled_changed |= control == Control::Encoder && result.is_ok();
                                    drawn(
                                        &mut Report {
                                            session,
                                            round_trips,
                                            page_timer: &mut page_timer,
                                        },
                                        control,
                                        bytes.len(),
                                        received,
                                        draw_start.elapsed(),
                                        result,
                                    );
                                }
                            }
                            Event::Blank => {
                                paused = Some(Paused::Blank);
                                flow = Flow::Pause;
                            }
                            Event::Rejected(message) => {
                                warn!("Companion rejected the device: {message}");
                                rejected = true;
                            }
                            Event::Ignored(reason) => debug!("ignored: {reason}"),
                            Event::Close | Event::None => {}
                        }
                        line_started = Instant::now();
                        flow
                    });
                    data = &data[used..];
                    match paused {
                        Some(Paused::Draw {
                            control,
                            key,
                            received,
                        }) => {
                            let line = framer.held_line().unwrap_or_default();
                            let msg = Message::parse(line).ok();
                            let draw_start = Instant::now();
                            let result = match msg.as_ref().and_then(session::bitmap) {
                                Some(bitmap) => {
                                    displays.draw_key(key, bitmap, png.as_deref_mut()).await
                                }
                                None => Err(DrawError::NotHeld),
                            };
                            drawn(
                                &mut Report {
                                    session,
                                    round_trips,
                                    page_timer: &mut page_timer,
                                },
                                control,
                                line.len(),
                                received,
                                draw_start.elapsed(),
                                result,
                            );
                            line_started = Instant::now();
                        }
                        Some(Paused::Blank) => {
                            displays.blank().await;
                            oled_changed = true;
                        }
                        None => {}
                    }
                }
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
                let now = now_ms();
                if let Some(change) = page_timer.as_mut().and_then(|t| t.tick(now)) {
                    log_page_change(&change, session, page_timer.as_mut());
                }
                if session.tick(now, out) == Event::Close {
                    return Ended::Silence;
                }
            }
            Either4::Third((button, pressed)) => {
                let control = button.control().and_then(|c| surface.layout.physical(c));
                if let Some(control) = control {
                    if pressed && session.is_registered() {
                        round_trips.sent(control, now_ms());
                        if let Some(timer) = page_timer.as_mut() {
                            timer.pressed(now_ms());
                        }
                    }
                    session.key(control, pressed, out);
                } else {
                    // BACK and CONFIRM stay on the device (KTD24), and a bench surface has
                    // no encoder; the inputs task logged it.
                    debug!("{button:?} not sent to Companion");
                }
            }
            Either4::Fourth(Either::First(())) => {
                // A `KEY-ROTATE` line is about 75 B, so a batch stays well inside the 1 KB
                // outgoing buffer; a longer spin is sent over several loops.
                let detents = encoder::take_pending(ROTATIONS_PER_SEND);
                // Dropped unless registered, like presses (KTD18, KTD23).
                if session.is_registered() && surface.layout == Layout::Normal && detents != 0 {
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

/// What [`drawn`] updates after an image.
struct Report<'a> {
    session: &'a Session,
    round_trips: &'a mut RoundTrips,
    page_timer: &'a mut Option<PageTimer>,
}

/// Logs an image's receive and draw times and feeds the page timer or the round trips.
#[allow(
    clippy::large_stack_frames,
    reason = "about 1 KB, mostly log formatting; called from the link task, whose stack has room"
)]
fn drawn(
    report: &mut Report<'_>,
    control: Control,
    wire_bytes: usize,
    received: Duration,
    draw_time: Duration,
    result: Result<(), DrawError>,
) {
    if let Err(e) = result {
        warn!("{} bitmap not drawn: {e:?}", control.id());
        return;
    }
    let (received_ms, drawn_ms) = (received.as_millis(), draw_time.as_millis());
    let Control::Bench(key) = control else {
        info!(
            "{} bitmap {wire_bytes} B received in {received_ms} ms, drawn in {drawn_ms} ms",
            control.id(),
        );
        // The OLED's round trip ends when its frame is sent, after the callback.
        if control != Control::Encoder {
            record_round_trip(report.round_trips, control);
        }
        return;
    };
    // At 32 keys an info line per image would add UART time to every page change.
    debug!(
        "{} bitmap {wire_bytes} B received in {received_ms} ms, drawn in {drawn_ms} ms",
        control.id(),
    );
    let done = now_ms();
    let image = Drawn {
        key,
        line_started_ms: done.saturating_sub(received_ms + drawn_ms),
        done_ms: done,
        wire_bytes,
        receive_ms: ms32(received_ms),
        decode_ms: ms32(drawn_ms),
    };
    if let Some(change) = report.page_timer.as_mut().and_then(|t| t.drawn(image)) {
        log_page_change(&change, report.session, report.page_timer.as_mut());
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

/// One log line per page change on a bench surface, and the median every so often (THI-76).
fn log_page_change(change: &PageChange, session: &Session, timer: Option<&mut PageTimer>) {
    let surface = session.config();
    info!(
        "page change {}/{} keys {} {}: press to last {}, first to last {} ms, {} images {} B, \
         receive sum {} max {} ms, decode sum {} max {} ms{}",
        change.drawn,
        change.keys,
        session.image_format().name(),
        if surface.draw { "draw" } else { "decode" },
        OptionalMs(change.press_to_last_ms),
        change.first_to_last_ms,
        change.images,
        change.wire_bytes,
        change.receive_ms_sum,
        change.receive_ms_max,
        change.decode_ms_sum,
        change.decode_ms_max,
        if change.complete() { "" } else { " (partial)" },
    );
    if let Some(timer) = timer
        && timer.summary_due()
        && let (Some(median), Some(max)) = (timer.median(), timer.max())
    {
        info!(
            "page changes over the last {WINDOW}: press to last key median {median} ms, max {max} ms; free heap {} B",
            esp_alloc::HEAP.free()
        );
    }
}

/// Takes the PNG decoder's memory from the heap, zeroed so nothing that size is built on a
/// stack (KTD4). `None` when the heap can't spare it; PNG images are then reported, not drawn.
fn alloc_png_workspace() -> Option<Box<PngWorkspace>> {
    let before = esp_alloc::HEAP.free();
    let layout = core::alloc::Layout::new::<PngWorkspace>();
    // SAFETY: the layout is PngWorkspace's own and non-zero in size, and all-zero bytes are a
    // valid PngWorkspace (documented in deck_core::png and tested there with a zeroed one).
    let workspace = unsafe {
        let ptr = alloc::alloc::alloc_zeroed(layout).cast::<PngWorkspace>();
        (!ptr.is_null()).then(|| Box::from_raw(ptr))
    };
    match &workspace {
        Some(_) => info!(
            "PNG decoder: {} B from the heap; free heap {before} B -> {} B",
            layout.size(),
            esp_alloc::HEAP.free()
        ),
        None => warn!(
            "PNG decoder needs {} B but the heap has {before} B free; png images won't draw",
            layout.size()
        ),
    }
    workspace
}

/// `123 ms`, or `none` when the page changed without a press from the deck.
struct OptionalMs(Option<u32>);

impl core::fmt::Display for OptionalMs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(ms) => write!(f, "{ms} ms"),
            None => f.write_str("none"),
        }
    }
}

fn ms32(ms: u64) -> u32 {
    u32::try_from(ms).unwrap_or(u32::MAX)
}

fn now_ms() -> u64 {
    Instant::now().as_millis()
}
