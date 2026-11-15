//! The ScreenKey's ST7735 panel: test pattern, status screens and Companion bitmaps (KTD15, KTD17).

use deck_core::bitmap::{self, BitmapError};
use deck_core::session::KEY_SIZE;
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_9X15_BOLD};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::pixelcolor::raw::RawU16;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Text};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::Blocking;
use esp_hal::delay::Delay;
use esp_hal::gpio::Output;
use esp_hal::spi::master::Spi;
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

type SpiDevice = ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, Delay>;
type Panel =
    mipidsi::Display<SpiInterface<'static, SpiDevice, Output<'static>>, ST7735s, Output<'static>>;

/// How many `FONT_6X10` characters fit across the 128 px key.
const HOST_CHARS_PER_LINE: usize = 21;

/// What the key shows while it is not registered with Companion (KTD17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status<'a> {
    NoWifi,
    NoHost,
    Connecting { host: &'a str },
}

pub struct KeyDisplay {
    panel: Panel,
}

impl KeyDisplay {
    /// Initialises the panel; `buffer` batches pixel bytes per SPI transfer.
    pub fn new(
        spi: Spi<'static, Blocking>,
        cs: Output<'static>,
        dc: Output<'static>,
        rst: Output<'static>,
        buffer: &'static mut [u8],
    ) -> Self {
        let device = match ExclusiveDevice::new(spi, cs, Delay::new()) {
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
            .reset_pin(rst)
            .init(&mut Delay::new())
            .unwrap_or_else(|e| panic!("ST7735 init failed: {e:?}"));
        Self { panel }
    }

    /// The calibration pattern: a 1 px white border and red/green/blue/white corner blocks
    /// (top-left red, top-right green, bottom-left blue, bottom-right white). If a border edge
    /// is missing, the offset is wrong; if red shows blue, the colour order is.
    pub fn test_pattern(&mut self) {
        self.fill(Rgb565::BLACK);
        let last = i32::from(KEY_SIZE) - 1;
        let _ = Rectangle::new(Point::zero(), Size::new_equal(u32::from(KEY_SIZE)))
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::WHITE, 1))
            .draw(&mut self.panel);
        for (corner, color) in [
            (Point::new(4, 4), Rgb565::RED),
            (Point::new(last - 27, 4), Rgb565::GREEN),
            (Point::new(4, last - 27), Rgb565::BLUE),
            (Point::new(last - 27, last - 27), Rgb565::WHITE),
        ] {
            let _ = Rectangle::new(corner, Size::new_equal(24))
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(&mut self.panel);
        }
        self.text_centered("TEST", 64, &FONT_9X15_BOLD);
    }

    pub fn status(&mut self, status: Status<'_>) {
        self.fill(Rgb565::BLACK);
        match status {
            Status::NoWifi => self.text_centered("No Wi-Fi", 60, &FONT_9X15_BOLD),
            Status::NoHost => {
                self.text_centered("No host", 52, &FONT_9X15_BOLD);
                self.text_centered("set it over", 76, &FONT_6X10);
                self.text_centered("USB serial", 88, &FONT_6X10);
            }
            Status::Connecting { host } => {
                self.text_centered("Connecting", 52, &FONT_9X15_BOLD);
                self.text_centered(
                    host.get(..HOST_CHARS_PER_LINE).unwrap_or(host),
                    78,
                    &FONT_6X10,
                );
            }
        }
    }

    /// Draws a Companion `rgb` bitmap. A bad bitmap is reported, not drawn.
    pub fn bitmap(&mut self, b64: &str) -> Result<(), BitmapError> {
        let pixels = bitmap::rgb565_pixels(b64, KEY_SIZE, KEY_SIZE)?;
        let end = KEY_SIZE - 1;
        let _ = self
            .panel
            .set_pixels(0, 0, end, end, pixels.map(|p| Rgb565::from(RawU16::new(p))));
        Ok(())
    }

    pub fn blank(&mut self) {
        self.fill(Rgb565::BLACK);
    }

    fn fill(&mut self, color: Rgb565) {
        let _ = self.panel.clear(color);
    }

    fn text_centered(&mut self, text: &str, y: i32, font: &'static MonoFont<'static>) {
        let style = MonoTextStyle::new(font, Rgb565::WHITE);
        let _ = Text::with_alignment(
            text,
            Point::new(i32::from(KEY_SIZE) / 2, y),
            style,
            Alignment::Center,
        )
        .draw(&mut self.panel);
    }
}
