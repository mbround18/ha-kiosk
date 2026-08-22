use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

use embedded_svc::http::{Headers, Method};
use embedded_svc::io::{Read, Write};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::{Configuration as HttpServerConfig, EspHttpServer};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
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

struct SharedState {
    wifi: BlockingWifi<EspWifi<'static>>,
    storage: Storage,
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
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs.clone()))?,
        sys_loop,
    )?;

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
    // `is_connected()` (a real STA association) to be true before it even checks netif state.
    // In this AP+STA Mixed configuration STA is only ever *enabled* (for scanning), never
    // connected, so `is_connected()` never becomes true and `wait_netif_up()` blocks for its
    // full 15s CONNECT_TIMEOUT and returns Err(ESP_ERR_TIMEOUT) - which previously crashed the
    // whole portal ~15s after every boot. Poll the AP netif directly instead.
    wait_for_ap_netif_up(&wifi)?;
    log::info!("Captive portal AP up, DHCP serving on {AP_IP}");

    let storage = Storage::new(nvs)?;
    // FR-006 only requires tearing down the AP once a *fresh* join completes during this very
    // session - not just because WiFi already happens to be configured, which is exactly the
    // case on a button-triggered re-entry (data-model.md: re-entry must work "even when a
    // working WiFi connection exists", and the portal must stay reachable so the user can
    // change WiFi/HA/dashboard independently, per FR-012/FR-013).
    let wifi_was_configured_at_entry = storage.setup_state().wifi_configured;
    let state = Arc::new(Mutex::new(SharedState { wifi, storage }));

    start_dns_hijack();
    let _server = start_http_server(state.clone())?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let mut guard = state.lock().unwrap();
        let wifi_configured = guard.storage.setup_state().wifi_configured;
        if wifi_configured && !wifi_was_configured_at_entry {
            log::info!("WiFi step just completed - tearing down captive portal AP (FR-006)");
            // Preserve the actual STA config the successful join set (research.md /
            // wifi_sta::join_network) rather than defaulting it, which would drop the freshly
            // established station association along with the AP.
            let sta_config = match guard.wifi.get_configuration() {
                Ok(Configuration::Mixed(sta, _)) => Configuration::Client(sta),
                _ => continue,
            };
            let _ = guard.wifi.set_configuration(&sta_config);
            return Ok(());
        }
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

fn start_http_server(state: Arc<Mutex<SharedState>>) -> anyhow::Result<EspHttpServer<'static>> {
    let mut server = EspHttpServer::new(&HttpServerConfig {
        stack_size: 10240,
        ..Default::default()
    })?;

    server.fn_handler("/", Method::Get, |req| -> anyhow::Result<()> {
        req.into_ok_response()?
            .write_all(include_bytes!("../../assets/portal/index.html"))?;
        Ok(())
    })?;

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
        server.fn_handler("/api/wifi", Method::Post, move |mut req| -> anyhow::Result<()> {
            let body: WifiJoinRequest = match read_json(&mut req) {
                Ok(b) => b,
                Err(_) => return write_json(req, 400, &WifiJoinResponse::failed(WifiJoinFailureReason::Other)),
            };

            let mut guard = state.lock().unwrap();
            match wifi_sta::join_network(&mut guard.wifi, &body.ssid, &body.password) {
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

                match ha::rest::verify_connection(&body.base_url, &body.access_token) {
                    Ok(instance_name) => {
                        let mut guard = state.lock().unwrap();
                        let _ = guard.storage.set_ha_connection(
                            &body.base_url,
                            &body.access_token,
                            &instance_name,
                        );
                        write_json(req, 200, &HaConnectResponse::connected(instance_name))
                    }
                    Err(reason) => write_json(req, 400, &HaConnectResponse::failed(reason)),
                }
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
