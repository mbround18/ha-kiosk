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

use crate::ha::rooms::{self, Climate, Clock, Room, Weather};
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
/// Consecutive failed `fetch_dashboard_data` cycles (see below) before the on-screen room/
/// weather/climate data is flagged as possibly stale. >1 so a single transient blip (HA
/// restarting, a dropped packet) doesn't flash the warning; a real outage lasting multiple
/// cycles is what this is meant to catch.
const STALE_AFTER_FAILURES: u32 = 3;
/// How long the persistent `HaWsSession` (see below) is allowed to live before it's proactively
/// recycled, even if every call on it keeps succeeding. Reconnecting-per-refresh-cycle was a real
/// measured leak (~750-800 bytes/cycle) fully root-caused and fixed; after that fix, on-device
/// logs still show a much smaller (~150-160 bytes/cycle) residual decline that doesn't come from
/// anything in this codebase's own subscription/cleanup logic - consistent with something at the
/// underlying `esp_websocket_client`/TLS layer itself (periodic keepalive traffic, most likely)
/// rather than an app-level bug to keep chasing. Rather than let that residual run unbounded for
/// the device's entire uptime, this puts a ceiling on it: at the measured worst-case rate, one
/// hour of the persistent connection costs at most a few tens of KB, comfortably recovered by a
/// clean reconnect (itself now cheap and side-effect-free, since the earlier leak fixes mean nothing
/// is left dangling when a session is dropped) long before this board's ~220KB internal heap
/// budget is ever at risk.
const SESSION_MAX_AGE: Duration = Duration::from_secs(60 * 60);

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
    let mut climate: Option<Climate> = None;
    let mut clock: Option<Clock> = None;
    let mut buttons: Option<crate::display::KioskButtons> = None;
    // Whether the screensaver (see `display::show_screensaver`) reflects the current
    // `clock`/`weather`/`climate` values - `None` (unlike `buttons`, which is dropped rather than
    // just marked stale) since there's no per-tap hit-testing state to preserve here, just a
    // "does the screen need a redraw" flag.
    let mut screensaver_drawn = false;
    // Per-area "don't act on taps again until" deadlines - see `ACTION_COOLDOWN`. This is the
    // actual anti-flood guard: a child mashing one room's buttons gets every tap past the first
    // silently dropped (not queued - dropped) until the cooldown clears, so HA only ever sees
    // one call per room per cooldown window no matter how fast the taps come in.
    let mut cooldowns: HashMap<String, Instant> = HashMap::new();
    let mut last_activity = Instant::now();
    let mut dimmed = false;
    // How many `fetch_dashboard_data` cycles in a row have failed - see `STALE_AFTER_FAILURES`.
    // Reset to 0 on any success; drives the "may be outdated" header warning in
    // `display::show_rooms` once it crosses the threshold, since a failed fetch otherwise leaves
    // `rooms`/`weather`/`climate` (and therefore the screen) frozen on whatever was last fetched
    // with no visible sign that's no longer current.
    let mut consecutive_failures: u32 = 0;
    let mut stale = false;

    // Both room refreshes and light toggles used to open a fresh WebSocket connect+auth every
    // time (1-2s+, sometimes 10+), which used to run inline on this loop and freeze touch
    // polling for the whole stretch. Fixed in two parts: this all happens on a single persistent
    // worker thread (not one thread per tap - that would let a kid mashing several rooms pile up
    // unbounded concurrent HA connections) so this loop stays responsive to touch every `TICK`
    // regardless of what HA is doing; and the worker keeps ONE `HaWsSession` open for the
    // kiosk's entire runtime instead of reconnecting every `ROOM_REFRESH_INTERVAL` - reconnecting
    // that often turned out to be the actual cause of a real heap leak (confirmed via on-device
    // serial logs: internal heap dropping every single refresh cycle, exhausting this board's
    // ~220KB internal heap in roughly 2 hours - the original "state goes stale after a few
    // hours" bug). The session is only ever dropped and replaced when a call on it actually
    // fails, so a reconnect only happens as often as the connection genuinely needs one.
    enum HaRequest {
        Refresh,
        SetLights(String, bool),
    }
    enum HaResponse {
        Dashboard(anyhow::Result<(Vec<Room>, Option<Weather>, Option<Climate>, Option<Clock>)>),
        LightsResult(String, bool, anyhow::Result<()>),
    }
    let (request_tx, request_rx) = mpsc::channel::<HaRequest>();
    let (response_tx, response_rx) = mpsc::channel::<HaResponse>();
    {
        let worker_ha = ha.clone();
        std::thread::Builder::new()
            .stack_size(16 * 1024)
            .spawn(move || {
                let mut session: Option<rooms::HaWsSession> = None;
                // When the current `session` was established - `None` alongside `session` being
                // `None` too. Compared against `SESSION_MAX_AGE` below to proactively recycle a
                // long-lived connection regardless of whether it's still working.
                let mut session_established_at: Option<Instant> = None;
                // Diagnostic-only (see `debug_log::log_heap_block_counts`) - logs the allocated-
                // block count every few refresh cycles rather than every single one, so the trend
                // (does it climb by ~1/cycle in lockstep with the byte decline, or not) is visible
                // in a few log lines instead of drowning `/api/debug` in a line every 15s cycle.
                let mut refresh_count: u32 = 0;
                const HEAP_BLOCK_COUNT_LOG_EVERY_N_REFRESHES: u32 = 4;
                loop {
                    // Waiting up to `ROOM_REFRESH_INTERVAL` for an explicit request (a tap's
                    // light toggle, or a kicked refresh after one fails - see below) doubles as
                    // this thread's own periodic-refresh timer: a timeout with nothing queued
                    // just means it's time to refresh anyway.
                    let request = match request_rx.recv_timeout(ROOM_REFRESH_INTERVAL) {
                        Ok(request) => request,
                        Err(mpsc::RecvTimeoutError::Timeout) => HaRequest::Refresh,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break, // kiosk::run's loop is gone
                    };

                    if session.is_some()
                        && session_established_at.is_some_and(|at| at.elapsed() >= SESSION_MAX_AGE)
                    {
                        crate::debug_log::event(
                            "kiosk: recycling HA connection after reaching SESSION_MAX_AGE (routine maintenance, not a failure)"
                                .to_string(),
                        );
                        session = None;
                    }

                    if session.is_none() {
                        session = match rooms::HaWsSession::connect(&worker_ha.base_url, &worker_ha.access_token) {
                            Ok(s) => {
                                session_established_at = Some(Instant::now());
                                Some(s)
                            }
                            Err(e) => {
                                crate::debug_log::event(format!("kiosk: HA reconnect failed: {e:?}"));
                                session_established_at = None;
                                None
                            }
                        };
                    }

                    let sent = match request {
                        HaRequest::Refresh => {
                            // Logged on every attempt (success or failure) specifically to make
                            // a slow heap leak visible over a long run - see `ha::rooms::
                            // HaWsSession`'s doc comment for the leak this whole persistent-
                            // connection design was written to fix. Internal (non-PSRAM) heap is
                            // tracked separately from the total - this board's ~29MB of free
                            // PSRAM would hide a leak for a very long time, but internal SRAM
                            // (where driver buffers, e.g. this WS client's TLS context, actually
                            // live) is far smaller and would show it fast.
                            let free_heap = unsafe { esp_idf_svc::sys::esp_get_free_heap_size() };
                            let free_internal_heap = unsafe {
                                esp_idf_svc::sys::heap_caps_get_free_size(esp_idf_svc::sys::MALLOC_CAP_INTERNAL)
                            };
                            let result = match session.as_mut() {
                                Some(s) => rooms::fetch_dashboard_data(s),
                                None => Err(anyhow::anyhow!("not connected to Home Assistant")),
                            };
                            if result.is_err() {
                                session = None; // this connection is suspect - reconnect fresh next time
                                session_established_at = None;
                            }
                            match &result {
                                Ok((fetched_rooms, ..)) => crate::debug_log::event(format!(
                                    "kiosk: fetched {} room(s): {:?} (free heap: {free_heap} total, {free_internal_heap} internal)",
                                    fetched_rooms.len(),
                                    fetched_rooms.iter().map(|r| &r.name).collect::<Vec<_>>()
                                )),
                                Err(e) => crate::debug_log::event(format!(
                                    "kiosk: failed to fetch dashboard data: {e:?} (free heap: {free_heap} total, {free_internal_heap} internal)"
                                )),
                            }
                            refresh_count += 1;
                            if refresh_count % HEAP_BLOCK_COUNT_LOG_EVERY_N_REFRESHES == 0 {
                                crate::debug_log::log_heap_block_counts(
                                    esp_idf_svc::sys::MALLOC_CAP_INTERNAL,
                                    "internal",
                                );
                            }
                            response_tx.send(HaResponse::Dashboard(result))
                        }
                        HaRequest::SetLights(area_id, turn_on) => {
                            let result = match session.as_mut() {
                                Some(s) => rooms::set_room_lights(s, &area_id, turn_on),
                                None => Err(anyhow::anyhow!("not connected to Home Assistant")),
                            };
                            if result.is_err() {
                                session = None;
                                session_established_at = None;
                            }
                            response_tx.send(HaResponse::LightsResult(area_id, turn_on, result))
                        }
                    };
                    if sent.is_err() {
                        break; // kiosk::run's loop is gone
                    }
                }
            })?;
    }
    let _ = request_tx.send(HaRequest::Refresh); // don't wait a full `ROOM_REFRESH_INTERVAL` for the first fetch

    loop {
        std::thread::sleep(TICK);
        let now = Instant::now();

        // Non-blocking: the actual HA work (and its logging) happens on the worker thread
        // spawned above, so this never stalls touch polling waiting on the network. Both a
        // finished refresh and a finished light toggle arrive on the same channel now (one
        // persistent worker handles both), so this one drain loop replaces what used to be two
        // separate ones.
        while let Ok(response) = response_rx.try_recv() {
            match response {
                HaResponse::Dashboard(Ok((fetched_rooms, fetched_weather, fetched_climate, fetched_clock))) => {
                    rooms = fetched_rooms;
                    weather = fetched_weather;
                    climate = fetched_climate;
                    clock = fetched_clock;
                    consecutive_failures = 0;
                    stale = false;
                    buttons = None; // force a redraw with the fresh room list/weather/climate
                    screensaver_drawn = false; // and the screensaver, if that's what's showing
                }
                HaResponse::Dashboard(Err(_)) => {
                    consecutive_failures += 1;
                    if !stale && firmware_core::screen::is_stale(consecutive_failures, STALE_AFTER_FAILURES) {
                        // The room/weather/climate data on screen isn't going to change (it stays
                        // whatever was last successfully fetched), but the header needs to flip
                        // to the warning state - force a redraw just for that.
                        stale = true;
                        buttons = None;
                    }
                }
                HaResponse::LightsResult(area_id, turn_on, Ok(())) => {
                    // The call really did succeed, so the optimistic flip was correct - confirm
                    // it (un-mute) now rather than waiting out the rest of the cooldown.
                    if let Some(rect) = buttons.as_ref().and_then(|b| b.rect_for(&area_id)) {
                        let is_on = rooms.iter().find(|r| r.area_id == area_id).map(|r| r.is_on).unwrap_or(turn_on);
                        crate::display::update_toggle(&mut display, rect, is_on, false);
                    }
                }
                HaResponse::LightsResult(area_id, turn_on, Err(e)) => {
                    // Previously this reverted `is_on` to `!turn_on` - a guess. That guess can be
                    // wrong: a timed-out/failed WebSocket round trip (which is exactly the shape
                    // of thing seen under memory pressure on long runs) doesn't necessarily mean
                    // the `call_service` never reached HA - it may have gone through just fine
                    // and only our client's confirmation was lost, which would make "confidently
                    // revert to the opposite" actively misleading (this is the state-mismatch bug
                    // reported after an hour-long run: HA said on, device said off). Instead of
                    // guessing, ask HA directly: force an immediate refresh instead. The toggle
                    // is left in its current muted state (not reverted, not confirmed) until
                    // that refresh redraws everything with ground truth.
                    log::warn!("kiosk: set_room_lights({area_id}, {turn_on}) failed: {e} - forcing an immediate refresh instead of guessing the outcome");
                    let _ = request_tx.send(HaRequest::Refresh);
                }
            }
        }

        // Polled up front (rather than after drawing, as before) so a wake-up tap is known before
        // deciding what to draw this tick - the screensaver's `continue` below has to happen
        // before any room-screen drawing/hit-testing runs. The actual dim/wake decision is a pure
        // function (`firmware_core::screen::decide_tick`, unit tested there) - this just acts on
        // it (backlight, forced redraws) since those need the real `Display`.
        let tap = display.poll_tap();
        if tap.is_some() {
            last_activity = now;
        }
        let decision = firmware_core::screen::decide_tick(dimmed, tap.is_some(), now, last_activity, IDLE_DIM_TIMEOUT);
        let woke_this_tick = decision.woke_this_tick;
        if decision.dimmed != dimmed {
            dimmed = decision.dimmed;
            if dimmed {
                if let Err(e) = display.set_brightness(DIM_BRIGHTNESS_PERCENT) {
                    log::warn!("kiosk: failed to dim brightness: {e}");
                }
                screensaver_drawn = false; // force the initial screensaver draw
            } else {
                if let Err(e) = display.set_brightness(FULL_BRIGHTNESS_PERCENT) {
                    log::warn!("kiosk: failed to restore brightness: {e}");
                }
                buttons = None; // force a fresh room-grid draw now that we're back
            }
        }

        if dimmed {
            // Nothing else this tick: no room hit-testing (there's no room grid on screen to hit
            // test against), no worker-result toggle drawing (nothing queued while idle this
            // long - see `screensaver_drawn`'s doc comment). Just keep the screensaver current.
            if !screensaver_drawn {
                match crate::display::show_screensaver(&mut display, clock.as_ref(), weather.as_ref(), climate.as_ref()) {
                    Ok(()) => screensaver_drawn = true,
                    Err(e) => log::warn!("kiosk: failed to draw screensaver: {e}"),
                }
            }
            continue;
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
            match crate::display::show_rooms(&mut display, &rooms, weather.as_ref(), climate.as_ref(), stale) {
                Ok(b) => buttons = Some(b),
                Err(e) => log::warn!("kiosk: failed to draw room list: {e}"),
            }
        }

        if woke_this_tick {
            continue; // this tap just woke the screen - don't also act on it as a room toggle
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
        if request_tx.send(HaRequest::SetLights(area_id.clone(), turn_on)).is_err() {
            log::warn!("kiosk: HA worker thread is gone, dropping tap on {area_id}");
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
