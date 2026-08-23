//! Runtime kiosk mode: once setup (WiFi/HA/dashboard) is complete, this is what actually runs on
//! the device's screen - a native per-room light on/off UI (`display::kiosk`/`ha::rooms`), not a
//! rendering of the Home Assistant dashboard the user picked during setup (see `ha::rooms`'s doc
//! comment for why that's not tractable on this hardware). `dashboard_selected` is still tracked
//! through setup for now, but isn't used here yet.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use embedded_svc::http::Method;
use embedded_svc::io::Write;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::http::server::{Configuration as HttpServerConfig, EspHttpServer};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
use esp_idf_svc::wifi::{BlockingWifi, ClientConfiguration, Configuration, EspWifi};

use crate::ha::rooms::{self, Room, Weather};
use crate::storage::Storage;

/// Poll cadence for touch input while the room list is on screen. Now that HA calls run on a
/// separate worker thread (see below) instead of blocking this loop, there's no reason this has
/// to be as slow as it used to need to be - lower is snappier-feeling touch response.
const TICK: Duration = Duration::from_millis(50);
/// How often to re-fetch rooms + weather/humidity from Home Assistant.
const ROOM_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
/// After a room's On/Off button is tapped, further taps on that same room are ignored for this
/// long (see the button-mashing note below) - long enough to comfortably cover a fresh
/// WebSocket connect+auth+service-call round trip, short enough that a legitimate follow-up tap
/// (e.g. "oops, meant Off") doesn't feel broken.
const ACTION_COOLDOWN: Duration = Duration::from_secs(2);
/// How long with no touch activity before dimming the backlight (see `Display::set_brightness`).
const IDLE_DIM_TIMEOUT: Duration = Duration::from_secs(60);
const DIM_BRIGHTNESS_PERCENT: u8 = 15;
const FULL_BRIGHTNESS_PERCENT: u8 = 100;

