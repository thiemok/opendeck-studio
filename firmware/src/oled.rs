//! The encoder module's 1.3" SH1106 OLED on async I²C: test pattern, status screens and
//! Companion bitmaps (KTD22, KTD26).
//!
//! Drawing only changes the in-memory frame and marks it dirty; [`Oled::flush`] sends it.
//! That split lets the link convert a bitmap inside the line callback, where the bitmap text
//! lives, and do the slow I²C write after it returns. A missing or failing panel is logged
//! and never stops the deck.

use crate::display::{DrawError, Status, is_png};
use core::convert::Infallible;
use deck_core::mono::{MonoFrame, PAGES};
use deck_core::png::PngWorkspace;
use deck_core::session::{OLED_HEIGHT, OLED_WIDTH};
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_9X15_BOLD};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Text};
use esp_hal::Async;
use esp_hal::i2c::master::{Error as I2cError, I2c};
use log::{info, warn};

/// The module ties the SH1106's address-select line low (vendor schematic).
pub const ADDRESS: u8 = 0x3C;

/// Panel calibration, recorded in `docs/notes/oled-encoder-module.md`.
pub mod calibration {
    /// The SH1106 has 132 columns of RAM; the 128-pixel glass starts at this column.
    pub const COLUMN_OFFSET: u8 = 2;
    /// I²C clock; 100 kHz is the fallback if the bench shows errors (KTD22).
    pub const I2C_KHZ: u32 = 400;
}

/// Control byte: the rest of the transfer is commands.
const COMMANDS: u8 = 0x00;
/// Control byte: the rest of the transfer is display data.
const DATA: u8 = 0x40;

/// SH1106 power-up sequence for a 128×64 panel with the internal DC-DC converter.
const INIT: [u8; 24] = [
    COMMANDS, //
    0xAE,     // display off
    0xD5, 0x80, // clock divider
    0xA8, 0x3F, // multiplex ratio: 64 rows
    0xD3, 0x00, // no vertical offset
    0x40, // start line 0
    0xAD, 0x8B, // DC-DC converter on
    0xA1, // segment remap: column 0 on the left
    0xC8, // COM scan from the bottom: row 0 at the top
    0xDA, 0x12, // alternative COM pin layout
    0x81, 0xCF, // contrast
    0xD9, 0x1F, // pre-charge period
    0xDB, 0x40, // VCOM deselect level
    0xA4, // show RAM contents
    0xA6, // not inverted
    0xAF, // display on
];

const WIDTH: usize = OLED_WIDTH as usize;

pub struct Oled {
    i2c: I2c<'static, Async>,
    frame: &'static mut MonoFrame,
    dirty: bool,
}

impl Oled {
    /// `frame` is static so the 1 KB frame never sits on a stack.
    pub fn new(i2c: I2c<'static, Async>, frame: &'static mut MonoFrame) -> Self {
        Self {
            i2c,
            frame,
            dirty: true,
        }
    }

    /// Powers the panel up; a failure is logged and drawing carries on in memory.
    pub async fn init(&mut self) {
        match self.i2c.write_async(ADDRESS, &INIT).await {
            Ok(()) => info!("OLED ready at {ADDRESS:#04x}"),
            Err(e) => warn!("OLED did not answer at {ADDRESS:#04x}: {e:?}"),
        }
    }

    /// Sends the frame if it changed since the last successful flush; after a failed write
    /// the next flush sends it again.
    pub async fn flush(&mut self) -> Result<(), I2cError> {
        if !self.dirty {
            return Ok(());
        }
        let column = calibration::COLUMN_OFFSET;
        let mut data = [0u8; 1 + WIDTH];
        data[0] = DATA;
        for page in 0..PAGES {
            let address = [
                COMMANDS,
                0xB0 | page as u8,    // page address
                column & 0x0F,        // column, low nibble
                0x10 | (column >> 4), // column, high nibble
            ];
            self.i2c.write_async(ADDRESS, &address).await?;
            data[1..].copy_from_slice(self.frame.page(page));
            self.i2c.write_async(ADDRESS, &data).await?;
        }
        self.dirty = false;
        Ok(())
    }

