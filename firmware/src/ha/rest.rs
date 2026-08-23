use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use embedded_svc::io::Read;
use esp_idf_svc::http::client::{Configuration as HttpClientConfig, EspHttpConnection};
use esp_idf_svc::tls::X509;

use super::tofu;
use crate::portal_api::HaConnectFailureReason;

#[derive(serde::Deserialize)]
struct HaConfigResponse {
    location_name: String,
}

/// Let's Encrypt's actual (self-signed) root, pinned as a fallback trust anchor. ESP-IDF's
/// built-in certificate bundle (`esp_crt_bundle_attach`, `CONFIG_MBEDTLS_CERTIFICATE_BUNDLE_
/// DEFAULT_FULL`) failed with "No matching trusted root certificate found" against a real HA
/// instance behind Home Assistant Cloud's remote UI (`*.ui.nabu.casa`), which chains through
/// Let's Encrypt's newer "ISRG Root YR" short-lived-cert hierarchy rather than the classic
/// R10/R11 intermediates - that hierarchy is new enough that this ESP-IDF release's bundle
/// snapshot doesn't include it. ISRG Root X1 itself is the actual trust anchor both hierarchies
/// ultimately chain to, so pinning it as a fallback fixes this without needing a newer ESP-IDF
/// release. Downloaded from https://letsencrypt.org/certs/isrgrootx1.pem (SHA256 fingerprint
/// 96:BC:EC:06:26:49:76:F3:74:60:77:9A:CF:28:C5:A7:CF:E8:A3:C0:AA:E1:1A:8F:FC:EE:05:C0:BD:DF:08:C6).
///
/// Pinning X1 alone still failed (mbedtls_ssl_handshake -0x2700, generic X.509 verify failure)
/// even with a confirmed-correct system clock, and pinning both X1 and the intermediate "ISRG
/// Root YR" together still fails the same way - tried a `reqwest`+rustls swap to sidestep
/// ESP-IDF's mbedTLS/esp-tls path entirely, but `ring` (rustls's crypto backend) has no support
/// at all for this board's RISC-V target CPU (`#error "Unknown target CPU"` in its C source),
/// so that's not viable here. Left as a known limitation - see project_hardware memory. The
/// practical workaround is using Home Assistant's local network URL instead of a remote-UI
/// URL chaining through this specific newer CA hierarchy.
const ISRG_ROOT_X1_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIIFazCCA1OgAwIBAgIRAIIQz7DSQONZRGPgu2OCiwAwDQYJKoZIhvcNAQELBQAw\n\
TzELMAkGA1UEBhMCVVMxKTAnBgNVBAoTIEludGVybmV0IFNlY3VyaXR5IFJlc2Vh\n\
cmNoIEdyb3VwMRUwEwYDVQQDEwxJU1JHIFJvb3QgWDEwHhcNMTUwNjA0MTEwNDM4\n\
WhcNMzUwNjA0MTEwNDM4WjBPMQswCQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJu\n\
ZXQgU2VjdXJpdHkgUmVzZWFyY2ggR3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBY\n\
MTCCAiIwDQYJKoZIhvcNAQEBBQADggIPADCCAgoCggIBAK3oJHP0FDfzm54rVygc\n\
h77ct984kIxuPOZXoHj3dcKi/vVqbvYATyjb3miGbESTtrFj/RQSa78f0uoxmyF+\n\
0TM8ukj13Xnfs7j/EvEhmkvBioZxaUpmZmyPfjxwv60pIgbz5MDmgK7iS4+3mX6U\n\
A5/TR5d8mUgjU+g4rk8Kb4Mu0UlXjIB0ttov0DiNewNwIRt18jA8+o+u3dpjq+sW\n\
T8KOEUt+zwvo/7V3LvSye0rgTBIlDHCNAymg4VMk7BPZ7hm/ELNKjD+Jo2FR3qyH\n\
B5T0Y3HsLuJvW5iB4YlcNHlsdu87kGJ55tukmi8mxdAQ4Q7e2RCOFvu396j3x+UC\n\
B5iPNgiV5+I3lg02dZ77DnKxHZu8A/lJBdiB3QW0KtZB6awBdpUKD9jf1b0SHzUv\n\
KBds0pjBqAlkd25HN7rOrFleaJ1/ctaJxQZBKT5ZPt0m9STJEadao0xAH0ahmbWn\n\
OlFuhjuefXKnEgV4We0+UXgVCwOPjdAvBbI+e0ocS3MFEvzG6uBQE3xDk3SzynTn\n\
jh8BCNAw1FtxNrQHusEwMFxIt4I7mKZ9YIqioymCzLq9gwQbooMDQaHWBfEbwrbw\n\
qHyGO0aoSCqI3Haadr8faqU9GY/rOPNk3sgrDQoo//fb4hVC1CLQJ13hef4Y53CI\n\
rU7m2Ys6xt0nUW7/vGT1M0NPAgMBAAGjQjBAMA4GA1UdDwEB/wQEAwIBBjAPBgNV\n\
HRMBAf8EBTADAQH/MB0GA1UdDgQWBBR5tFnme7bl5AFzgAiIyBpY9umbbjANBgkq\n\
hkiG9w0BAQsFAAOCAgEAVR9YqbyyqFDQDLHYGmkgJykIrGF1XIpu+ILlaS/V9lZL\n\
ubhzEFnTIZd+50xx+7LSYK05qAvqFyFWhfFQDlnrzuBZ6brJFe+GnY+EgPbk6ZGQ\n\
3BebYhtF8GaV0nxvwuo77x/Py9auJ/GpsMiu/X1+mvoiBOv/2X/qkSsisRcOj/KK\n\
NFtY2PwByVS5uCbMiogziUwthDyC3+6WVwW6LLv3xLfHTjuCvjHIInNzktHCgKQ5\n\
ORAzI4JMPJ+GslWYHb4phowim57iaztXOoJwTdwJx4nLCgdNbOhdjsnvzqvHu7Ur\n\
TkXWStAmzOVyyghqpZXjFaH3pO3JLF+l+/+sKAIuvtd7u+Nxe5AW0wdeRlN8NwdC\n\
jNPElpzVmbUq4JUagEiuTDkHzsxHpFKVK7q4+63SM1N95R1NbdWhscdCb+ZAJzVc\n\
oyi3B43njTOQ5yOf+1CceWxG1bQVs5ZufpsMljq4Ui0/1lvh+wjChP4kqKOJ2qxq\n\
4RgqsahDYVvTH9w7jXbyLeiNdd8XM2w9U/t7y0Ff/9yi0GE44Za4rF2LN9d11TPA\n\
mRGunUHBcnWEvgJBQl9nJEiU0Zsnvgc/ubhPgXRR4Xq37Z0j4r7g1SgEEzwxA57d\n\
emyPxgcYxn/eR44/KJ4EBs+lVDR3veyJm+kXQ99b21/+jh5Xos1AnX5iItreGCc=\n\
-----END CERTIFICATE-----\n\
-----BEGIN CERTIFICATE-----\n\
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\n\
-----END CERTIFICATE-----\n\0";

