use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

use embedded_svc::http::{Headers, Method};
use embedded_svc::io::{Read, Write};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::{Configuration as HttpServerConfig, EspHttpServer};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};

use crate::ha;
use crate::net::wifi_sta;
use crate::portal_api::*;
use crate::storage::Storage;

/// research.md: the device's own AP IP - ESP-IDF's default softAP subnet (observed at
/// 192.168.71.1 on this hardware/esp-idf-svc version; the captive-portal DNS hijack always
/// answers with this address regardless of the query name).
const AP_IP: &str = "192.168.71.1";
const SSID_PREFIX: &str = "HA-Kiosk-";

/// How long the on-screen tap-to-trust prompt (`display::show_trust_confirm`) waits for a human
/// before giving up - see `PendingTrust`.
const TRUST_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

struct SharedState {
    wifi: BlockingWifi<EspWifi<'static>>,
    storage: Storage,
    /// Set by the `/api/home-assistant` handler when every automatic HTTPS trust path failed
    /// (`ha::rest::VerifyOutcome::NeedsTrustConfirmation`) and a human needs to approve the
    /// captured certificate on the device's own screen before this can proceed. `run`'s main
    /// loop is what actually renders the prompt, polls touch input, and resolves this.
    pending_trust: Option<PendingTrust>,
}

struct PendingTrust {
    base_url: String,
    access_token: String,
    host: String,
    issuer: String,
    fingerprint: String,
    pem: Vec<u8>,
    deadline: std::time::Instant,
    /// `None` while still waiting on a tap or the deadline; set once the main loop has acted.
    /// Left in place (rather than cleared immediately) so a phone polling
    /// `/api/home-assistant/status` after the fact still sees the resolved outcome once.
    outcome: Option<TrustOutcome>,
}

enum TrustOutcome {
    Connected(String),
    Rejected,
    TimedOut,
    Failed(HaConnectFailureReason),
}

