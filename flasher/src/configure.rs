//! Auto-provisions a freshly flashed device by replaying the exact same HTTP calls the captive
//! portal's setup web page makes (see `specs/001-wifi-ha-provisioning/contracts/captive-portal-
//! api.md`), using credentials from this repo's `.env` - so a known dev/demo device can skip the
//! on-screen WiFi/Home Assistant setup screens entirely.
//!
//! Requires your computer to already be joined to the device's own setup-mode WiFi network -
//! there's no way to automate that step portably across OSes, so this only covers what happens
//! once you're already talking to the device.

use anyhow::Context;
use serde::Deserialize;
use serde_json::json;

use crate::repo;

#[derive(Deserialize, Debug)]
struct StatusResponse {
    wifi_configured: bool,
    ha_configured: bool,
    dashboard_selected: bool,
    instance_name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct HaConnectResponse {
    status: String,
    instance_name: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct DashboardsResponse {
    dashboards: Vec<DashboardInfo>,
}

#[derive(Deserialize, Debug)]
struct DashboardInfo {
    url_path: String,
    title: String,
}

pub fn run(ip: String, trust_timeout_secs: u64) -> anyhow::Result<()> {
    let env_path = repo::env_file();
    anyhow::ensure!(
        env_path.exists(),
        "no .env found at {} - copy .env.example (see the README) and fill in your WiFi/Home \
         Assistant details first",
        env_path.display()
    );
    dotenvy::from_path(&env_path).with_context(|| format!("failed to read {}", env_path.display()))?;

    let ssid = std::env::var("SSID_NAME").context("SSID_NAME not set in .env")?;
    let password = std::env::var("SSID_PASSWORD").unwrap_or_default();
    let ha_url = std::env::var("HOME_ASSISTANT_URL").context("HOME_ASSISTANT_URL not set in .env")?;
    let ha_token = std::env::var("HOME_ASSISTANT_TOKEN").context("HOME_ASSISTANT_TOKEN not set in .env")?;

    let base = format!("http://{ip}");
    println!("==> Talking to the device's setup API at {base} ...");
    let status: StatusResponse = ureq::get(&format!("{base}/api/status"))
        .call()
        .with_context(|| {
            format!(
                "couldn't reach {base} - make sure your computer is joined to the device's own \
                 setup WiFi network first (its name is shown on the kiosk's screen)"
            )
        })?
        .into_json()
        .context("unexpected response from /api/status")?;

    if status.wifi_configured && status.ha_configured && status.dashboard_selected {
        println!(
            "Already fully configured{} - nothing to do.",
            status
                .instance_name
                .map(|n| format!(" (connected to \"{n}\")"))
                .unwrap_or_default()
        );
        return Ok(());
    }

    if status.wifi_configured {
        println!("WiFi already configured - skipping.");
    } else {
        println!("==> Joining WiFi \"{ssid}\"...");
        let resp: serde_json::Value = ureq::post(&format!("{base}/api/wifi"))
            .send_json(json!({ "ssid": ssid, "password": password }))
            .context("POST /api/wifi failed")?
            .into_json()?;
        anyhow::ensure!(resp["status"] == "connected", "WiFi join failed: {resp}");
        println!("    joined.");
    }

    if status.ha_configured {
        println!("Home Assistant already configured - skipping.");
    } else {
        println!("==> Connecting to Home Assistant ({ha_url})...");
        let resp: HaConnectResponse = ureq::post(&format!("{base}/api/home-assistant"))
            .send_json(json!({ "base_url": ha_url, "access_token": ha_token }))
            .context("POST /api/home-assistant failed")?
            .into_json()?;
        match resp.status.as_str() {
            "connected" => {
                println!("    connected (instance: {}).", resp.instance_name.unwrap_or_default());
            }
            "awaiting_confirmation" => {
                println!("    This device hasn't seen this Home Assistant's TLS certificate before.");
                println!("    Tap \"Trust\" on the kiosk's screen now - waiting up to {trust_timeout_secs}s...");
                wait_for_trust_confirmation(&base, trust_timeout_secs)?;
            }
            other => anyhow::bail!("Home Assistant connection failed: {other} ({:?})", resp.reason),
        }
    }

    if status.dashboard_selected {
        println!("Dashboard already selected - skipping.");
    } else {
        // Kiosk mode is room/area-based now (not a rendered dashboard - see
        // firmware/src/ha/rooms.rs), so this step no longer does anything functionally, but
        // `SetupState.dashboard_selected` still gates leaving setup mode. Pick whatever HA
        // reports first, falling back to a placeholder if there are no dashboards yet at all.
        println!("==> Selecting a dashboard (required to finish setup, unused by kiosk mode)...");
        let dashboards: DashboardsResponse = ureq::get(&format!("{base}/api/dashboards"))
            .call()
            .context("GET /api/dashboards failed")?
            .into_json()?;
        let chosen = dashboards
            .dashboards
            .into_iter()
            .next()
            .unwrap_or(DashboardInfo { url_path: "kiosk".to_string(), title: "Kiosk".to_string() });
        ureq::post(&format!("{base}/api/dashboard"))
            .send_json(json!({ "url_path": chosen.url_path, "title": chosen.title }))
            .context("POST /api/dashboard failed")?;
        println!("    done.");
    }

    println!("==> Setup complete - the device will reboot into kiosk mode.");
    Ok(())
}

fn wait_for_trust_confirmation(base: &str, timeout_secs: u64) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        let resp: HaConnectResponse = ureq::get(&format!("{base}/api/home-assistant/status"))
            .call()
            .context("GET /api/home-assistant/status failed")?
            .into_json()?;
        match resp.status.as_str() {
            "connected" => {
                println!("    confirmed (instance: {}).", resp.instance_name.unwrap_or_default());
                return Ok(());
            }
            "rejected" => anyhow::bail!("Certificate trust was rejected on the device's screen."),
            "timed_out" => anyhow::bail!("Nobody tapped the on-screen trust prompt in time."),
            "awaiting_confirmation" => {
                anyhow::ensure!(
                    std::time::Instant::now() < deadline,
                    "timed out after {timeout_secs}s waiting for the on-screen trust prompt"
                );
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            other => anyhow::bail!("unexpected status while waiting for confirmation: {other}"),
        }
    }
}
