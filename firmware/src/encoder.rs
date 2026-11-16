//! Counts encoder detents in hardware and hands them to the link (KTD23).
//!
//! One PCNT unit decodes the two phases in quadrature, so no edge is lost while a blocking
//! key draw stalls the executor. A 5 ms task turns the count into detents and adds them to a
//! pending total that the link drains, one `KEY-ROTATE` per detent.

use core::cell::Cell;
use deck_core::encoder::DetentCounter;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Ticker};
use esp_hal::gpio::Input;
use esp_hal::pcnt::channel::{CtrlMode, EdgeMode};
use esp_hal::pcnt::unit::Unit;
use log::info;

/// Encoder calibration, recorded in `docs/notes/oled-encoder-module.md`.
pub mod calibration {
    /// Quadrature counts per detent: the EC11 has 20 pulses and 20 detents per turn.
    pub const COUNTS_PER_DETENT: i16 = 4;
    /// `1` if the counter rises when turning clockwise, `-1` if it falls.
    pub const CLOCKWISE_SIGN: i32 = -1;
}

/// The counter resets to zero at ± this value; a multiple of the counts per detent (KTD23).
const LIMIT: i16 = 4_000;
/// 10 µs at the 80 MHz APB clock: ignores contact chatter and the ESP32's short input
/// glitches, yet passes every edge of a fast spin.
const FILTER_CYCLES: u16 = 800;
const POLL: Duration = Duration::from_millis(5);

/// Detents not yet sent: positive clockwise.
static PENDING: Mutex<CriticalSectionRawMutex, Cell<i32>> = Mutex::new(Cell::new(0));
/// Raised whenever `PENDING` changes, so the link wakes at once.
pub static TURNED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Takes up to `max` detents in either direction; the rest stay pending and `TURNED` is
/// raised again so the link comes back for them.
pub fn take_pending(max: u32) -> i32 {
    let max = i32::try_from(max).unwrap_or(i32::MAX);
    let (taken, rest) = PENDING.lock(|p| {
        let pending = p.get();
        let taken = pending.clamp(-max, max);
        p.set(pending - taken);
        (taken, pending - taken)
    });
    if rest != 0 {
        TURNED.signal(());
    }
    taken
}

/// Configures `unit` to decode the two phases in quadrature and starts counting.
pub fn configure(unit: &Unit<'static, 0>, phase_a: &Input<'static>, phase_b: &Input<'static>) {
    let _ = unit.set_low_limit(Some(-LIMIT));
    let _ = unit.set_high_limit(Some(LIMIT));
    let _ = unit.set_filter(Some(FILTER_CYCLES));
    unit.clear();

    let a = phase_a.peripheral_input();
    let b = phase_b.peripheral_input();
    let ch0 = &unit.channel0;
    ch0.set_ctrl_signal(a.clone());
    ch0.set_edge_signal(b.clone());
    ch0.set_ctrl_mode(CtrlMode::Reverse, CtrlMode::Keep);
    ch0.set_input_mode(EdgeMode::Increment, EdgeMode::Decrement);
    let ch1 = &unit.channel1;
    ch1.set_ctrl_signal(b);
    ch1.set_edge_signal(a);
    ch1.set_ctrl_mode(CtrlMode::Reverse, CtrlMode::Keep);
    ch1.set_input_mode(EdgeMode::Decrement, EdgeMode::Increment);
    unit.resume();
}

/// The phase inputs are held here so they stay configured for the unit's lifetime.
#[embassy_executor::task]
pub async fn encoder_task(unit: Unit<'static, 0>, _phases: [Input<'static>; 2]) {
    let mut counter = DetentCounter::new(calibration::COUNTS_PER_DETENT, LIMIT, unit.value());
    let mut ticker = Ticker::every(POLL);
    loop {
        ticker.next().await;
        let detents = counter.update(unit.value()) * calibration::CLOCKWISE_SIGN;
        if detents != 0 {
            info!("encoder {detents:+}");
            PENDING.lock(|p| p.set(p.get() + detents));
            TURNED.signal(());
        }
    }
}