/// Brings up AP+STA WiFi (Mixed mode: STA is required for network scanning even though the
/// device isn't joined to anything yet), starts the DNS-hijack captive-portal responder, and
/// serves the setup HTTP API + page. Blocks forever - the caller is expected to run this only
/// when the device needs its captive portal (data-model.md's SetupState logic).
pub fn run(
    peripherals: Peripherals,
    sys_loop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> anyhow::Result<()> {
    // Kept alongside the one `EspWifi::new`/`BlockingWifi::wrap` need - `wifi_sta::join_network`
    // uses this separately to subscribe for the real WiFi disconnect reason code during a join
    // attempt (see its doc comment).
    let wifi_event_loop = sys_loop.clone();
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs.clone()))?,
        sys_loop,
    )?;

    let storage = Storage::new(nvs)?;
    let entry_state = storage.setup_state();

    // Two very different starting points: a fresh device with no saved WiFi needs the AP +
    // captive portal + WiFi-join QR; a device that already has WiFi (freshly joined moments ago,
    // or rebooted with HA/dashboard still unconfigured - e.g. after the FR-012 reprovision
    // button) can just reconnect as a plain STA on the real network and skip straight to
    // "configure Home Assistant" - no AP, no DNS hijack, no WiFi-join QR to confuse a user whose
    // phone is already on the same network. `is_captive` also gates the one-time AP-teardown
    // logic below: only a fresh join needs it.
    let is_captive = !entry_state.wifi_configured;
    let (ssid, portal_ip) = if is_captive {
        let mac = wifi.wifi().get_mac(esp_idf_svc::wifi::WifiDeviceId::Ap)?;
        let ssid = format!("{SSID_PREFIX}{:02X}{:02X}", mac[4], mac[5]);
        log::info!("Starting captive portal AP '{ssid}'");

        let ap_config = Configuration::Mixed(
            ClientConfiguration::default(),
            AccessPointConfiguration {
                ssid: ssid.as_str().try_into().unwrap(),
                ssid_hidden: false,
                auth_method: AuthMethod::None,
                channel: 6,
                ..Default::default()
            },
        );
        wifi.set_configuration(&ap_config)?;
        wifi.start()?;
        // NOT `wifi.wait_netif_up()`: that waits on `EspWifi::is_up()`, which requires
        // `is_connected()` (a real STA association) to be true before it even checks netif
        // state. In this AP+STA Mixed configuration STA is only ever *enabled* (for scanning),
        // never connected, so `is_connected()` never becomes true and `wait_netif_up()` blocks
        // for its full 15s CONNECT_TIMEOUT and returns Err(ESP_ERR_TIMEOUT) - which previously
        // crashed the whole portal ~15s after every boot. Poll the AP netif directly instead.
        wait_for_ap_netif_up(&wifi)?;
        log::info!("Captive portal AP up, DHCP serving on {AP_IP}");
        (ssid, AP_IP.to_string())
    } else {
        let creds = storage
            .get_wifi_credentials()
            .ok_or_else(|| anyhow::anyhow!("wifi_configured but no saved credentials in NVS"))?;
        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            ssid: creds.ssid.as_str().try_into().unwrap(),
            password: creds.password.as_str().try_into().unwrap(),
            auth_method: if creds.password.is_empty() {
                AuthMethod::None
            } else {
                AuthMethod::WPA2WPA3Personal
            },
            ..Default::default()
        }))?;
        wifi.start()?;
        wifi.connect()?;
        wifi.wait_netif_up()?;
        let ip = wifi.wifi().sta_netif().get_ip_info()?.ip;
        log::info!("Reconnected to saved WiFi '{}', IP {ip}", creds.ssid);
        sync_time();
        (creds.ssid, ip.to_string())
    };

    let ssid_for_server = ssid.clone();
    // FR-006 only requires tearing down the AP once a *fresh* join completes during this very
    // session - not just because WiFi already happens to be configured, which is exactly the
    // case on a button-triggered re-entry (data-model.md: re-entry must work "even when a
    // working WiFi connection exists", and the portal must stay reachable so the user can
    // change WiFi/HA/dashboard independently, per FR-012/FR-013).
    let wifi_was_configured_at_entry = entry_state.wifi_configured;
    let mut ap_torn_down = false;
    let state = Arc::new(Mutex::new(SharedState {
        wifi,
        storage,
        pending_trust: None,
    }));

    if is_captive {
        start_dns_hijack();
    }
    let _server = start_http_server(state.clone(), ssid_for_server, wifi_event_loop)?;

    // Physical-screen splash (user request: show setup QR codes on the device itself, since a
    // phone needs to already be connected before it can load the portal's web page - the QR
    // codes can't usefully live there). Not fatal if it fails: the network/portal side must
    // keep working even if the display has a problem. Kept alive (not `mem::forget`'d) for the
    // rest of this function so the AP-teardown transition below can redraw it.
    let mut display = match crate::display::Display::init() {
        Ok(mut display) => {
            let portal_url = format!("http://{portal_ip}/");
            let result = if is_captive {
                crate::display::show_setup_splash(&mut display, &ssid, &portal_url)
            } else {
                crate::display::show_finish_setup_splash(&mut display, &portal_url)
            };
            if let Err(e) = result {
                log::warn!("Failed to draw setup splash: {e}");
            }
            Some(display)
        }
        Err(e) => {
            log::warn!("Display init failed, continuing without on-screen splash: {e}");
            None
        }
    };

    // Bounds of the currently-drawn Trust/Cancel buttons, for hit-testing `Display::poll_tap()`
    // against - `None` whenever no trust-confirm screen is on screen right now.
    let mut trust_buttons: Option<crate::display::TrustButtons> = None;
    // Debounce state for the trust-confirm screen's touch reads - the touch controller's I2C
    // bus logs a marginal-pull-ups warning at boot ("unexpected behavior would happen"), so a
    // single noisy read shouldn't be enough to silently resolve this by itself. `Instant` is
    // when the screen was (re)drawn - a tap read within the first 500ms of that is ignored
    // outright, since GT911 has been observed reporting a stale/spurious touch right at
    // redraw. Beyond that, the same button must register on two consecutive ~150ms ticks
    // before it counts.
    let mut trust_screen_shown_at: Option<std::time::Instant> = None;
    let mut trust_tap_streak: Option<(bool, u8)> = None;
    // Temporary crash-diagnosis counter - see `Storage::set_debug_stage`. Breadcrumbs are only
    // written for the first few ticks of a trust-confirm session to avoid excess NVS writes.
    let mut trust_tick_n: u32 = 0;
    let mut last_setup_state_check = std::time::Instant::now();

    loop {
        // Faster than the setup-state poll below needs, so a physical tap on the trust-confirm
        // screen feels responsive rather than needing to be held for up to a second.
        std::thread::sleep(std::time::Duration::from_millis(150));

        // Home Assistant trust-confirmation: render the prompt (once) and poll for a tap,
        // independent of the once-a-second setup-state polling below. The lock is only ever
        // held briefly for cheap field access - the heavier fields (the PEM can be a few KB)
        // are only cloned out when actually needed (the initial draw, or finalizing an
        // approval), not on every ~150ms tick, and `finalize_trusted`'s blocking TLS handshake
        // runs with no lock held at all so a phone polling `/api/home-assistant/status` isn't
        // stalled for its duration.
        {
            let deadline = {
                let guard = state.lock().unwrap();
                guard
                    .pending_trust
                    .as_ref()
                    .and_then(|p| p.outcome.is_none().then_some(p.deadline))
            };

            if let Some(deadline) = deadline {
                trust_tick_n += 1;
                let n = trust_tick_n;
                macro_rules! stage {
                    ($s:expr) => {
                        crate::debug_log::event(format!("trust t{n}: {}", $s));
                    };
                }

                stage!("tick_start");

                if trust_buttons.is_none() {
                    stage!("before_draw");
                    let drawn = {
                        let guard = state.lock().unwrap();
                        guard
                            .pending_trust
                            .as_ref()
                            .map(|p| (p.host.clone(), p.issuer.clone(), p.fingerprint.clone()))
                    };
                    if let (Some((host, issuer, fingerprint)), Some(d)) = (drawn, display.as_mut()) {
                        match crate::display::show_trust_confirm(d, &host, &issuer, &fingerprint) {
                            Ok(buttons) => {
                                stage!("after_draw");
                                trust_buttons = Some(buttons);
                                trust_screen_shown_at = Some(std::time::Instant::now());
                                trust_tap_streak = None;
                            }
                            Err(e) => log::warn!("Failed to draw trust-confirm screen: {e}"),
                        }
                    }
                }

                stage!("before_poll_tap");
                let raw_tap = display.as_mut().and_then(|d| d.poll_tap());
                stage!("after_poll_tap");
                if let Some(p) = raw_tap {
                    log::info!("Trust-confirm: raw tap at ({}, {})", p.x, p.y);
                }
                let past_settle_window = trust_screen_shown_at
                    .is_some_and(|shown| shown.elapsed() > std::time::Duration::from_millis(500));

                stage!("before_hit_test");
                let this_tick_hit = raw_tap
                    .filter(|_| past_settle_window)
                    .and_then(|p| trust_buttons.as_ref().and_then(|b| b.hit(p)));
                stage!("after_hit_test");

                let tapped_decision = match (this_tick_hit, trust_tap_streak) {
                    (Some(hit), Some((prev_hit, streak))) if hit == prev_hit => {
                        if streak + 1 >= 2 {
                            log::info!(
                                "Trust-confirm: {} confirmed after {} consecutive reads",
                                if hit { "Trust" } else { "Cancel" },
                                streak + 1
                            );
                            Some(hit)
                        } else {
                            trust_tap_streak = Some((hit, streak + 1));
                            None
                        }
                    }
                    (Some(hit), _) => {
                        trust_tap_streak = Some((hit, 1));
                        None
                    }
                    (None, _) => {
                        trust_tap_streak = None;
                        None
                    }
                };
                let timed_out = std::time::Instant::now() > deadline;

                let outcome = if let Some(approve) = tapped_decision {
                    if approve {
                        let approved = {
                            let guard = state.lock().unwrap();
                            guard
                                .pending_trust
                                .as_ref()
                                .map(|p| (p.base_url.clone(), p.access_token.clone(), p.pem.clone()))
                        };
                        approved.map(|(base_url, access_token, pem)| {
                            crate::debug_log::event("trust: before finalize_trusted");
                            let result = ha::rest::finalize_trusted(&base_url, &access_token, &pem);
                            crate::debug_log::event(format!(
                                "trust: after finalize_trusted, result={}",
                                match &result {
                                    Ok(name) => format!("Ok({name})"),
                                    Err(reason) => format!("Err({})", reason.as_str()),
                                }
                            ));
                            match result {
                                Ok(instance_name) => {
                                    let mut guard = state.lock().unwrap();
                                    let _ = guard.storage.set_ha_connection(
                                        &base_url,
                                        &access_token,
                                        &instance_name,
                                    );
                                    let _ = guard.storage.set_pinned_ca(&pem);
                                    TrustOutcome::Connected(instance_name)
                                }
                                Err(reason) => TrustOutcome::Failed(reason),
                            }
                        })
                    } else {
                        Some(TrustOutcome::Rejected)
                    }
                } else if timed_out {
                    Some(TrustOutcome::TimedOut)
                } else {
                    None
                };
                stage!("after_outcome_computed");

                if let Some(outcome) = outcome {
                    let mut guard = state.lock().unwrap();
                    if let Some(p) = guard.pending_trust.as_mut() {
                        p.outcome = Some(outcome);
                    }
                    trust_buttons = None;
                    trust_screen_shown_at = None;
                    trust_tap_streak = None;
                    if let Some(d) = display.as_mut() {
                        if let Ok(info) = guard.wifi.wifi().sta_netif().get_ip_info() {
                            let portal_url = format!("http://{}/", info.ip);
                            let _ = crate::display::show_finish_setup_splash(d, &portal_url);
                        }
                    }
                }
                stage!("tick_end");
            }
        }

        if last_setup_state_check.elapsed() < std::time::Duration::from_secs(1) {
            continue;
        }
        last_setup_state_check = std::time::Instant::now();

        let mut guard = state.lock().unwrap();
        let setup_state = guard.storage.setup_state();

        if is_captive && !ap_torn_down && setup_state.wifi_configured && !wifi_was_configured_at_entry {
            log::info!("WiFi step just completed - tearing down captive portal AP (FR-006)");
            // Preserve the actual STA config the successful join set (research.md /
            // wifi_sta::join_network) rather than defaulting it, which would drop the freshly
            // established station association along with the AP.
            match guard.wifi.get_configuration() {
                Ok(Configuration::Mixed(sta, _)) => {
                    let _ = guard.wifi.set_configuration(&Configuration::Client(sta));
                    ap_torn_down = true;
                    sync_time();
                    if let Some(display) = display.as_mut() {
                        match guard.wifi.wifi().sta_netif().get_ip_info() {
                            Ok(info) => {
                                let portal_url = format!("http://{}/", info.ip);
                                if let Err(e) =
                                    crate::display::show_finish_setup_splash(display, &portal_url)
                                {
                                    log::warn!("Failed to draw finish-setup splash: {e}");
                                }
                            }
                            Err(e) => log::warn!("Failed to read STA IP for splash redraw: {e}"),
                        }
                    }
                }
                _ => {}
            }
        }

        if setup_state.ha_configured && setup_state.dashboard_selected {
            if let Some(display) = display {
                std::mem::forget(display); // keep the framebuffer alive for the life of the device
            }
            return Ok(());
        }
    }
}

