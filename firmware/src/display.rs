//! The ScreenKeys' ST7735 panels on one shared SPI bus: test pattern, status screens and
//! Companion bitmaps (KTD15, KTD17, KTD21).
//!
//! Each panel's CS sits on the MCP23017. The caller holds it low for a whole draw, not per
//! SPI transaction, which would cost two I²C writes for each of the 30-plus transactions in
//! a frame (THI-89 KTD3). Drawing stays synchronous; selecting is async and happens around
//! it.

use core::cell::RefCell;
use core::convert::Infallible;
use deck_core::bitmap::{self, BitmapError};
use deck_core::png::{self, PngError, PngWorkspace};
use deck_core::session::KEY_SIZE;
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_9X15_BOLD};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::pixelcolor::raw::RawU16;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Text};
use embedded_hal::digital::{ErrorType, OutputPin};
use embedded_hal_bus::spi::RefCellDevice;
use esp_hal::Blocking;
use esp_hal::delay::Delay;
use esp_hal::gpio::Output;
use esp_hal::spi::master::Spi;
use mipidsi::NoResetPin;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7735s;
use mipidsi::options::{ColorInversion, ColorOrder};

/// Panel calibration, recorded in `docs/notes/deck-wiring.md`.
pub mod calibration {
    use super::{ColorInversion, ColorOrder};

    /// Where the 128×128 window sits inside the controller's 132×162 RAM.
    pub const OFFSET: (u16, u16) = (2, 1);
    pub const COLOR_ORDER: ColorOrder = ColorOrder::Bgr;
    pub const INVERSION: ColorInversion = ColorInversion::Inverted;
}

pub type SpiBus = Spi<'static, Blocking>;
type SpiDevice = RefCellDevice<'static, SpiBus, NoCs, Delay>;
type Panel = mipidsi::Display<SpiInterface<'static, SpiDevice, SharedPin>, ST7735s, NoResetPin>;

/// How many `FONT_6X10` characters fit across the 128 px key.
const HOST_CHARS_PER_LINE: usize = 21;

/// Why a Companion bitmap was not (fully) drawn.
#[derive(Debug)]
#[allow(dead_code, reason = "the fields are read through Debug, in the log")]
pub enum DrawError {
    Rgb(BitmapError),
    Png(PngError),
    /// A `png` bitmap arrived but the decoder's memory could not be allocated.
    NoPngWorkspace,
    /// The expander didn't take the chip select, so nothing was sent to the panel.
    NotSelected,
    /// The line held for the draw no longer parsed to a bitmap; it was already parsed once,
    /// so this means the framer's buffer changed underneath.
    NotHeld,
}

/// Whether a `BITMAP` value is a PNG data URL rather than base64 `rgb`.
pub fn is_png(bitmap: &str) -> bool {
    bitmap.starts_with(png::DATA_URL_PREFIX)
}

/// Decodes a 128×128 key bitmap without drawing it, for the bench's decode-only mode
/// (THI-76). The checksum keeps the compiler from dropping the decode.
pub fn decode_only(bitmap: &str, ws: Option<&mut PngWorkspace>) -> Result<u32, DrawError> {
    if is_png(bitmap) {
        let ws = ws.ok_or(DrawError::NoPngWorkspace)?;
        let mut sum = 0u32;
        png::decode(bitmap, KEY_SIZE, KEY_SIZE, ws, |_, rgb| {
            for px in rgb.as_chunks::<3>().0 {
                sum = sum.rotate_left(1) ^ u32::from(bitmap::rgb565(*px));
            }
        })
        .map_err(DrawError::Png)?;
        Ok(sum)
    } else {
        let pixels = bitmap::rgb565_pixels(bitmap, KEY_SIZE, KEY_SIZE).map_err(DrawError::Rgb)?;
        Ok(pixels.fold(0u32, |sum, p| sum.rotate_left(1) ^ u32::from(p)))
    }
}

/// What the displays show while the deck is not registered with Companion (KTD17, KTD26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status<'a> {
    /// No address yet; shows [`crate::NO_NETWORK`].
    NoNetwork,
    NoHost,
    Connecting {
        host: &'a str,
    },
}

/// An output pin shared by both panels' interfaces: the DC line (KTD21). Only the link task
/// draws, so the `RefCell` is never contended.
pub struct SharedPin(&'static RefCell<Output<'static>>);

impl SharedPin {
    pub fn new(pin: &'static RefCell<Output<'static>>) -> Self {
        Self(pin)
    }
}

impl ErrorType for SharedPin {
    type Error = Infallible;
}