/// Optional extra trust anchor embedded at build time from `BUNDLED_CA_PATH` (see
/// `firmware/build.rs`) - for networks where HA's hostname resolves to a certificate signed by a
/// private CA (e.g. a UniFi/UDM controller's own local CA for LAN-only HTTPS) rather than a
/// public one. `esp-x509-crt-bundle`/`ISRG_ROOT_X1_PEM` can never help there since no public
/// bundle has any reason to trust a private CA. Zero-length when unset.
const BUNDLED_CA_PEM: &[u8] = include_bytes!(env!("BUNDLED_CA_PEM_PATH"));

/// `verify_connection`'s result: either resolved outright, or handed back to the caller
/// (`net::ap_portal`'s pending-trust flow) because a human needs to look at an unverifiable
/// certificate before this can proceed.
pub enum VerifyOutcome {
    Connected(String),
    Failed(HaConnectFailureReason),
    NeedsTrustConfirmation(tofu::CapturedCert, String),
}

/// contracts/captive-portal-api.md: POST /api/home-assistant verification step.
///
/// Calls `GET {base_url}/api/config` with the given bearer token. research.md: this single
/// REST call both confirms the token is valid and returns the instance's display name.
///
/// Tries, fully automatically, in order: ESP-IDF's full CA bundle (covers the vast majority of
/// setups); a previously-approved pinned CA from NVS (`stored_pinned_ca`), if any; the optional
/// build-time bundled private CA (`BUNDLED_CA_PEM`); Let's Encrypt's root (`ISRG_ROOT_X1_PEM`),
/// for the common case of a public HA Cloud remote-UI hostname that happens to chain through a
/// hierarchy this ESP-IDF release's bundle lacks. If every one of those fails, this does NOT
/// silently trust whatever certificate the server presents - nothing here can tell a legitimate
/// private CA from a MITM's own, so it captures the cert (`ha::tofu`) and hands it back as
/// `NeedsTrustConfirmation` for a human to approve via the on-screen tap-to-trust prompt.
pub fn verify_connection(
    base_url: &str,
    access_token: &str,
    stored_pinned_ca: Option<&[u8]>,
) -> VerifyOutcome {
    let try_step = |cert: Option<X509<'static>>| match try_connect(base_url, access_token, cert) {
        Ok(instance_name) => Some(VerifyOutcome::Connected(instance_name)),
        Err(HaConnectFailureReason::Unauthorized) => {
            Some(VerifyOutcome::Failed(HaConnectFailureReason::Unauthorized))
        }
        Err(HaConnectFailureReason::Unreachable) => None,
    };

    if let Some(outcome) = try_step(None) {
        return outcome;
    }
    if !base_url.starts_with("https://") {
        return VerifyOutcome::Failed(HaConnectFailureReason::Unreachable);
    }

    if let Some(pinned) = stored_pinned_ca.filter(|p| !p.is_empty()) {
        if let Some(outcome) = try_step(Some(X509::pem_until_nul(leak_pem(pinned.to_vec())))) {
            return outcome;
        }
    }

    if !BUNDLED_CA_PEM.is_empty() {
        if let Some(outcome) = try_step(Some(X509::pem_until_nul(BUNDLED_CA_PEM))) {
            return outcome;
        }
    }

    if let Some(outcome) = try_step(Some(X509::pem_until_nul(ISRG_ROOT_X1_PEM))) {
        return outcome;
    }

    let (host, port) = parse_host_port(base_url);
    match tofu::fetch_peer_cert(&host, port) {
        Ok(captured) => VerifyOutcome::NeedsTrustConfirmation(captured, host),
        Err(e) => {
            log::warn!("TOFU certificate capture from {host}:{port} failed: {e}");
            VerifyOutcome::Failed(HaConnectFailureReason::Unreachable)
        }
    }
}

