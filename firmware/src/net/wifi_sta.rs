use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::wifi::{
    AccessPointInfo, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
    WifiEvent,
};

use crate::portal_api::{NetworkInfo, WifiJoinFailureReason};

/// contracts/captive-portal-api.md: GET /api/networks
///
/// A multi-AP/mesh household (many physical radios sharing one SSID) would otherwise list the
/// same network once per radio. Collapse to one entry per SSID, keeping the strongest signal
/// seen (the AP the user is actually closest to / most likely to join through).
pub fn scan_networks(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<Vec<NetworkInfo>> {
    let results = wifi.scan()?;

    let mut by_ssid: HashMap<String, NetworkInfo> = HashMap::new();
    for ap in results.into_iter().map(|ap: AccessPointInfo| NetworkInfo {
        ssid: ap.ssid.to_string(),
        rssi: ap.signal_strength,
        secure: ap.auth_method.map(|m| m != AuthMethod::None).unwrap_or(true),
    }) {
        by_ssid
            .entry(ap.ssid.clone())
            .and_modify(|existing| {
                if ap.rssi > existing.rssi {
                    *existing = ap.clone();
                }
            })
            .or_insert(ap);
    }

    let mut networks: Vec<NetworkInfo> = by_ssid.into_values().collect();
    networks.sort_by(|a, b| b.rssi.cmp(&a.rssi));
    Ok(networks)
}

/// contracts/captive-portal-api.md: POST /api/wifi
///
/// Classifies the failure reason on a best-effort basis (FR-005: "distinguishing ... wherever
/// the underlying failure reason allows it") by checking whether the requested SSID was even
/// visible in a fresh scan before attempting to join. `BlockingWifi::connect` itself only
/// returns a generic timeout/error - to see the *actual* WiFi disconnect reason code (auth
/// failure vs. 4-way handshake timeout vs. something else entirely), this subscribes to
/// `WifiEvent::StaDisconnected` for the duration of the attempt and logs whatever reason code
/// comes back via `debug_log`, rather than continuing to guess from the generic error alone.
pub fn join_network(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ssid: &str,
    password: &str,
    sys_loop: &EspSystemEventLoop,
) -> Result<(), WifiJoinFailureReason> {
    // Scanned directly (not via `scan_networks`, which collapses to `NetworkInfo` and drops
    // `auth_method`) so the actual advertised auth mode can be matched exactly below, rather
    // than guessed. Keeps the strongest-signal duplicate, same as `scan_networks`.
    let scan_results = wifi.scan();
    let scan_succeeded = scan_results.is_ok();
    let scanned_ap = scan_results.ok().and_then(|results| {
        results
            .into_iter()
            .filter(|ap: &AccessPointInfo| ap.ssid.as_str() == ssid)
            .max_by_key(|ap| ap.signal_strength)
    });

    // If the scan itself failed, don't jump to "not found" - fall through and let the actual
    // connect attempt be the source of truth instead.
    if scan_succeeded && scanned_ap.is_none() {
        return Err(WifiJoinFailureReason::NetworkNotFound);
    }

    // Match the AP's own advertised auth mode exactly rather than guessing a fixed combined
    // mode - `AuthMethod::WPA2WPA3Personal` turned out to set a *minimum* auth-mode threshold
    // on this ESP-IDF version (`wifi_sta_config_t.threshold.authmode`), not an "accept either"
    // setting as first assumed: it correctly handles APs in WPA2/WPA3 "transition mode" (which
    // WPA2Personal alone fails to associate with, misclassified below as WrongPassword even
    // with a correct password), but it also rejects a plain WPA2-only AP outright at the scan
    // stage (`WIFI_REASON_NO_AP_FOUND_IN_AUTHMODE_THRESHOLD`, reason code 211) since that AP
    // doesn't meet the WPA2/WPA3-combined floor. Falls back to the old WPA2WPA3Personal guess
    // only if the scan didn't report an auth mode for some reason.
    let auth_method = if password.is_empty() {
        AuthMethod::None
    } else {
        scanned_ap
            .as_ref()
            .and_then(|ap| ap.auth_method)
            .filter(|m| *m != AuthMethod::None)
            .unwrap_or(AuthMethod::WPA2WPA3Personal)
    };

    let client_config = ClientConfiguration {
        ssid: ssid.try_into().map_err(|_| WifiJoinFailureReason::Other)?,
        password: password
            .try_into()
            .map_err(|_| WifiJoinFailureReason::Other)?,
        auth_method,
        ..Default::default()
    };

    // Preserve the existing AccessPoint side of the config (Configuration::Client would tear
    // down the captive portal AP immediately, before we even know whether the join succeeded -
    // which was previously killing every in-flight request, success or failure, mid-response).
    let ap_config = match wifi.get_configuration().map_err(|_| WifiJoinFailureReason::Other)? {
        Configuration::Mixed(_, ap) | Configuration::AccessPoint(ap) => ap,
        _ => return Err(WifiJoinFailureReason::Other),
    };

    wifi.set_configuration(&Configuration::Mixed(client_config, ap_config))
        .map_err(|_| WifiJoinFailureReason::Other)?;

    if !wifi.is_started().unwrap_or(false) {
        wifi.start().map_err(|_| WifiJoinFailureReason::Other)?;
    }

    let last_disconnect_reason: Arc<Mutex<Option<u16>>> = Arc::new(Mutex::new(None));
    let reason_slot = last_disconnect_reason.clone();
    let _subscription = sys_loop
        .subscribe::<WifiEvent, _>(move |event: WifiEvent| {
            if let WifiEvent::StaDisconnected(disconnected) = event {
                *reason_slot.lock().unwrap() = Some(disconnected.reason());
            }
        })
        .ok();

    let connect_result = wifi.connect();

    if let Err(e) = &connect_result {
        let reason = *last_disconnect_reason.lock().unwrap();
        crate::debug_log::event(format!(
            "join_network: connect() failed: {e:?}, last disconnect reason={reason:?}"
        ));
    }

    connect_result.map_err(|_| WifiJoinFailureReason::WrongPassword)?;

    Ok(())
}
