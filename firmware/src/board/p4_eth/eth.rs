//! Wired Ethernet on the Waveshare ESP32-P4-ETH and the embassy-net stack (THI-84 KTD8).
//!
//! esp-hal's EMAC driver runs the board's IP101GRI PHY over RMII; the PHY drives the 50 MHz
//! reference clock into GPIO50. `GenericPhy` has no reset-pin handling, so the PHY's reset
//! line (GPIO51) is pulsed here before the driver starts. The returned stack is the same
//! `Stack<'static>` the Feather's Wi-Fi gives, so `link` cannot tell them apart.
//!
//! The driver asks the MAC to insert IP, TCP and UDP checksums, but the P4's transmit FIFO is
//! too small to hold a full frame, and the MAC cannot insert checksums in a frame larger than
//! its FIFO. Such frames leave with a bad checksum and the receiver drops them: on the bench the
//! deck's 540-byte `ADD-DEVICE` never reached Companion, while every ACK did.
//! [`SoftwareTxChecksums`] has smoltcp compute transmit checksums instead; the MAC leaves a
//! correct checksum alone.

use core::task::Context;
use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources};
use embassy_net_driver::{Capabilities, Checksum, HardwareAddress, LinkState};
use embassy_time::{Duration, Timer};
use esp_hal::Async;
use esp_hal::ethernet::clock::ExternalRefClock;
use esp_hal::ethernet::phy::generic::GenericPhy;
use esp_hal::ethernet::{Ethernet, EthernetDmaStorage, RmiiPinBundle};
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::peripherals::{
    ETH, GPIO28, GPIO29, GPIO30, GPIO31, GPIO34, GPIO35, GPIO49, GPIO50, GPIO51, GPIO52,
};
use esp_hal::rng::Rng;
use log::{info, warn};
use static_cell::{ConstStaticCell, StaticCell};

/// DHCP, DNS and one TCP socket.
const SOCKETS: usize = 3;

/// How long the PHY's reset is held low. The IP101GRI needs at least 10 ms.
const PHY_RESET_TIME: Duration = Duration::from_millis(20);

type Driver = SoftwareTxChecksums<Ethernet<'static, Async, GenericPhy>>;

/// The RMII and PHY-reset pins as the P4-ETH schematic wires them; they are also esp-hal's
/// defaults for the ESP32-P4.
pub struct Pins {
    pub ref_clk: GPIO50<'static>,
    pub rxd0: GPIO29<'static>,
    pub rxd1: GPIO30<'static>,
    pub crs_dv: GPIO28<'static>,
    pub txd0: GPIO34<'static>,
    pub txd1: GPIO35<'static>,
    pub tx_en: GPIO49<'static>,
    pub mdc: GPIO31<'static>,
    pub mdio: GPIO52<'static>,
    pub phy_reset: GPIO51<'static>,
}

/// Resets the PHY, starts the EMAC and the network stack. Whether the deck has an address
/// is `stack.is_config_up()`.
pub async fn start(
    spawner: Spawner,
    eth: ETH<'static>,
    pins: Pins,
    mac: [u8; 6],
) -> Stack<'static> {
    static PHY_RESET: StaticCell<Output<'static>> = StaticCell::new();
    let reset = PHY_RESET.init(Output::new(
        pins.phy_reset,
        Level::Low,
        OutputConfig::default(),
    ));
    Timer::after(PHY_RESET_TIME).await;
    reset.set_high();
    Timer::after(PHY_RESET_TIME).await;

    static DMA: ConstStaticCell<EthernetDmaStorage<10, 10>> =
        ConstStaticCell::new(EthernetDmaStorage::new());
    let driver = Ethernet::new(
        eth,
        DMA.take(),
        mac,
        GenericPhy::new_auto(),
        RmiiPinBundle {
            clock: ExternalRefClock::new(pins.ref_clk),
            rxd0: pins.rxd0,
            rxd1: pins.rxd1,
            rx_dv: pins.crs_dv,
            txd0: pins.txd0,
            txd1: pins.txd1,
            tx_en: pins.tx_en,
            mdc: pins.mdc,
            mdio: pins.mdio,
        },
    )
    .expect("Ethernet init failed: is the PHY out of reset?")
    .into_async();
    let driver = SoftwareTxChecksums(driver);

    let rng = Rng::new();
    let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());
    static RESOURCES: ConstStaticCell<StackResources<SOCKETS>> =
        ConstStaticCell::new(StackResources::new());
    let (stack, runner) = embassy_net::new(
        driver,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.take(),
        seed,
    );

    spawner.spawn(net_task(runner).expect("spawn net task"));
    spawner.spawn(link_log_task(stack).expect("spawn link log task"));
    stack
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Driver>) {
    runner.run().await
}

/// Logs cable and DHCP changes, the Ethernet counterpart of the Wi-Fi connect log.
#[embassy_executor::task]
async fn link_log_task(stack: Stack<'static>) {
    loop {
        stack.wait_link_up().await;
        info!("Ethernet link up");
        stack.wait_config_up().await;
        if let Some(config) = stack.config_v4() {
            info!("DHCP address {}", config.address);
        }
        stack.wait_link_down().await;
        warn!("Ethernet link down");
    }
}

/// Passes a driver through, but has smoltcp compute transmit checksums rather than trusting
/// the MAC to insert them (see the module docs). Receive checks stay with the hardware.
struct SoftwareTxChecksums<D>(D);

impl<D: embassy_net_driver::Driver> embassy_net_driver::Driver for SoftwareTxChecksums<D> {
    type RxToken<'a>
        = D::RxToken<'a>
    where
        Self: 'a;
    type TxToken<'a>
        = D::TxToken<'a>
    where
        Self: 'a;

    fn receive(&mut self, cx: &mut Context) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.0.receive(cx)
    }

    fn transmit(&mut self, cx: &mut Context) -> Option<Self::TxToken<'_>> {
        self.0.transmit(cx)
    }

    fn link_state(&mut self, cx: &mut Context) -> LinkState {
        self.0.link_state(cx)
    }

    fn capabilities(&self) -> Capabilities {
        let mut caps = self.0.capabilities();
        caps.checksum.ipv4 = Checksum::Tx;
        caps.checksum.tcp = Checksum::Tx;
        caps.checksum.udp = Checksum::Tx;
        caps.checksum.icmpv4 = Checksum::Tx;
        caps
    }

    fn hardware_address(&self) -> HardwareAddress {
        self.0.hardware_address()
    }
}