impl OutputPin for SharedPin {
    fn set_low(&mut self) -> Result<(), Infallible> {
        self.0.borrow_mut().set_low();
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Infallible> {
        self.0.borrow_mut().set_high();
        Ok(())
    }
}

/// The SPI device's chip select. The real CS is on the expander and is held for a whole
/// draw by the caller, so per-transaction CS does nothing.
pub struct NoCs;

impl ErrorType for NoCs {
    type Error = Infallible;
}

impl OutputPin for NoCs {
    fn set_low(&mut self) -> Result<(), Infallible> {
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

pub struct KeyDisplay {
    panel: Panel,
}

impl KeyDisplay {
    /// Initialises one panel on the shared bus; `buffer` batches pixel bytes per SPI transfer.
    /// The caller selects the panel first.
    pub fn new(bus: &'static RefCell<SpiBus>, dc: SharedPin, buffer: &'static mut [u8]) -> Self {
        let device = match RefCellDevice::new(bus, NoCs, Delay::new()) {
            Ok(device) => device,
            Err(never) => match never {},
        };
        let interface = SpiInterface::new(device, dc, buffer);
        let (x, y) = calibration::OFFSET;
        let panel = mipidsi::Builder::new(ST7735s, interface)
            .display_size(KEY_SIZE, KEY_SIZE)
            .display_offset(x, y)
            .color_order(calibration::COLOR_ORDER)
            .invert_colors(calibration::INVERSION)
            .init(&mut Delay::new())
            .unwrap_or_else(|e| panic!("ST7735 init failed: {e:?}"));
        Self { panel }
    }

    /// The calibration pattern: a 1 px white border and red/green/blue/white corner blocks
    /// (top-left red, top-right green, bottom-left blue, bottom-right white). If a border edge
    /// is missing, the offset is wrong; if red shows blue, the colour order is.
    pub fn test_pattern(&mut self, label: &str) {
        let panel = &mut self.panel;
        fill(panel, Rgb565::BLACK);
        let last = i32::from(KEY_SIZE) - 1;
        let _ = Rectangle::new(Point::zero(), Size::new_equal(u32::from(KEY_SIZE)))
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::WHITE, 1))
            .draw(panel);
        for (corner, color) in [
            (Point::new(4, 4), Rgb565::RED),
            (Point::new(last - 27, 4), Rgb565::GREEN),
            (Point::new(4, last - 27), Rgb565::BLUE),
            (Point::new(last - 27, last - 27), Rgb565::WHITE),
        ] {
            let _ = Rectangle::new(corner, Size::new_equal(24))
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(panel);
        }
        text_centered(panel, label, 64, &FONT_9X15_BOLD);
    }

    pub fn status(&mut self, status: Status<'_>) {
        let panel = &mut self.panel;
        fill(panel, Rgb565::BLACK);
        match status {
            Status::NoNetwork => text_centered(panel, crate::NO_NETWORK, 60, &FONT_9X15_BOLD),
            Status::NoHost => {
                text_centered(panel, "No host", 52, &FONT_9X15_BOLD);
                text_centered(panel, "set it over", 76, &FONT_6X10);
                text_centered(panel, "USB serial", 88, &FONT_6X10);
            }
            Status::Connecting { host } => {
                text_centered(panel, "Connecting", 52, &FONT_9X15_BOLD);
                text_centered(
                    panel,
                    host.get(..HOST_CHARS_PER_LINE).unwrap_or(host),
                    78,
                    &FONT_6X10,
                );
            }
        }
    }

    /// Draws a Companion bitmap, `rgb` or `png`. A bad `rgb` bitmap is reported and not
    /// drawn; a `png` fault is found as rows arrive, so the rows before it stay drawn.
    pub fn bitmap(&mut self, bitmap: &str, ws: Option<&mut PngWorkspace>) -> Result<(), DrawError> {
        let end = KEY_SIZE - 1;
        if !is_png(bitmap) {
            let pixels =
                bitmap::rgb565_pixels(bitmap, KEY_SIZE, KEY_SIZE).map_err(DrawError::Rgb)?;
            let _ =
                self.panel
                    .set_pixels(0, 0, end, end, pixels.map(|p| Rgb565::from(RawU16::new(p))));
            return Ok(());
        }
        let ws = ws.ok_or(DrawError::NoPngWorkspace)?;
        let panel = &mut self.panel;
        png::decode(bitmap, KEY_SIZE, KEY_SIZE, ws, |y, rgb| {
            let pixels = rgb
                .as_chunks::<3>()
                .0
                .iter()
                .map(|px| Rgb565::from(RawU16::new(bitmap::rgb565(*px))));
            let _ = panel.set_pixels(0, y, end, y, pixels);
        })
        .map_err(DrawError::Png)
    }

    pub fn blank(&mut self) {
        fill(&mut self.panel, Rgb565::BLACK);
    }
}

fn fill(panel: &mut Panel, color: Rgb565) {
    let _ = panel.clear(color);
}

fn text_centered(panel: &mut Panel, text: &str, y: i32, font: &'static MonoFont<'static>) {
    let style = MonoTextStyle::new(font, Rgb565::WHITE);
    let _ = Text::with_alignment(
        text,
        Point::new(i32::from(KEY_SIZE) / 2, y),
        style,
        Alignment::Center,
    )
    .draw(panel);
}