/// The ESP32-P4 has no RTC battery, so it boots with an arbitrary/wrong system clock. mbedTLS's
/// X.509 chain verification checks each certificate's validity period against that clock, so
/// without this, essentially every real HTTPS server's certificate fails verification with a
/// generic "Failed to verify certificate" / `mbedtls_ssl_handshake returned -0x3000` error -
/// this is what was actually behind the HA connection step's "Unreachable" failures, not a DNS
/// or connectivity problem. Blocks up to 10s; not fatal if it times out (logs a warning and lets
/// setup continue) since WiFi/HA are otherwise independent of clock correctness. The `EspSntp`
/// handle is intentionally leaked (`mem::forget`) so it keeps periodically resyncing for the
/// life of the device rather than stopping the moment this function returns.
fn sync_time() {
    match EspSntp::new_default() {
        Ok(sntp) => {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut completed = false;
            while std::time::Instant::now() < deadline {
                if sntp.get_sync_status() == SyncStatus::Completed {
                    completed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if completed {
                log::info!("SNTP time sync completed, unix time now {now}");
            } else {
                log::warn!(
                    "SNTP time sync did not complete within 10s (unix time now {now}) - HTTPS \
                     requests (e.g. Home Assistant) may fail certificate validation until it does"
                );
            }
            std::mem::forget(sntp);
        }
        Err(e) => log::warn!("Failed to start SNTP client: {e}"),
    }
}

fn wait_for_ap_netif_up(wifi: &BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if wifi.wifi().ap_netif().is_up()? {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            anyhow::bail!("AP netif did not come up in time");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn start_dns_hijack() {
    std::thread::spawn(|| {
        let socket = match UdpSocket::bind("0.0.0.0:53") {
            Ok(s) => s,
            Err(e) => {
                log::error!("DNS hijack: failed to bind :53: {e}");
                return;
            }
        };
        let ap_ip: [u8; 4] = AP_IP
            .split('.')
            .map(|s| s.parse().unwrap())
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();

        let mut buf = [0u8; 512];
        loop {
            let (len, src) = match socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("DNS hijack: recv error: {e}");
                    continue;
                }
            };
            if len < 12 {
                continue;
            }
            if let Some(response) = build_dns_response(&buf[..len], ap_ip) {
                let _ = socket.send_to(&response, src);
            }
        }
    });
}

/// Minimal captive-portal DNS responder: answers every query with a single A record pointing
/// at the device's own AP IP, regardless of the name being asked about. research.md.
fn build_dns_response(query: &[u8], ap_ip: [u8; 4]) -> Option<Vec<u8>> {
    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if qdcount == 0 {
        return None;
    }

    // Question section starts right after the 12-byte header and runs to the end of the
    // (single) question we expect from a typical stub resolver.
    let question = &query[12..];

    let mut response = Vec::with_capacity(query.len() + 16);
    response.extend_from_slice(&query[0..2]); // transaction ID
    response.extend_from_slice(&[0x81, 0x80]); // flags: standard response, no error
    response.extend_from_slice(&query[4..6]); // QDCOUNT (echoed)
    response.extend_from_slice(&[0x00, 0x01]); // ANCOUNT = 1
    response.extend_from_slice(&[0x00, 0x00]); // NSCOUNT
    response.extend_from_slice(&[0x00, 0x00]); // ARCOUNT
    response.extend_from_slice(question); // echo the question section back

    // Answer: name = pointer to question at offset 12, type A, class IN, TTL 60, RDLENGTH 4
    response.extend_from_slice(&[0xC0, 0x0C]);
    response.extend_from_slice(&[0x00, 0x01]);
    response.extend_from_slice(&[0x00, 0x01]);
    response.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]);
    response.extend_from_slice(&[0x00, 0x04]);
    response.extend_from_slice(&ap_ip);

    Some(response)
}

