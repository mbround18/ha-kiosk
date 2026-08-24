use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};

/// Defined in `firmware_core::portal_api` (not here) so it stays usable from that crate's
/// hardware-independent, host-testable `StatusResponse::from_state` without depending on
/// anything ESP-IDF-specific - see that module's doc comment.
pub use firmware_core::portal_api::SetupState;

const NAMESPACE: &str = "kiosk_cfg";

/// data-model.md: WifiCredentials
pub struct WifiCredentials {
    pub ssid: String,
    pub password: String,
}

/// data-model.md: HomeAssistantConnection
#[derive(Clone)]
pub struct HomeAssistantConnection {
    pub base_url: String,
    pub access_token: String,
    pub instance_name: String,
}

/// data-model.md: DashboardSelection
pub struct DashboardSelection {
    pub url_path: String,
    pub title: String,
}

pub struct Storage {
    nvs: EspNvs<NvsDefault>,
}

impl Storage {
    pub fn new(partition: EspDefaultNvsPartition) -> anyhow::Result<Self> {
        let nvs = EspNvs::new(partition, NAMESPACE, true)?;
        Ok(Self { nvs })
    }

    fn key_exists(&self, name: &str) -> bool {
        self.nvs.find_key(name).ok().flatten().is_some()
    }

    pub fn setup_state(&self) -> SetupState {
        SetupState {
            wifi_configured: self.key_exists("wifi_ssid"),
            ha_configured: self.key_exists("ha_url"),
            dashboard_selected: self.key_exists("ha_dash_path"),
        }
    }

    pub fn get_wifi_credentials(&self) -> Option<WifiCredentials> {
        let mut ssid_buf = [0u8; 33];
        let mut pass_buf = [0u8; 65];
        let ssid = self.nvs.get_str("wifi_ssid", &mut ssid_buf).ok()??;
        let password = self
            .nvs
            .get_str("wifi_pass", &mut pass_buf)
            .ok()?
            .unwrap_or("");
        Some(WifiCredentials {
            ssid: ssid.to_string(),
            password: password.to_string(),
        })
    }

    /// Only ever called after a verified-successful WiFi join (FR-005 / research.md).
    pub fn set_wifi_credentials(&mut self, ssid: &str, password: &str) -> anyhow::Result<()> {
        self.nvs.set_str("wifi_ssid", ssid)?;
        self.nvs.set_str("wifi_pass", password)?;
        Ok(())
    }

    pub fn get_ha_connection(&self) -> Option<HomeAssistantConnection> {
        let mut url_buf = [0u8; 256];
        let mut token_buf = [0u8; 512];
        let mut name_buf = [0u8; 128];
        let base_url = self.nvs.get_str("ha_url", &mut url_buf).ok()??;
        let access_token = self.nvs.get_str("ha_token", &mut token_buf).ok()??;
        let instance_name = self
            .nvs
            .get_str("ha_instance_name", &mut name_buf)
            .ok()?
            .unwrap_or("");
        Some(HomeAssistantConnection {
            base_url: base_url.to_string(),
            access_token: access_token.to_string(),
            instance_name: instance_name.to_string(),
        })
    }

    /// Only ever called after a verified-successful GET /api/config call (FR-008 / research.md).
    pub fn set_ha_connection(
        &mut self,
        base_url: &str,
        access_token: &str,
        instance_name: &str,
    ) -> anyhow::Result<()> {
        self.nvs.set_str("ha_url", base_url)?;
        self.nvs.set_str("ha_token", access_token)?;
        self.nvs.set_str("ha_instance_name", instance_name)?;
        Ok(())
    }

    pub fn set_dashboard(&mut self, url_path: &str, title: &str) -> anyhow::Result<()> {
        self.nvs.set_str("ha_dash_path", url_path)?;
        self.nvs.set_str("ha_dash_title", title)?;
        Ok(())
    }

    pub fn get_dashboard(&self) -> Option<DashboardSelection> {
        let mut path_buf = [0u8; 128];
        let mut title_buf = [0u8; 128];
        let url_path = self.nvs.get_str("ha_dash_path", &mut path_buf).ok()??;
        let title = self
            .nvs
            .get_str("ha_dash_title", &mut title_buf)
            .ok()?
            .unwrap_or("");
        Some(DashboardSelection {
            url_path: url_path.to_string(),
            title: title.to_string(),
        })
    }

    /// FR-012 edge case: clear only the WiFi step so a device with a dead saved network can be
    /// re-provisioned without losing its Home Assistant connection or dashboard choice.
    pub fn clear_wifi_credentials(&mut self) -> anyhow::Result<()> {
        self.nvs.remove("wifi_ssid")?;
        self.nvs.remove("wifi_pass")?;
        Ok(())
    }

    /// A PEM-encoded CA trust anchor captured at runtime via TOFU (`ha::tofu`) for HA instances
    /// behind a private CA - see `ha::rest::verify_connection`. Persisted so the capture dance
    /// only ever has to happen once per HA instance (after a human approves it via the
    /// on-screen tap-to-trust prompt), not on every boot.
    pub fn get_pinned_ca(&self) -> Option<Vec<u8>> {
        let len = self.nvs.blob_len("ha_pinned_ca").ok()??;
        let mut buf = vec![0u8; len];
        self.nvs.get_blob("ha_pinned_ca", &mut buf).ok()??;
        Some(buf)
    }

    pub fn set_pinned_ca(&mut self, pem: &[u8]) -> anyhow::Result<()> {
        self.nvs.set_blob("ha_pinned_ca", pem)?;
        Ok(())
    }
}
