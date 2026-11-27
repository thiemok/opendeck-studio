//! Reads the five buttons through the MCP23017 and sends debounced edges to the link
//! (KTD18, KTD24; THI-89 KTD4, KTD5).
//!
//! The task waits on the interrupt line's level, not an edge, so an interrupt left
//! asserted wakes it again instead of being lost.
//!
//! Bench only: key A's switch may also be wired to the reference pin, and the task logs how long the
//! expander path took to see a press on it. The edge is stamped in a GPIO interrupt
//! handler: key draws and OLED writes run synchronously on the executor and would hide
//! exactly the wait being measured.

use core::cell::{Cell, RefCell};
use deck_core::buttons::{Next, PressSampler, ReadLoop, Reference};
use deck_core::debounce::{DEFAULT_INTERVAL_MS, Debouncer, Edge};
use deck_core::expander::{BUTTON_COUNT, BUTTONS, Button};
use deck_core::latency::Samples;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::{Event, Input, Io};
use esp_hal::handler;
use log::{info, warn};

use crate::expander::Expander;

const KEY_A: usize = 0;
const _: () = assert!(matches!(BUTTONS[KEY_A], Button::KeyA));

const SAMPLE_PERIOD: Duration = Duration::from_millis(2);
/// After a failed read the task waits this long before trying again, instead of spinning
/// on a latched interrupt or on the floating line of a missing expander.
const RETRY_AFTER_FAILURE: Duration = Duration::from_millis(100);
/// Longest the task waits for INTB before reading anyway.
const SAFETY_POLL: Duration = Duration::from_millis(250);
/// Edge-to-detection samples between summaries.
const SUMMARY_EVERY: usize = 20;
/// A reference stamp older than this came from noise on the idle line, not from this press.
const MAX_REFERENCE_AGE_US: u64 = 250_000;

/// Debounced edges for the link: `true` = pressed. Full means the link is busy; an edge is
/// then dropped, which matches "never queued" for a link that isn't registered.
pub static EDGES: Channel<CriticalSectionRawMutex, (Button, bool), 16> = Channel::new();

/// The bench-only reference pin, owned by the GPIO interrupt handler.
static REFERENCE_PIN: Mutex<CriticalSectionRawMutex, RefCell<Option<Input<'static>>>> =
    Mutex::new(RefCell::new(None));
/// When the reference pin first fell, in µs since boot. Later bounces keep the first stamp;
/// the first expander read that shows key A pressed takes it.
static REFERENCE_EDGE_US: Mutex<CriticalSectionRawMutex, Cell<Option<u64>>> =
    Mutex::new(Cell::new(None));

/// Installs the GPIO interrupt handler that stamps the reference pin's falling edges and,
/// on boards that decode the encoder in software, steps the decoder. It clears only those
/// pins' status, so async waits on other pins keep working.
pub fn install_gpio_handler(io: &mut Io<'_>, mut pin: Input<'static>) {
    io.set_interrupt_handler(gpio_handler);
    REFERENCE_PIN.lock(|slot| {
        pin.listen(Event::FallingEdge);
        slot.replace(Some(pin));
    });
}

#[handler]
fn gpio_handler() {
    crate::board::on_gpio_interrupt();
    REFERENCE_PIN.lock(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(pin) = slot.as_mut() else {
            return;
        };
        if pin.is_interrupt_set() {
            let now = Instant::now().as_micros();
            REFERENCE_EDGE_US.lock(|edge| {
                if edge.get().is_none() {
                    edge.set(Some(now));
                }
            });
            pin.clear_interrupt();
        }
    });
}