/// Called once a human has approved the certificate `ha::tofu::fetch_peer_cert` captured (via
/// `VerifyOutcome::NeedsTrustConfirmation`) - re-verifies pinned to exactly that certificate
/// before the caller persists it, rather than trusting the capture unconditionally.
pub fn finalize_trusted(
    base_url: &str,
    access_token: &str,
    pem: &[u8],
) -> Result<String, HaConnectFailureReason> {
    try_connect(
        base_url,
        access_token,
        Some(X509::pem_until_nul(leak_pem(pem.to_vec()))),
    )
}

/// `esp_idf_svc`'s `HttpClientConfig::server_certificate` requires `X509<'static>` - fine for
/// the two consts above (compiled into the binary), but a runtime-sourced PEM (from NVS or a
/// fresh TOFU capture) has no `'static` buffer to borrow from. Leaking it is a deliberate,
/// bounded tradeoff: this only runs on the rare, user-initiated HA-connect step, not a hot path,
/// and each leak is at most a few KB.
fn leak_pem(mut pem: Vec<u8>) -> &'static [u8] {
    if pem.last() != Some(&0) {
        pem.push(0);
    }
    Box::leak(pem.into_boxed_slice())
}

fn parse_host_port(base_url: &str) -> (String, u16) {
    let default_port = if base_url.starts_with("https://") { 443 } else { 80 };
    let rest = base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let authority = rest.split('/').next().unwrap_or(rest);
    match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host.to_string(), port.parse().unwrap_or(default_port))
        }
        _ => (authority.to_string(), default_port),
    }
}

