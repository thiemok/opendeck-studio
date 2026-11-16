//! Samples the five buttons and sends debounced edges to the link (KTD18, KTD24).

use deck_core::debounce::{DEFAULT_INTERVAL_MS, Debouncer, Edge};
use deck_core::session::Control;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Ticker};
use esp_hal::gpio::Input;
use log::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    KeyA,
    KeyB,
    EncoderPush,
    Back,
    Confirm,
}

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

/// Every button reads low while pressed: the keys through the internal pull-up, the module's
/// buttons through its own 10 kΩ pull-ups.
const ACTIVE_LOW: bool = true;
const SAMPLE_PERIOD: Duration = Duration::from_millis(2);
const BUTTONS: usize = 5;

/// Debounced edges for the link: `true` = pressed. Full means the link is busy; an edge is
/// then dropped, which matches "never queued" for a link that isn't registered.
pub static EDGES: Channel<CriticalSectionRawMutex, (Button, bool), 16> = Channel::new();

#[embassy_executor::task]
pub async fn inputs_task(pins: [(Button, Input<'static>); BUTTONS]) {
    let mut debouncers: [Debouncer; BUTTONS] =
        core::array::from_fn(|_| Debouncer::new(DEFAULT_INTERVAL_MS));
    let mut ticker = Ticker::every(SAMPLE_PERIOD);
    loop {
        ticker.next().await;
        let now = Instant::now().as_millis();
        for ((button, pin), debouncer) in pins.iter().zip(debouncers.iter_mut()) {
            let pressed = pin.is_low() == ACTIVE_LOW;
            if let Some(edge) = debouncer.update(pressed, now) {
                let pressed = edge == Edge::Pressed;
                info!(
                    "{button:?} {}",
                    if pressed { "pressed" } else { "released" }
                );
                let _ = EDGES.try_send((*button, pressed));
            }
        }
    }
}