/// The captive portal's frontend (firmware/portal-ui: Vite + React + TypeScript + shadcn/ui) is
/// built ahead of time (`pnpm build` in that directory) and its output embedded directly into
/// the firmware binary - there's no filesystem/SPIFFS partition serving this, just static byte
/// arrays. `portal-ui/vite.config.ts` configures fixed, unhashed output filenames specifically
/// so this table doesn't need regenerating on every frontend change.
struct EmbeddedAsset {
    path: &'static str,
    content_type: &'static str,
    bytes: &'static [u8],
}

const EMBEDDED_ASSETS: &[EmbeddedAsset] = &[
    EmbeddedAsset {
        path: "/",
        content_type: "text/html",
        bytes: include_bytes!("../../portal-ui/dist/index.html"),
    },
    EmbeddedAsset {
        path: "/assets/app.js",
        content_type: "text/javascript",
        bytes: include_bytes!("../../portal-ui/dist/assets/app.js"),
    },
    EmbeddedAsset {
        path: "/assets/app.css",
        content_type: "text/css",
        bytes: include_bytes!("../../portal-ui/dist/assets/app.css"),
    },
    EmbeddedAsset {
        path: "/assets/app.woff2",
        content_type: "font/woff2",
        bytes: include_bytes!("../../portal-ui/dist/assets/app.woff2"),
    },
    EmbeddedAsset {
        path: "/assets/app2.woff2",
        content_type: "font/woff2",
        bytes: include_bytes!("../../portal-ui/dist/assets/app2.woff2"),
    },
    EmbeddedAsset {
        path: "/assets/app3.woff2",
        content_type: "font/woff2",
        bytes: include_bytes!("../../portal-ui/dist/assets/app3.woff2"),
    },
    EmbeddedAsset {
        path: "/assets/app4.woff2",
        content_type: "font/woff2",
        bytes: include_bytes!("../../portal-ui/dist/assets/app4.woff2"),
    },
    EmbeddedAsset {
        path: "/assets/app5.woff2",
        content_type: "font/woff2",
        bytes: include_bytes!("../../portal-ui/dist/assets/app5.woff2"),
    },
    EmbeddedAsset {
        path: "/favicon.svg",
        content_type: "image/svg+xml",
        bytes: include_bytes!("../../portal-ui/dist/favicon.svg"),
    },
    EmbeddedAsset {
        path: "/icons.svg",
        content_type: "image/svg+xml",
        bytes: include_bytes!("../../portal-ui/dist/icons.svg"),
    },
];

