//! The MCP23017 I/O expander that carries the buttons, the key chip selects and the key
//! reset (THI-89, KTD2 in `docs/plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md`).
//!
//! Registers use the power-on layout (IOCON.BANK = 0), where port A and port B registers
//! alternate.
//!
//! Port A drives outputs: GPA0 is key A's CS, GPA1 key B's CS, GPA2 the keys' shared RST.
//! Port B reads the five buttons, GPB0–GPB4, each low while pressed. GPA7 and GPB7 are
//! output-only on current chips; they are made outputs so they never float, and stay
//! unconnected.

use crate::session::Control;

/// The breakout's address with its three address jumpers open.
pub const ADDRESS: u8 = 0x20;

/// Register addresses with IOCON.BANK = 0.
pub mod reg {
    pub const IODIRA: u8 = 0x00;
    pub const IODIRB: u8 = 0x01;
    pub const IPOLB: u8 = 0x03;
    pub const GPINTENB: u8 = 0x05;
    pub const INTCONB: u8 = 0x09;
    pub const IOCON: u8 = 0x0A;
    pub const GPPUB: u8 = 0x0D;
    pub const GPIOB: u8 = 0x13;
    pub const OLATA: u8 = 0x14;
}

const CS_A: u8 = 1 << 0;
const CS_B: u8 = 1 << 1;
const RST: u8 = 1 << 2;
const OUTPUTS: u8 = CS_A | CS_B | RST;
/// GPA7 and GPB7, output-only on current chips.
const PIN_7: u8 = 1 << 7;

pub const BUTTON_COUNT: usize = 5;
const BUTTON_PINS: u8 = (1 << BUTTON_COUNT) - 1;

/// What [`INIT`] writes to IODIRA. The chip resets to all inputs, so reading anything else
/// back means it lost power.
pub const IODIRA_INIT: u8 = !(OUTPUTS | PIN_7);

/// Register writes, as `(register, value)`, that bring the expander up: outputs latched
/// high before they are driven, so no panel is selected or held in reset, and port B
/// interrupting on any button change. Read [`reg::GPIOB`] afterwards to clear a pending
/// interrupt.
pub const INIT: [(u8, u8); 8] = [
    // BANK = 0, interrupt pins separate, sequential addressing, push-pull, active low.
    (reg::IOCON, 0x00),
    (reg::OLATA, OUTPUTS),
    (reg::IODIRA, IODIRA_INIT),
    (reg::IODIRB, !PIN_7),
    (reg::IPOLB, 0x00),
    // Every button gets a pull-up. The keys need one; the module's buttons have their own
    // 10 kΩ ones, but read pressed while the module is unplugged without this.
    (reg::GPPUB, BUTTON_PINS),
    // Interrupt on change against the previous value, not against DEFVAL.
    (reg::INTCONB, 0x00),
    (reg::GPINTENB, BUTTON_PINS),
];

/// The deck's five buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    KeyA,
    KeyB,
    EncoderPush,
    Back,
    Confirm,
}

/// The buttons in port-B bit order: `BUTTONS[n]` is wired to GPBn.
pub const BUTTONS: [Button; BUTTON_COUNT] = [
    Button::KeyA,
    Button::KeyB,
    Button::EncoderPush,
    Button::Back,
    Button::Confirm,
];

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

/// Which key panel a chip select addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    A,
    B,
}

/// The key panel a control's images are drawn on. Bench keys `0/0` and `0/1` are the
/// physical keys; the others are only decoded, unless `draw` sends them to panel A (even)
/// or B (odd) as well, adding the SPI time a real deck would spend (THI-76).
pub fn key_for(control: Control, draw: bool) -> Option<Key> {
    match control {
        Control::KeyA | Control::Bench(0) => Some(Key::A),
        Control::KeyB | Control::Bench(1) => Some(Key::B),
        Control::Bench(n) if draw && n % 2 == 0 => Some(Key::A),
        Control::Bench(_) if draw => Some(Key::B),
        Control::Bench(_) | Control::Encoder => None,
    }
}

/// The port-A output latch, so changing one line never clobbers another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortA {
    latch: u8,
}

impl Default for PortA {
    fn default() -> Self {
        Self::new()
    }
}

impl PortA {
    /// The state [`INIT`] leaves: both CS high (not selected), RST high (not in reset).
    pub const fn new() -> Self {
        Self { latch: OUTPUTS }
    }

    /// The latch as it is, for writing it again after the chip was reconfigured.
    pub fn byte(&self) -> u8 {
        self.latch
    }

    /// Selects `key` alone; returns the byte to write to [`reg::OLATA`].
    pub fn select(&mut self, key: Key) -> u8 {
        let cs = match key {
            Key::A => CS_A,
            Key::B => CS_B,
        };
        self.latch = (self.latch | CS_A | CS_B) & !cs;
        self.latch
    }

    /// Deselects both keys; returns the byte to write to [`reg::OLATA`].
    pub fn deselect(&mut self) -> u8 {
        self.latch |= CS_A | CS_B;
        self.latch
    }

    /// Drives the shared RST low (`true`) or releases it; returns the byte to write.
    pub fn reset(&mut self, asserted: bool) -> u8 {
        if asserted {
            self.latch &= !RST;
        } else {
            self.latch |= RST;
        }
        self.latch
    }
}

