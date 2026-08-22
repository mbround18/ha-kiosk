use esp_idf_svc::wifi::{
    AccessPointInfo, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};

use crate::portal_api::{NetworkInfo, WifiJoinFailureReason};

/// contracts/captive-portal-api.md: GET /api/networks
pub fn scan_networks(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<Vec<NetworkInfo>> {
    let results = wifi.scan()?;
    Ok(results
        .into_iter()
        .map(|ap: AccessPointInfo| NetworkInfo {
            ssid: ap.ssid.to_string(),
            rssi: ap.signal_strength,
            secure: ap.auth_method.map(|m| m != AuthMethod::None).unwrap_or(true),
        })
        .collect())
}

/// contracts/captive-portal-api.md: POST /api/wifi
///
/// Classifies the failure reason on a best-effort basis (FR-005: "distinguishing ... wherever
/// the underlying failure reason allows it") by checking whether the requested SSID was even
/// visible in a fresh scan before attempting to join - `BlockingWifi::connect` itself only
/// exposes a generic timeout/error, not the underlying WiFi disconnect reason code.
pub fn join_network(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ssid: &str,
    password: &str,
) -> Result<(), WifiJoinFailureReason> {
    let found = scan_networks(wifi)
        .map(|nets| nets.iter().any(|n| n.ssid == ssid))
        .unwrap_or(true); // if the scan itself fails, don't jump to "not found" - fall through

    if !found {
        return Err(WifiJoinFailureReason::NetworkNotFound);
    }

    let auth_method = if password.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
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

    wifi.connect().map_err(|_| WifiJoinFailureReason::WrongPassword)?;

    Ok(())
}
