//! Reads the five buttons through the MCP23017 and sends debounced edges to the link
//! (KTD18, KTD24; THI-89 KTD4, KTD5).
//!
//! The task waits on the interrupt line's level, not an edge, so an interrupt left
//! asserted wakes it again instead of being lost.
//!
//! Bench only: key A's switch may also be wired to GPIO32, and the task logs how long the
//! expander path took to see a press on it. The edge is stamped in a GPIO interrupt
//! handler: key draws and OLED writes run synchronously on the executor and would hide
//! exactly the wait being measured.

use core::cell::{Cell, RefCell};
use deck_core::debounce::{DEFAULT_INTERVAL_MS, Debouncer, Edge};
use deck_core::expander::BUTTON_COUNT;
use deck_core::latency::Samples;
use deck_core::session::Control;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::{Event, Input, Io};
use esp_hal::handler;
use log::{info, warn};

use crate::expander::Expander;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    KeyA,
    KeyB,
    EncoderPush,
    Back,
    Confirm,
}

/// Must match `deck_core::expander::pressed`'s port-B bit order, GPB0–GPB4.
const BUTTONS: [Button; BUTTON_COUNT] = [
    Button::KeyA,
    Button::KeyB,
    Button::EncoderPush,
    Button::Back,
    Button::Confirm,
];
const KEY_A: usize = 0;

impl Button {
    /// The Companion control a button reports to. BACK and CONFIRM belong to the device and
    /// are never reported (KTD24).
    pub fn control(self) -> Option<Control> {
        match self {
            Button::KeyA => Some(Control::KeyA),
            Button::KeyB => Some(Control::KeyB),
            Button::EncoderPush => Some(Control::Encoder),
            Button::Back | Button::Confirm => None,
        }
    }
}

const SAMPLE_PERIOD: Duration = Duration::from_millis(2);
/// After a failed read the task waits this long before trying again, instead of spinning
/// on a latched interrupt or on the floating line of a missing expander.
const RETRY_AFTER_FAILURE: Duration = Duration::from_millis(100);
/// Edge-to-detection samples between summaries.
const SUMMARY_EVERY: usize = 20;
/// A reference stamp older than this came from noise on the idle line, not from this press.
const MAX_REFERENCE_AGE_US: u64 = 250_000;

/// Debounced edges for the link: `true` = pressed. Full means the link is busy; an edge is
/// then dropped, which matches "never queued" for a link that isn't registered.
pub static EDGES: Channel<CriticalSectionRawMutex, (Button, bool), 16> = Channel::new();

/// The bench-only reference pin (GPIO32), owned by the GPIO interrupt handler.
static REFERENCE_PIN: Mutex<CriticalSectionRawMutex, RefCell<Option<Input<'static>>>> =
    Mutex::new(RefCell::new(None));
/// When the reference pin first fell, in µs since boot. Later bounces keep the first stamp;
/// the first expander read that shows key A pressed takes it.
static REFERENCE_EDGE_US: Mutex<CriticalSectionRawMutex, Cell<Option<u64>>> =
    Mutex::new(Cell::new(None));

/// Installs the GPIO interrupt handler that stamps the reference pin's falling edges. The
/// handler clears only that pin's status, so async waits on other pins keep working.
pub fn install_reference(io: &mut Io<'_>, mut pin: Input<'static>) {
    io.set_interrupt_handler(gpio_handler);
    REFERENCE_PIN.lock(|slot| {
        pin.listen(Event::FallingEdge);
        slot.replace(Some(pin));
    });
}

#[handler]
fn gpio_handler() {
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
    // True from key A's debounced release until the first read of its next press, so each
    // press yields one edge-to-detection sample however much the switch bounces.
    let mut key_a_idle = true;
    let mut delays = Samples::new(SUMMARY_EVERY);
    let mut failing = false;
    loop {
        interrupt.wait_for_low().await;
        loop {
            let levels = match expander.buttons() {
                Ok(levels) => {
                    if failing {
                        info!("MCP23017 reads again");
                        failing = false;
                    }
                    levels
                }
                Err(e) => {
                    if !failing {
                        warn!("MCP23017 read failed: {e:?}; retrying every 100 ms");
                        failing = true;
                    }
                    Timer::after(RETRY_AFTER_FAILURE).await;
                    // The last good read cleared the interrupt, so a button still settling
                    // would never raise it again: keep reading until it has counted.
                    if debouncers.iter().all(Debouncer::settled) {
                        break;
                    }
                    continue;
                }
            };
            let now_us = Instant::now().as_micros();
            if levels[KEY_A] && key_a_idle {
                key_a_idle = false;
                record_reference_delay(now_us, &mut delays);
            }
            for ((button, level), debouncer) in BUTTONS.iter().zip(levels).zip(&mut debouncers) {
                if let Some(edge) = debouncer.update(level, now_us / 1000) {
                    let pressed = edge == Edge::Pressed;
                    if *button == Button::KeyA && !pressed {
                        // Stamps from this press's bounces must not time the next press.
                        key_a_idle = true;
                        REFERENCE_EDGE_US.lock(|edge| edge.set(None));
                    }
                    info!(
                        "{button:?} {}",
                        if pressed { "pressed" } else { "released" }
                    );
                    let _ = EDGES.try_send((*button, pressed));
                }
            }
            if debouncers.iter().all(Debouncer::settled) {
                break;
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
