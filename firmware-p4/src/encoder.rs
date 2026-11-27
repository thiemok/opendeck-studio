//! Counts encoder detents and hands them to the link (PoC plan KTD23, THI-84 KTD13).
//!
//! esp-hal has no PCNT driver for the ESP32-P4, so the GPIO interrupt reads both phases on
//! every edge and steps a [`Quadrature`] decoder. The interrupt runs even while a blocking key
//! draw stalls the executor, so no edge is lost. A 5 ms task turns the count into detents
//! and adds them to a pending total that the link drains, one `KEY-ROTATE` per detent.

use core::cell::{Cell, RefCell};
use deck_core::encoder::DetentCounter;
use deck_core::quadrature::Quadrature;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Ticker};
use esp_hal::gpio::{Event, Input};
use log::info;

/// Encoder calibration, recorded in `docs/notes/oled-encoder-module.md`.
pub mod calibration {
    /// Quadrature counts per detent: the EC11 has 20 pulses and 20 detents per turn.
    pub const COUNTS_PER_DETENT: i16 = 4;
    /// `1` if the count rises when turning clockwise, `-1` if it falls.
    pub const CLOCKWISE_SIGN: i32 = -1;
}

/// The count resets to zero at ± this value; a multiple of the counts per detent (KTD23).
const LIMIT: i16 = 4_000;
/// Edges closer than this to the last accepted one are contact bounce. A fast spin of five
/// turns a second still leaves about 2.5 ms between edges.
const MIN_EDGE_INTERVAL_US: u64 = 200;
const POLL: Duration = Duration::from_millis(5);

/// Detents not yet sent: positive clockwise.
static PENDING: Mutex<CriticalSectionRawMutex, Cell<i32>> = Mutex::new(Cell::new(0));
/// Raised whenever `PENDING` changes, so the link wakes at once.
pub static TURNED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// The phase inputs and their decoder, owned by the GPIO interrupt handler.
static DECODER: Mutex<CriticalSectionRawMutex, RefCell<Option<Decoder>>> =
    Mutex::new(RefCell::new(None));

struct Decoder {
    phases: [Input<'static>; 2],
    quadrature: Quadrature,
}

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

/// Starts decoding. The GPIO interrupt handler must already be installed
/// ([`crate::inputs::install_reference`]); it calls [`on_gpio_interrupt`].
pub fn install(mut phase_a: Input<'static>, mut phase_b: Input<'static>) {
    DECODER.lock(|slot| {
        let quadrature = Quadrature::new(
            phase_a.is_high(),
            phase_b.is_high(),
            LIMIT,
            MIN_EDGE_INTERVAL_US,
        );
        phase_a.listen(Event::AnyEdge);
        phase_b.listen(Event::AnyEdge);
        slot.replace(Some(Decoder {
            phases: [phase_a, phase_b],
            quadrature,
        }));
    });
}

/// Steps the decoder if either phase changed. Clears only the phases' interrupt status, so
/// other pins sharing the handler keep theirs.
pub fn on_gpio_interrupt() {
    DECODER.lock(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(decoder) = slot.as_mut() else {
            return;
        };
        let [a, b] = &mut decoder.phases;
        if !a.is_interrupt_set() && !b.is_interrupt_set() {
            return;
        }
        a.clear_interrupt();
        b.clear_interrupt();
        let now = Instant::now().as_micros();
        decoder.quadrature.update(a.is_high(), b.is_high(), now);
    });
}

fn count() -> i16 {
    DECODER.lock(|slot| slot.borrow().as_ref().map_or(0, |d| d.quadrature.count()))
}

#[embassy_executor::task]
pub async fn encoder_task() {
    let mut counter = DetentCounter::new(calibration::COUNTS_PER_DETENT, LIMIT, count());
    let mut ticker = Ticker::every(POLL);
    loop {
        ticker.next().await;
        let detents = counter.update(count()) * calibration::CLOCKWISE_SIGN;
        if detents != 0 {
            info!("encoder {detents:+}");
            PENDING.lock(|p| p.set(p.get() + detents));
            TURNED.signal(());
        }
    }
}
