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

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let force_setup = reprovision_button_held();
    if force_setup {
        log::info!("BOOT button held at startup - forcing captive portal setup mode");
    }

    // data-model.md boot decision: enter the AP/captive-portal only when there's no working
    // network path yet. Ethernet link detection is stubbed (net::ethernet) pending board-
    // specific PHY verification, so today this always evaluates the WiFi-not-configured branch;
    // once Ethernet is wired up, an already-up link will skip straight past this.
    let wifi_already_configured = {
        let storage = storage::Storage::new(nvs.clone())?;
        storage.setup_state().wifi_configured
    };

    if !force_setup && (net::ethernet::link_up() || wifi_already_configured) {
        log::info!("Network already configured or Ethernet is up - normal operation (display/UI not yet implemented)");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    }

    log::info!("No working network - starting captive portal setup");
    net::ap_portal::run(peripherals, sys_loop, nvs)?;

    log::info!("Captive portal setup finished - continuing to normal operation");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}