/// Turns a [`reg::GPIOB`] byte into the five buttons' pressed states, in [`BUTTONS`] order.
/// Every button reads low while pressed.
pub fn pressed(port_b: u8) -> [bool; BUTTON_COUNT] {
    core::array::from_fn(|bit| port_b & (1 << bit) == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_value(register: u8) -> u8 {
        let writes: Vec<_> = INIT.iter().filter(|(r, _)| *r == register).collect();
        assert_eq!(writes.len(), 1, "register {register:#04x} written once");
        writes[0].1
    }

    #[test]
    fn init_makes_gpa0_to_gpa2_outputs() {
        let iodira = init_value(reg::IODIRA);
        assert_eq!(iodira & 0b0000_0111, 0, "GPA0–GPA2 are outputs");
    }

    #[test]
    fn init_drives_the_output_only_gpa7_and_gpb7_instead_of_leaving_them_floating() {
        assert_eq!(init_value(reg::IODIRA) & 0b1000_0000, 0);
        assert_eq!(init_value(reg::IODIRB) & 0b1000_0000, 0);
    }

    #[test]
    fn key_images_go_to_the_panel_their_control_belongs_to() {
        assert_eq!(key_for(Control::KeyA, false), Some(Key::A));
        assert_eq!(key_for(Control::KeyB, false), Some(Key::B));
        assert_eq!(
            key_for(Control::Encoder, true),
            None,
            "the encoder is on the OLED"
        );
        assert_eq!(key_for(Control::Bench(0), false), Some(Key::A));
        assert_eq!(key_for(Control::Bench(1), false), Some(Key::B));
    }

    #[test]
    fn other_bench_keys_are_drawn_on_alternate_panels_only_when_drawing() {
        assert_eq!(key_for(Control::Bench(2), false), None, "decode only");
        assert_eq!(key_for(Control::Bench(2), true), Some(Key::A));
        assert_eq!(key_for(Control::Bench(7), true), Some(Key::B));
        assert_eq!(key_for(Control::Bench(30), true), Some(Key::A));
    }

    #[test]
    fn init_reads_the_buttons_with_pull_ups_and_interrupts_on_change() {
        assert_eq!(init_value(reg::IODIRB) & 0b0111_1111, 0b0111_1111);
        assert_eq!(
            init_value(reg::GPPUB),
            0b0001_1111,
            "every button is pulled up, so an unplugged module reads released"
        );
        assert_eq!(init_value(reg::GPINTENB), 0b0001_1111);
        assert_eq!(init_value(reg::INTCONB), 0x00);
        assert_eq!(init_value(reg::IPOLB), 0x00);
    }

    #[test]
    fn init_latches_the_outputs_high_before_driving_them() {
        let latch = INIT.iter().position(|(r, _)| *r == reg::OLATA).unwrap();
        let direction = INIT.iter().position(|(r, _)| *r == reg::IODIRA).unwrap();
        assert!(latch < direction);
        assert_eq!(init_value(reg::OLATA) & 0b0000_0111, 0b0000_0111);
        assert_eq!(
            PortA::new(),
            PortA {
                latch: init_value(reg::OLATA)
            }
        );
    }

    #[test]
    fn init_enables_the_interrupt_last() {
        assert_eq!(INIT.last().unwrap().0, reg::GPINTENB);
    }

    #[test]
    fn selecting_a_key_deselects_the_other() {
        let mut port = PortA::new();
        assert_eq!(port.select(Key::A), 0b0000_0110);
        assert_eq!(port.select(Key::B), 0b0000_0101);
    }

    #[test]
    fn byte_returns_the_latch_without_changing_it() {
        let mut port = PortA::new();
        let selected = port.select(Key::B);
        assert_eq!(port.byte(), selected);
        assert_eq!(port.byte(), selected);
    }

    #[test]
    fn deselect_and_reset_leave_each_others_bits_alone() {
        let mut port = PortA::new();
        port.select(Key::A);
        assert_eq!(
            port.reset(true),
            0b0000_0010,
            "RST low, key A still selected"
        );
        assert_eq!(port.deselect(), 0b0000_0011, "both CS high, RST still low");
        assert_eq!(port.reset(false), 0b0000_0111);
    }

    #[test]
    fn each_wired_pin_reads_as_its_button() {
        // The port-B table in docs/notes/deck-wiring.md.
        let wiring = [
            (0, Button::KeyA),
            (1, Button::KeyB),
            (2, Button::EncoderPush),
            (3, Button::Back),
            (4, Button::Confirm),
        ];
        for (gpb, button) in wiring {
            let levels = pressed(!(1 << gpb));
            let down: Vec<_> = BUTTONS
                .into_iter()
                .zip(levels)
                .filter_map(|(b, down)| down.then_some(b))
                .collect();
            assert_eq!(down, [button], "GPB{gpb}");
        }
    }

    #[test]
    fn only_the_keys_and_the_encoder_push_report_to_companion() {
        assert_eq!(
            BUTTONS.map(Button::control),
            [
                Some(Control::KeyA),
                Some(Control::KeyB),
                Some(Control::Encoder),
                None,
                None,
            ]
        );
    }

    #[test]
    fn port_b_bits_map_to_the_five_buttons() {
        assert_eq!(pressed(0b1111_1110), [true, false, false, false, false]);
        assert_eq!(pressed(0b1110_0000), [true; 5]);
        assert_eq!(pressed(0b0001_1111), [false; 5], "GPB5–GPB7 are ignored");
        assert_eq!(pressed(0b1110_1111), [false, false, false, false, true]);
    }
}
