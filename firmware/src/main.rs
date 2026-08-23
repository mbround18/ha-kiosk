mod audio;
mod debug_log;
mod display;
mod ha;
mod net;
mod portal_api;
mod storage;

use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{
    gpio_get_level, gpio_mode_t_GPIO_MODE_INPUT, gpio_pull_mode_t_GPIO_PULLUP_ONLY,
    gpio_set_direction, gpio_set_pull_mode,
};

/// FR-012 / data-model.md: the user must always have a way to re-enter setup mode, even when
/// WiFi/HA/dashboard are already configured. Holding GPIO0 (the standard "BOOT" button present
/// on essentially every Espressif dev board, including this one) low at boot is that trigger.
/// GPIO0 is only meaningful to the ROM bootloader before our app starts; reading it here as a
/// plain input afterward is the standard, well-established technique for reusing it as a user
/// button. Uses the raw ESP-IDF GPIO calls directly rather than the Rust HAL's `PinDriver`
/// (which needs to take ownership of the `gpio0` field out of `Peripherals`, entangling this
/// one-shot boot-time read with the ownership of the whole `Peripherals` struct that
/// `net::ap_portal::run` also needs).
const REPROVISION_BUTTON_GPIO: i32 = 0;

fn reprovision_button_held() -> bool {
    unsafe {
        gpio_set_direction(REPROVISION_BUTTON_GPIO, gpio_mode_t_GPIO_MODE_INPUT);
        gpio_set_pull_mode(REPROVISION_BUTTON_GPIO, gpio_pull_mode_t_GPIO_PULLUP_ONLY);
    }
    std::thread::sleep(Duration::from_millis(50)); // debounce
    unsafe { gpio_get_level(REPROVISION_BUTTON_GPIO) == 0 }
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    debug_log::install();
    debug_log::event("app_main: boot");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let force_setup = reprovision_button_held();
    if force_setup {
        log::info!("BOOT button held at startup - forcing captive portal setup mode");
    }

    // data-model.md boot decision: skip setup only once WiFi/Ethernet AND Home Assistant AND a
    // dashboard are all configured - checking wifi_configured alone (as this used to) left a
    // device with WiFi but no HA/dashboard yet stranded with no way to reach that step at all,
    // since `net::ap_portal::run` used to tear its whole HTTP server down the instant WiFi
    // joined. Ethernet link detection is stubbed (net::ethernet) pending board-specific PHY
    // verification, so today this always evaluates the network-not-up branch of that OR; once
    // Ethernet is wired up, an already-up link still can't skip HA/dashboard setup on its own.
    let setup_state = {
        let storage = storage::Storage::new(nvs.clone())?;
        storage.setup_state()
    };
    let network_up = setup_state.wifi_configured || net::ethernet::link_up();
    let setup_complete =
        network_up && setup_state.ha_configured && setup_state.dashboard_selected;

    if !force_setup && setup_complete {
        log::info!("Network up and setup complete - starting kiosk mode");
        net::kiosk::run(peripherals, sys_loop, nvs)?;
        unreachable!("net::kiosk::run only returns on error, which is propagated by `?` above");
    }

    log::info!("Setup incomplete - starting captive portal / setup flow");
    net::ap_portal::run(peripherals, sys_loop, nvs)?;

    // `net::kiosk::run` needs its own fresh `Peripherals`/`EspSystemEventLoop`/
    // `EspDefaultNvsPartition` (singletons that can't cleanly be re-taken from the ones
    // `ap_portal::run` just consumed), so rather than threading ownership through a hand-off,
    // just reboot into the fast path above - a standard, robust way to switch an embedded
    // device between fundamentally different runtime modes.
    log::info!("Captive portal setup finished - rebooting into kiosk mode");
    unsafe { esp_idf_svc::sys::esp_restart() };
}
