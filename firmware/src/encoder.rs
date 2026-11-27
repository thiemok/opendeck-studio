//! Turns the encoder's count into detents and hands them to the link (KTD23).
//!
//! The board decodes the two phases without losing an edge while a blocking key draw stalls
//! the executor: the Feather in its pulse counter, the P4-ETH in the GPIO interrupt
//! (THI-84 KTD13). A 5 ms task turns the count into detents and adds them to a pending total
//! that the link drains, one `KEY-ROTATE` per detent.

use crate::board::{CLOCKWISE_SIGN, EncoderCount};
use core::cell::Cell;
use deck_core::encoder::DetentCounter;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Ticker};
use log::info;

/// Encoder calibration, recorded in `docs/notes/oled-encoder-module.md`.
pub mod calibration {
    /// Quadrature counts per detent: the EC11 has 20 pulses and 20 detents per turn.
    pub const COUNTS_PER_DETENT: i16 = 4;
}

/// The count resets to zero at ± this value; a multiple of the counts per detent (KTD23).
pub const LIMIT: i16 = 4_000;
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

#[embassy_executor::task]
pub async fn encoder_task(source: EncoderCount) {
    let mut counter = DetentCounter::new(calibration::COUNTS_PER_DETENT, LIMIT, source.count());
    let mut ticker = Ticker::every(POLL);
    loop {
        ticker.next().await;
        let detents = counter.update(source.count()) * CLOCKWISE_SIGN;
        if detents != 0 {
            info!("encoder {detents:+}");
            PENDING.lock(|p| p.set(p.get() + detents));
            TURNED.signal(());
        }
    }
}