#[embassy_executor::task]
#[allow(
    clippy::large_stack_frames,
    reason = "about 1.5 KB, mostly log formatting and the 64-sample delay window; the task \
    stack has room, as for the link"
)]
pub async fn inputs_task(expander: &'static Expander, mut interrupt: Input<'static>) {
    if !expander.present() {
        return;
    }
    let mut debouncers: [Debouncer; BUTTON_COUNT] =
        core::array::from_fn(|_| Debouncer::new(DEFAULT_INTERVAL_MS));
    let mut sampler = PressSampler::new(DEFAULT_INTERVAL_MS);
    let mut settled = true;
    let mut delays = Samples::new(SUMMARY_EVERY);
    let mut failing = false;
    let mut read_loop = ReadLoop::new();
    let mut last_check = Instant::now();
    loop {
        // The timer also catches an interrupt line that never falls (floating high, or a
        // change that landed during the previous read) and a chip that lost its
        // configuration to a power loss.
        if let Either::Second(()) =
            select(interrupt.wait_for_low(), Timer::after(SAFETY_POLL)).await
        {
            expander.restore_if_reset().await;
            last_check = Instant::now();
        }
        loop {
            let levels = match expander.buttons().await {
                Ok(levels) => {
                    if failing {
                        info!("MCP23017 reads again");
                        failing = false;
                        // It may have lost power, which resets every register, so these
                        // levels may lack the pull-ups: read again once it's configured.
                        expander.reinit().await;
                        continue;
                    }
                    levels
                }
                Err(e) => {
                    if !failing {
                        warn!(
                            "MCP23017 read failed: {e:?}; retrying every {} ms",
                            RETRY_AFTER_FAILURE.as_millis()
                        );
                        failing = true;
                    }
                    Timer::after(RETRY_AFTER_FAILURE).await;
                    if read_loop.after_failed_read(settled) {
                        continue;
                    }
                    break;
                }
            };
            let now_us = Instant::now().as_micros();
            let now_ms = now_us / 1000;
            match sampler.update(levels[KEY_A], now_ms) {
                Some(Reference::Sample) => record_reference_delay(now_us, &mut delays),
                Some(Reference::Rearm) => REFERENCE_EDGE_US.lock(|edge| edge.set(None)),
                None => {}
            }
            for ((button, level), debouncer) in BUTTONS.iter().zip(levels).zip(&mut debouncers) {
                if let Some(edge) = debouncer.update(level, now_ms) {
                    let pressed = edge == Edge::Pressed;
                    info!(
                        "{button:?} {}",
                        if pressed { "pressed" } else { "released" }
                    );
                    let _ = EDGES.try_send((*button, pressed));
                }
            }
            settled = debouncers.iter().all(Debouncer::settled) && sampler.settled();
            match read_loop.after_read(settled, interrupt.is_low()) {
                Next::WaitForInterrupt => break,
                Next::Poll { warn_stuck } => {
                    if warn_stuck {
                        warn!(
                            "INTB stays low after reads; is IB wired to {}? polling every {} ms",
                            crate::board::INTB_PIN,
                            SAMPLE_PERIOD.as_millis()
                        );
                    }
                }
            }
            // A line stuck low keeps the loop here, away from the timer's check above.
            if last_check.elapsed() >= SAFETY_POLL {
                expander.restore_if_reset().await;
                last_check = Instant::now();
            }
            Timer::after(SAMPLE_PERIOD).await;
        }
    }
}

/// Logs the time from the reference pin's edge to this read, when a stamp is waiting.
fn record_reference_delay(now_us: u64, delays: &mut Samples) {
    let Some(edge_us) = REFERENCE_EDGE_US.lock(|edge| edge.take()) else {
        return;
    };
    let elapsed = now_us.saturating_sub(edge_us);
    if elapsed > MAX_REFERENCE_AGE_US {
        info!(
            "KEY A edge stamp is {elapsed} µs old, over the {} ms limit; not counted",
            MAX_REFERENCE_AGE_US / 1000
        );
        return;
    }
    let delay_us = u32::try_from(elapsed).unwrap_or(u32::MAX);
    info!("KEY A edge to expander read: {delay_us} µs");
    delays.push(delay_us);
    if delays.summary_due() {
        info!(
            "KEY A edge to expander read, recent presses: median {} µs, max {} µs",
            delays.median().unwrap_or(0),
            delays.max().unwrap_or(0)
        );
    }
}
