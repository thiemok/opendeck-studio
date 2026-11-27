//! Wi-Fi station with build-time credentials (KTD11) and the embassy-net stack.

use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController,
};
use log::{info, warn};
use static_cell::ConstStaticCell;

/// From `firmware/wifi.env` via `build.rs`.
const SSID: &str = env!("DECK_WIFI_SSID");
const PASSWORD: &str = env!("DECK_WIFI_PASSWORD");

/// DHCP, DNS and one TCP socket.
const SOCKETS: usize = 3;

/// Starts Wi-Fi and the network stack. Whether the deck has an address is
/// `stack.is_config_up()`.
#[allow(
    clippy::large_stack_frames,
    reason = "about 2 KB of Wi-Fi config structs, once at boot from the main task"
)]
pub fn start(spawner: Spawner, wifi: WIFI<'static>) -> Stack<'static> {
    let station = Config::Station(
        StationConfig::default()
            .with_ssid(SSID.try_into().expect("DECK_WIFI_SSID too long"))
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().expect("DECK_WIFI_PASSWORD too long"),
            )),
    );
    let controller = WifiController::new(
        wifi,
        ControllerConfig::default().with_initial_config(station),
    )
    .expect("Wi-Fi controller init failed");

    let rng = Rng::new();
    let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());
    static RESOURCES: ConstStaticCell<StackResources<SOCKETS>> =
        ConstStaticCell::new(StackResources::new());
    let (stack, runner) = embassy_net::new(
        Interface::station(),
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.take(),
        seed,
    );

    spawner.spawn(connection_task(controller).expect("spawn Wi-Fi task"));
    spawner.spawn(net_task(runner).expect("spawn net task"));
    stack
}

/// Keeps the station connected: connect, wait for a drop, retry after 5 s.
#[allow(
    clippy::large_stack_frames,
    reason = "about 1-1.6 KB, mostly log formatting; the main task stack has far more room"
)]
#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
    loop {
        match controller.connect_async().await {
            Ok(info) => {
                info!("Wi-Fi connected: {info:?}");
                let reason = controller.wait_for_disconnect_async().await.ok();
                warn!("Wi-Fi disconnected: {reason:?}");
            }
            Err(e) => warn!("Wi-Fi connect to {SSID:?} failed: {e:?}"),
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
