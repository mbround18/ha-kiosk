use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use embedded_svc::io::Read;
use esp_idf_svc::http::client::{Configuration as HttpClientConfig, EspHttpConnection};

use crate::portal_api::HaConnectFailureReason;

#[derive(serde::Deserialize)]
struct HaConfigResponse {
    location_name: String,
}

/// contracts/captive-portal-api.md: POST /api/home-assistant verification step.
///
/// Calls `GET {base_url}/api/config` with the given bearer token. research.md: this single
/// REST call both confirms the token is valid and returns the instance's display name.
pub fn verify_connection(
    base_url: &str,
    access_token: &str,
) -> Result<String, HaConnectFailureReason> {
    let is_https = base_url.starts_with("https://");
    let config = HttpClientConfig {
        crt_bundle_attach: is_https.then_some(esp_idf_svc::sys::esp_crt_bundle_attach),
        ..Default::default()
    };
    let connection =
        EspHttpConnection::new(&config).map_err(|_| HaConnectFailureReason::Unreachable)?;
    let mut client = HttpClient::wrap(connection);

    let url = format!("{}/api/config", base_url.trim_end_matches('/'));
    let auth_header = format!("Bearer {access_token}");
    let headers = [("authorization", auth_header.as_str())];

    let request = client
        .request(Method::Get, &url, &headers)
        .map_err(|_| HaConnectFailureReason::Unreachable)?;
    let mut response = request
        .submit()
        .map_err(|_| HaConnectFailureReason::Unreachable)?;

    let status = response.status();
    if status == 401 || status == 403 {
        return Err(HaConnectFailureReason::Unauthorized);
    }
    if status != 200 {
        return Err(HaConnectFailureReason::Unreachable);
    }

    let mut buf = [0u8; 2048];
    let mut total = 0usize;
    loop {
        let n = response
            .read(&mut buf[total..])
            .map_err(|_| HaConnectFailureReason::Unreachable)?;
        if n == 0 {
            break;
        }
        total += n;
        if total >= buf.len() {
            break;
        }
    }

    let parsed: HaConfigResponse =
        serde_json::from_slice(&buf[..total]).map_err(|_| HaConnectFailureReason::Unreachable)?;

    Ok(parsed.location_name)
}