    /// Takes a Companion bitmap, `rgb` or `png`. A bad `rgb` bitmap is reported and the
    /// frame keeps its image; a `png` fault leaves the rows before it changed.
    pub fn bitmap(&mut self, bitmap: &str, ws: Option<&mut PngWorkspace>) -> Result<(), DrawError> {
        if is_png(bitmap) {
            let ws = ws.ok_or(DrawError::NoPngWorkspace)?;
            self.dirty = true;
            self.frame.fill_from_png(bitmap, ws).map_err(DrawError::Png)
        } else {
            self.frame.fill_from_rgb(bitmap).map_err(DrawError::Rgb)?;
            self.dirty = true;
            Ok(())
        }
    }

    /// While a bench surface is registered the encoder isn't, so the OLED names the bench
    /// (THI-76), e.g. "32 keys png" and "decode only".
    pub fn bench(&mut self, surface: &str, mode: &str) {
        self.blank();
        self.text_centered("BENCH", 20, &FONT_9X15_BOLD);
        self.text_centered(surface, 40, &FONT_6X10);
        self.text_centered(mode, 54, &FONT_6X10);
    }

    pub fn blank(&mut self) {
        self.frame.clear();
        self.dirty = true;
    }

    /// A 1 px border and a filled block in the top-left corner: a missing edge means the
    /// column offset is wrong, and a block elsewhere means the panel is mirrored.
    pub fn test_pattern(&mut self) {
        self.blank();
        let mut canvas = Canvas(self.frame);
        let _ = Rectangle::new(Point::zero(), canvas.size())
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut canvas);
        let _ = Rectangle::new(Point::new(4, 4), Size::new_equal(12))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut canvas);
        self.text_centered("OLED", 36, &FONT_9X15_BOLD);
    }

    /// The same states the keys show (KTD26), laid out for 128×64.
    pub fn status(&mut self, status: Status<'_>) {
        self.blank();
        match status {
            Status::NoWifi => self.text_centered("No Wi-Fi", 36, &FONT_9X15_BOLD),
            Status::NoHost => {
                self.text_centered("No host", 24, &FONT_9X15_BOLD);
                self.text_centered("set it over", 42, &FONT_6X10);
                self.text_centered("USB serial", 54, &FONT_6X10);
            }
            Status::Connecting { host } => {
                self.text_centered("Connecting", 26, &FONT_9X15_BOLD);
                self.text_centered(host.get(..WIDTH / 6).unwrap_or(host), 48, &FONT_6X10);
            }
        }
    }

    fn text_centered(&mut self, text: &str, y: i32, font: &'static MonoFont<'static>) {
        let style = MonoTextStyle::new(font, BinaryColor::On);
        let _ = Text::with_alignment(
            text,
            Point::new(i32::from(OLED_WIDTH) / 2, y),
            style,
            Alignment::Center,
        )
        .draw(&mut Canvas(self.frame));
        self.dirty = true;
    }
}

/// embedded-graphics view of the frame, for test pattern and status text.
struct Canvas<'a>(&'a mut MonoFrame);

impl OriginDimensions for Canvas<'_> {
    fn size(&self) -> Size {
        Size::new(u32::from(OLED_WIDTH), u32::from(OLED_HEIGHT))
    }
}

impl DrawTarget for Canvas<'_> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Infallible>
    where
        I: IntoIterator<Item = Pixel<BinaryColor>>,
    {
        for Pixel(point, color) in pixels {
            if let (Ok(x), Ok(y)) = (usize::try_from(point.x), usize::try_from(point.y)) {
                self.0.set(x, y, color.is_on());
            }
        }
        Ok(())
    }
}
