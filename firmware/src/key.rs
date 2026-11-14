//! Samples the ScreenKey switch and sends debounced edges to the link (KTD18).

use deck_core::debounce::{DEFAULT_INTERVAL_MS, Debouncer, Edge};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Ticker};
use esp_hal::gpio::Input;
use log::info;

/// The KEY line reads low while pressed (internal pull-up). Confirmed on the bench in U8.
pub const ACTIVE_LOW: bool = true;
const SAMPLE_PERIOD: Duration = Duration::from_millis(2);

/// Debounced edges for the link: `true` = pressed. Full means the link is busy; an edge
/// is then dropped, which matches "never queued" for a link that isn't registered.
pub static EDGES: Channel<CriticalSectionRawMutex, bool, 8> = Channel::new();

#[embassy_executor::task]
pub async fn key_task(pin: Input<'static>) {
    let mut debouncer = Debouncer::new(DEFAULT_INTERVAL_MS);
    let mut ticker = Ticker::every(SAMPLE_PERIOD);
    loop {
        ticker.next().await;
        let pressed = pin.is_low() == ACTIVE_LOW;
        if let Some(edge) = debouncer.update(pressed, Instant::now().as_millis()) {
            info!(
                "key {}",
                if edge == Edge::Pressed {
                    "pressed"
                } else {
                    "released"
                }
            );
            let _ = EDGES.try_send(edge == Edge::Pressed);
        }
    }
}