fn try_connect(
    base_url: &str,
    access_token: &str,
    server_certificate: Option<X509<'static>>,
) -> Result<String, HaConnectFailureReason> {
    let is_https = base_url.starts_with("https://");
    let config = HttpClientConfig {
        crt_bundle_attach: (is_https && server_certificate.is_none())
            .then_some(esp_idf_svc::sys::esp_crt_bundle_attach),
        server_certificate,
        ..Default::default()
    };
    let connection = EspHttpConnection::new(&config).map_err(|e| {
        crate::debug_log::event(format!("try_connect: EspHttpConnection::new failed: {e:?}"));
        HaConnectFailureReason::Unreachable
    })?;
    let mut client = HttpClient::wrap(connection);

    let url = format!("{}/api/config", base_url.trim_end_matches('/'));
    let auth_header = format!("Bearer {access_token}");
    let headers = [("authorization", auth_header.as_str())];

    let request = client.request(Method::Get, &url, &headers).map_err(|e| {
        crate::debug_log::event(format!("try_connect: client.request failed: {e:?}"));
        HaConnectFailureReason::Unreachable
    })?;
    let mut response = request.submit().map_err(|e| {
        crate::debug_log::event(format!("try_connect: request.submit failed: {e:?}"));
        HaConnectFailureReason::Unreachable
    })?;

    let status = response.status();
    crate::debug_log::event(format!("try_connect: got status {status}"));
    if status == 401 || status == 403 {
        return Err(HaConnectFailureReason::Unauthorized);
    }
    if status != 200 {
        return Err(HaConnectFailureReason::Unreachable);
    }

    // A fixed 2048-byte buffer was silently truncating this response on real HA instances with
    // many components installed (a genuinely large `/api/config` payload - "components" alone
    // can list 100+ entries) rather than actually erroring, which then failed to parse as
    // truncated JSON. Grows as needed instead, capped well above anything `/api/config`
    // realistically returns.
    const MAX_RESPONSE_LEN: usize = 128 * 1024;
    let mut buf = vec![0u8; 8192];
    let mut total = 0usize;
    loop {
        if total == buf.len() {
            if buf.len() >= MAX_RESPONSE_LEN {
                crate::debug_log::event("try_connect: response exceeded MAX_RESPONSE_LEN");
                return Err(HaConnectFailureReason::Unreachable);
            }
            buf.resize((buf.len() * 2).min(MAX_RESPONSE_LEN), 0);
        }
        let n = response.read(&mut buf[total..]).map_err(|e| {
            crate::debug_log::event(format!("try_connect: response.read failed: {e:?}"));
            HaConnectFailureReason::Unreachable
        })?;
        if n == 0 {
            break;
        }
        total += n;
    }

    let parsed: HaConfigResponse = serde_json::from_slice(&buf[..total]).map_err(|e| {
        crate::debug_log::event(format!(
            "try_connect: JSON parse failed: {e:?}, body={:?}",
            String::from_utf8_lossy(&buf[..total])
        ));
        HaConnectFailureReason::Unreachable
    })?;

    Ok(parsed.location_name)
}