fn start_http_server(
    state: Arc<Mutex<SharedState>>,
    ssid: String,
    wifi_event_loop: EspSystemEventLoop,
) -> anyhow::Result<EspHttpServer<'static>> {
    let mut server = EspHttpServer::new(&HttpServerConfig {
        stack_size: 10240,
        ..Default::default()
    })?;

    for asset in EMBEDDED_ASSETS {
        server.fn_handler(asset.path, Method::Get, move |req| -> anyhow::Result<()> {
            // Every embedded asset lives at a fixed, unhashed filename (see EMBEDDED_ASSETS'
            // doc comment) - without this, a browser that already cached an older firmware's
            // portal page can silently keep serving stale JS/CSS after a reflash (a real
            // suspect behind one confusing WiFi-join test during development, where a hard
            // refresh was needed to get consistent behavior). "no-store" means every load
            // re-fetches from the device instead of ever needing a manual hard-refresh.
            req.into_response(
                200,
                None,
                &[
                    ("Content-Type", asset.content_type),
                    ("Cache-Control", "no-store"),
                ],
            )?
            .write_all(asset.bytes)?;
            Ok(())
        })?;
    }

    // Captive-portal OS-detection endpoints (contracts/captive-portal-api.md): respond with
    // something other than the OS's expected "this network is fine" answer, so the OS pops its
    // captive-portal UI pointed at "/".
    for path in ["/generate_204", "/hotspot-detect.html", "/connecttest.txt", "/ncsi.txt"] {
        server.fn_handler(path, Method::Get, |req| -> anyhow::Result<()> {
            req.into_response(302, Some("Found"), &[("Location", "/")])?;
            Ok(())
        })?;
    }

    {
        // Live-ish debug dump - see `crate::debug_log`. Returns immediately with everything
        // buffered so far; poll it repeatedly for a live view (e.g. `watch -n1 curl -s
        // http://<device-ip>/api/debug`). This does NOT hold the connection open: ESP-IDF's
        // httpd here processes one request at a time on a single task, so a handler that never
        // returns (the original Server-Sent-Events design) blocks every other request - phone
        // and portal traffic included - for as long as it's connected. Diagnostic tool, not part
        // of contracts/captive-portal-api.md.
        server.fn_handler("/api/debug", Method::Get, |req| -> anyhow::Result<()> {
            let (events, _) = crate::debug_log::events_since(0);
            let mut response = req.into_response(200, None, &[("Content-Type", "text/plain")])?;
            response.write_all(events.join("\n").as_bytes())?;
            Ok(())
        })?;
    }

    {
        let ssid = ssid.clone();
        server.fn_handler("/api/portal-info", Method::Get, move |req| -> anyhow::Result<()> {
            write_json(
                req,
                200,
                &PortalInfoResponse {
                    ssid: ssid.clone(),
                    ip: AP_IP.to_string(),
                },
            )
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/status", Method::Get, move |req| -> anyhow::Result<()> {
            let guard = state.lock().unwrap();
            let setup_state = guard.storage.setup_state();
            let instance_name = guard
                .storage
                .get_ha_connection()
                .map(|c| c.instance_name)
                .filter(|n| !n.is_empty());
            let body = StatusResponse::from_state(setup_state, instance_name);
            write_json(req, 200, &body)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/networks", Method::Get, move |req| -> anyhow::Result<()> {
            let mut guard = state.lock().unwrap();
            let networks = wifi_sta::scan_networks(&mut guard.wifi).unwrap_or_default();
            write_json(req, 200, &NetworksResponse { networks })
        })?;
    }

    {
        let state = state.clone();
        let wifi_event_loop = wifi_event_loop.clone();
        server.fn_handler("/api/wifi", Method::Post, move |mut req| -> anyhow::Result<()> {
            let body: WifiJoinRequest = match read_json(&mut req) {
                Ok(b) => b,
                Err(_) => return write_json(req, 400, &WifiJoinResponse::failed(WifiJoinFailureReason::Other)),
            };

            let mut guard = state.lock().unwrap();
            match wifi_sta::join_network(&mut guard.wifi, &body.ssid, &body.password, &wifi_event_loop) {
                Ok(()) => {
                    let _ = guard.storage.set_wifi_credentials(&body.ssid, &body.password);
                    write_json(req, 200, &WifiJoinResponse::connected())
                }
                Err(reason) => write_json(req, 400, &WifiJoinResponse::failed(reason)),
            }
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler(
            "/api/home-assistant",
            Method::Post,
            move |mut req| -> anyhow::Result<()> {
                let body: HaConnectRequest = match read_json(&mut req) {
                    Ok(b) => b,
                    Err(_) => {
                        return write_json(
                            req,
                            400,
                            &HaConnectResponse::failed(HaConnectFailureReason::Unreachable),
                        )
                    }
                };

                crate::debug_log::event(format!(
                    "POST /api/home-assistant: base_url={}",
                    body.base_url
                ));
                let stored_pinned_ca = state.lock().unwrap().storage.get_pinned_ca();

                let outcome = ha::rest::verify_connection(
                    &body.base_url,
                    &body.access_token,
                    stored_pinned_ca.as_deref(),
                );

                match outcome {
                    ha::rest::VerifyOutcome::Connected(instance_name) => {
                        crate::debug_log::event(format!(
                            "verify_connection: Connected({instance_name})"
                        ));
                        let mut guard = state.lock().unwrap();
                        let _ = guard.storage.set_ha_connection(
                            &body.base_url,
                            &body.access_token,
                            &instance_name,
                        );
                        write_json(req, 200, &HaConnectResponse::connected(instance_name))
                    }
                    ha::rest::VerifyOutcome::Failed(reason) => {
                        crate::debug_log::event(format!(
                            "verify_connection: Failed({})",
                            reason.as_str()
                        ));
                        write_json(req, 400, &HaConnectResponse::failed(reason))
                    }
                    ha::rest::VerifyOutcome::NeedsTrustConfirmation(captured, host) => {
                        crate::debug_log::event(format!(
                            "verify_connection: NeedsTrustConfirmation(host={host}, issuer={}, pem_len={})",
                            captured.issuer,
                            captured.pem.len()
                        ));
                        let mut guard = state.lock().unwrap();
                        let response = HaConnectResponse::awaiting_confirmation(
                            captured.issuer.clone(),
                            captured.fingerprint_sha256.clone(),
                        );
                        guard.pending_trust = Some(PendingTrust {
                            base_url: body.base_url,
                            access_token: body.access_token,
                            host,
                            issuer: captured.issuer,
                            fingerprint: captured.fingerprint_sha256,
                            pem: captured.pem,
                            deadline: std::time::Instant::now() + TRUST_CONFIRM_TIMEOUT,
                            outcome: None,
                        });
                        crate::debug_log::event("pending_trust set, responded awaiting_confirmation");
                        write_json(req, 200, &response)
                    }
                }
            },
        )?;
    }

    {
        let state = state.clone();
        server.fn_handler(
            "/api/home-assistant/status",
            Method::Get,
            move |req| -> anyhow::Result<()> {
                let guard = state.lock().unwrap();
                let response = match &guard.pending_trust {
                    Some(pending) => match &pending.outcome {
                        None => HaConnectResponse::awaiting_confirmation(
                            pending.issuer.clone(),
                            pending.fingerprint.clone(),
                        ),
                        Some(TrustOutcome::Connected(instance_name)) => {
                            HaConnectResponse::connected(instance_name.clone())
                        }
                        Some(TrustOutcome::Rejected) => HaConnectResponse::rejected(),
                        Some(TrustOutcome::TimedOut) => HaConnectResponse::timed_out(),
                        Some(TrustOutcome::Failed(reason)) => HaConnectResponse::failed(*reason),
                    },
                    None => match guard.storage.get_ha_connection() {
                        Some(conn) => HaConnectResponse::connected(conn.instance_name),
                        None => HaConnectResponse::failed(HaConnectFailureReason::Unreachable),
                    },
                };
                write_json(req, 200, &response)
            },
        )?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/dashboards", Method::Get, move |req| -> anyhow::Result<()> {
            let ha_conn = { state.lock().unwrap().storage.get_ha_connection() };
            let Some(ha_conn) = ha_conn else {
                return write_json(req, 400, &DashboardsResponse { dashboards: vec![] });
            };
            let dashboards =
                ha::dashboards::list_dashboards(&ha_conn.base_url, &ha_conn.access_token)
                    .unwrap_or_default();
            write_json(req, 200, &DashboardsResponse { dashboards })
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler(
            "/api/dashboard",
            Method::Post,
            move |mut req| -> anyhow::Result<()> {
                let body: DashboardSelectRequest = match read_json(&mut req) {
                    Ok(b) => b,
                    Err(_) => return write_json(req, 400, &DashboardSelectResponse { status: "failed" }),
                };
                let mut guard = state.lock().unwrap();
                let _ = guard.storage.set_dashboard(&body.url_path, &body.title);
                write_json(req, 200, &DashboardSelectResponse { status: "saved" })
            },
        )?;
    }

    Ok(server)
}

fn read_json<T: serde::de::DeserializeOwned>(
    req: &mut esp_idf_svc::http::server::Request<&mut esp_idf_svc::http::server::EspHttpConnection<'_>>,
) -> anyhow::Result<T> {
    let len = req.content_len().unwrap_or(0) as usize;
    let mut buf = vec![0u8; len];
    req.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}

fn write_json<T: serde::Serialize>(
    req: esp_idf_svc::http::server::Request<&mut esp_idf_svc::http::server::EspHttpConnection<'_>>,
    status: u16,
    body: &T,
) -> anyhow::Result<()> {
    let json = serde_json::to_vec(body)?;
    let mut response = req.into_response(
        status,
        None,
        &[("Content-Type", "application/json")],
    )?;
    response.write_all(&json)?;
    Ok(())
}