pub fn run(
    peripherals: Peripherals,
    sys_loop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> anyhow::Result<()> {
    let storage = Storage::new(nvs.clone())?;
    let Some(creds) = storage.get_wifi_credentials() else {
        anyhow::bail!("kiosk::run entered without saved WiFi credentials");
    };
    let Some(ha) = storage.get_ha_connection() else {
        anyhow::bail!("kiosk::run entered without a saved Home Assistant connection");
    };

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: creds.ssid.as_str().try_into().unwrap(),
        password: creds.password.as_str().try_into().unwrap(),
        auth_method: if creds.password.is_empty() {
            esp_idf_svc::wifi::AuthMethod::None
        } else {
            esp_idf_svc::wifi::AuthMethod::WPA2WPA3Personal
        },
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.connect()?;
    wifi.wait_netif_up()?;
    log::info!("kiosk: WiFi connected, IP {}", wifi.wifi().sta_netif().get_ip_info()?.ip);

    sync_time();

    // Kept alive for the rest of this function (never dropped, since `run` never returns on
    // success) so `/api/debug` (see `crate::debug_log`) stays reachable throughout kiosk mode
    // too, not just during captive-portal setup - there's otherwise no way to see what this
    // mode is doing once the setup flow's own HTTP server has gone away.
    let _debug_server = start_debug_server()?;

    let mut display = crate::display::Display::init()
        .map_err(|e| anyhow::anyhow!("kiosk: display init failed: {e}"))?;

    let mut audio = match crate::audio::Audio::init() {
        Ok(a) => Some(a),
        Err(e) => {
            log::warn!("kiosk: audio init failed, continuing without click feedback: {e}");
            None
        }
    };

    let mut rooms: Vec<Room> = Vec::new();
    let mut weather: Option<Weather> = None;
    let mut buttons: Option<crate::display::KioskButtons> = None;
    let mut last_room_refresh = std::time::Instant::now() - ROOM_REFRESH_INTERVAL;
    // Per-area "don't act on taps again until" deadlines - see `ACTION_COOLDOWN`. This is the
    // actual anti-flood guard: a child mashing one room's buttons gets every tap past the first
    // silently dropped (not queued - dropped) until the cooldown clears, so HA only ever sees
    // one call per room per cooldown window no matter how fast the taps come in.
    let mut cooldowns: HashMap<String, Instant> = HashMap::new();
    let mut last_activity = Instant::now();
    let mut dimmed = false;

    // `set_room_lights` is a fresh WebSocket connect+auth+call every time (1-2s+), and used to
    // run inline on this loop - meaning touch polling, dimming, and every other room's UI simply
    // froze for that whole stretch. A single persistent worker thread (not one thread per tap -
    // that would let a kid mashing several different rooms pile up unbounded concurrent HA
    // connections) takes it off this loop entirely, processing one action at a time while this
    // loop stays responsive to touch every `TICK` regardless of what HA is doing.
    let (action_tx, action_rx) = mpsc::channel::<(String, bool)>();
    let (result_tx, result_rx) = mpsc::channel::<(String, bool, anyhow::Result<()>)>();
    {
        let worker_ha = ha.clone();
        std::thread::Builder::new()
            .stack_size(16 * 1024)
            .spawn(move || {
                for (area_id, turn_on) in action_rx {
                    let result =
                        rooms::set_room_lights(&worker_ha.base_url, &worker_ha.access_token, &area_id, turn_on);
                    if result_tx.send((area_id, turn_on, result)).is_err() {
                        break; // kiosk::run's loop is gone
                    }
                }
            })?;
    }

    loop {
        std::thread::sleep(TICK);
        let now = Instant::now();

        if last_room_refresh.elapsed() >= ROOM_REFRESH_INTERVAL {
            match rooms::fetch_dashboard_data(&ha.base_url, &ha.access_token) {
                Ok((fetched_rooms, fetched_weather)) => {
                    crate::debug_log::event(format!(
                        "kiosk: fetched {} room(s): {:?}",
                        fetched_rooms.len(),
                        fetched_rooms.iter().map(|r| &r.name).collect::<Vec<_>>()
                    ));
                    rooms = fetched_rooms;
                    weather = fetched_weather;
                    buttons = None; // force a redraw with the fresh room list/weather
                }
                Err(e) => crate::debug_log::event(format!("kiosk: failed to fetch dashboard data: {e:?}")),
            }
            last_room_refresh = std::time::Instant::now();
        }

        // Once a cooldown expires, force a full redraw so that room's toggle visibly goes from
        // muted back to its normal color - otherwise it'd stay looking "just changed" until the
        // next room refresh even though taps would already be accepted again.
        if buttons.is_some() {
            let expired: Vec<String> = cooldowns
                .iter()
                .filter(|&(_, &until)| now >= until)
                .map(|(area_id, _)| area_id.clone())
                .collect();
            if !expired.is_empty() {
                for area_id in expired {
                    cooldowns.remove(&area_id);
                }
                buttons = None;
            }
        }

        if buttons.is_none() {
            match crate::display::show_rooms(&mut display, &rooms, weather.as_ref()) {
                Ok(b) => buttons = Some(b),
                Err(e) => log::warn!("kiosk: failed to draw room list: {e}"),
            }
        }

        // Pick up any HA call(s) the worker thread has finished since last tick. Doing this
        // separately from the cooldown timer (rather than just waiting `ACTION_COOLDOWN` out)
        // means a toggle confirms as soon as the real round trip completes - usually well under
        // 2s - instead of always waiting the full cooldown before looking "settled".
        while let Ok((area_id, turn_on, result)) = result_rx.try_recv() {
            if let Err(e) = result {
                log::warn!("kiosk: set_room_lights({area_id}, {turn_on}) failed: {e}");
                if let Some(room) = rooms.iter_mut().find(|r| r.area_id == area_id) {
                    room.is_on = !turn_on; // revert the optimistic flip - the call never took effect
                }
            }
            if let Some(rect) = buttons.as_ref().and_then(|b| b.rect_for(&area_id)) {
                let is_on = rooms.iter().find(|r| r.area_id == area_id).map(|r| r.is_on).unwrap_or(turn_on);
                crate::display::update_toggle(&mut display, rect, is_on, false);
            }
        }

        let tap = display.poll_tap();
        if tap.is_some() {
            last_activity = now;
            if dimmed {
                if let Err(e) = display.set_brightness(FULL_BRIGHTNESS_PERCENT) {
                    log::warn!("kiosk: failed to restore brightness: {e}");
                }
                dimmed = false;
            }
        } else if !dimmed && now.duration_since(last_activity) >= IDLE_DIM_TIMEOUT {
            if let Err(e) = display.set_brightness(DIM_BRIGHTNESS_PERCENT) {
                log::warn!("kiosk: failed to dim brightness: {e}");
            }
            dimmed = true;
        }

        let Some(tap) = tap else {
            continue;
        };
        let Some(area_id) = buttons.as_ref().and_then(|b| b.hit(tap)) else {
            continue;
        };
        let area_id = area_id.to_string();

        if cooldowns.get(&area_id).is_some_and(|&until| now < until) {
            continue; // mashed - this room is still cooling down from its last action
        }

        let Some(room) = rooms.iter_mut().find(|r| r.area_id == area_id) else {
            continue;
        };
        let turn_on = !room.is_on;
        room.is_on = turn_on; // optimistic - flips the toggle immediately, corrected by the next refresh if this call fails

        log::info!("kiosk: {area_id} -> {}", if turn_on { "on" } else { "off" });
        if let Some(audio) = audio.as_mut() {
            audio.click();
        }

        // Instant on-screen feedback (toggle flips + goes muted) - the actual HA call happens on
        // the worker thread (see above), so this loop keeps polling touch/dimming immediately
        // instead of freezing for the round trip.
        if let Some(rect) = buttons.as_ref().and_then(|b| b.rect_for(&area_id)) {
            crate::display::update_toggle(&mut display, rect, turn_on, true);
        }
        if action_tx.send((area_id.clone(), turn_on)).is_err() {
            log::warn!("kiosk: HA action worker thread is gone, dropping tap on {area_id}");
        }

        cooldowns.insert(area_id.clone(), Instant::now() + ACTION_COOLDOWN);
    }
}

/// Just `/api/debug` for now (see `crate::debug_log`) - a diagnostic tool, not part of
/// contracts/captive-portal-api.md, which doesn't apply to kiosk mode at all.
fn start_debug_server() -> anyhow::Result<EspHttpServer<'static>> {
    let mut server = EspHttpServer::new(&HttpServerConfig {
        stack_size: 10240,
        ..Default::default()
    })?;
    server.fn_handler("/api/debug", Method::Get, |req| -> anyhow::Result<()> {
        let (events, _) = crate::debug_log::events_since(0);
        let mut response = req.into_response(200, None, &[("Content-Type", "text/plain")])?;
        response.write_all(events.join("\n").as_bytes())?;
        Ok(())
    })?;
    Ok(server)
}

/// Same rationale as `net::ap_portal`'s `sync_time`: this board has no RTC battery, so without
/// this every HTTPS/WSS connection to Home Assistant would fail certificate validation against
/// whatever arbitrary clock it booted with.
fn sync_time() {
    match EspSntp::new_default() {
        Ok(sntp) => {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while sntp.get_sync_status() != SyncStatus::Completed
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            std::mem::forget(sntp);
        }
        Err(e) => log::warn!("kiosk: failed to start SNTP client: {e}"),
    }
}
