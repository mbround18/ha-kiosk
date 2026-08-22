use serde::{Deserialize, Serialize};

use crate::storage::SetupState;

/// contracts/captive-portal-api.md: GET /api/networks response entry
#[derive(Serialize)]
pub struct NetworkInfo {
    pub ssid: String,
    pub rssi: i8,
    pub secure: bool,
}

#[derive(Serialize)]
pub struct NetworksResponse {
    pub networks: Vec<NetworkInfo>,
}

/// contracts/captive-portal-api.md: POST /api/wifi request
#[derive(Deserialize)]
pub struct WifiJoinRequest {
    pub ssid: String,
    pub password: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WifiJoinFailureReason {
    WrongPassword,
    NetworkNotFound,
    Other,
}

impl WifiJoinFailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WrongPassword => "wrong_password",
            Self::NetworkNotFound => "network_not_found",
            Self::Other => "other",
        }
    }
}

#[derive(Serialize)]
pub struct WifiJoinResponse {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

impl WifiJoinResponse {
    pub fn connected() -> Self {
        Self {
            status: "connected",
            reason: None,
        }
    }

    pub fn failed(reason: WifiJoinFailureReason) -> Self {
        Self {
            status: "failed",
            reason: Some(reason.as_str()),
        }
    }
}

/// contracts/captive-portal-api.md: POST /api/home-assistant request
#[derive(Deserialize)]
pub struct HaConnectRequest {
    pub base_url: String,
    pub access_token: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HaConnectFailureReason {
    Unreachable,
    Unauthorized,
}

impl HaConnectFailureReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::Unauthorized => "unauthorized",
        }
    }
}

#[derive(Serialize)]
pub struct HaConnectResponse {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

impl HaConnectResponse {
    pub fn connected(instance_name: String) -> Self {
        Self {
            status: "connected",
            instance_name: Some(instance_name),
            reason: None,
        }
    }

    pub fn failed(reason: HaConnectFailureReason) -> Self {
        Self {
            status: "failed",
            instance_name: None,
            reason: Some(reason.as_str()),
        }
    }
}

/// contracts/captive-portal-api.md: GET /api/dashboards response entry
#[derive(Serialize, Deserialize, Clone)]
pub struct DashboardInfo {
    pub url_path: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct DashboardsResponse {
    pub dashboards: Vec<DashboardInfo>,
}

/// contracts/captive-portal-api.md: POST /api/dashboard request
#[derive(Deserialize)]
pub struct DashboardSelectRequest {
    pub url_path: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct DashboardSelectResponse {
    pub status: &'static str,
}

/// contracts/captive-portal-api.md: GET /api/status response
#[derive(Serialize)]
pub struct StatusResponse {
    pub wifi_configured: bool,
    pub ha_configured: bool,
    pub dashboard_selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_name: Option<String>,
}

impl StatusResponse {
    pub fn from_state(state: SetupState, instance_name: Option<String>) -> Self {
        Self {
            wifi_configured: state.wifi_configured,
            ha_configured: state.ha_configured,
            dashboard_selected: state.dashboard_selected,
            instance_name,
        }
    }
}

/// T030: pure-logic unit tests for the portal's JSON contract (contracts/captive-portal-api.md)
/// and failure-reason classification, runnable independent of any ESP-IDF hardware/runtime.
#[cfg(test)]
mod tests {
    #[test]
    fn wifi_join_connected_serializes_without_reason() {
        let json = serde_json::to_string(&WifiJoinResponse::connected()).unwrap();
        assert_eq!(json, r#"{"status":"connected"}"#);
    }

    #[test]
    fn wifi_join_failed_serializes_with_reason() {
        let json =
            serde_json::to_string(&WifiJoinResponse::failed(WifiJoinFailureReason::WrongPassword))
                .unwrap();
        assert_eq!(json, r#"{"status":"failed","reason":"wrong_password"}"#);
    }

    #[test]
    fn wifi_join_failure_reasons_match_contract_strings() {
        assert_eq!(WifiJoinFailureReason::WrongPassword.as_str(), "wrong_password");
        assert_eq!(WifiJoinFailureReason::NetworkNotFound.as_str(), "network_not_found");
        assert_eq!(WifiJoinFailureReason::Other.as_str(), "other");
    }

    #[test]
    fn ha_connect_connected_serializes_with_instance_name_no_reason() {
        let json =
            serde_json::to_string(&HaConnectResponse::connected("My Home".to_string())).unwrap();
        assert_eq!(json, r#"{"status":"connected","instance_name":"My Home"}"#);
    }

    #[test]
    fn ha_connect_failed_serializes_with_reason_no_instance_name() {
        let json =
            serde_json::to_string(&HaConnectResponse::failed(HaConnectFailureReason::Unauthorized))
                .unwrap();
        assert_eq!(json, r#"{"status":"failed","reason":"unauthorized"}"#);
    }

    #[test]
    fn ha_connect_failure_reasons_match_contract_strings() {
        assert_eq!(HaConnectFailureReason::Unreachable.as_str(), "unreachable");
        assert_eq!(HaConnectFailureReason::Unauthorized.as_str(), "unauthorized");
    }

    #[test]
    fn wifi_join_request_deserializes_from_contract_shape() {
        let req: WifiJoinRequest =
            serde_json::from_str(r#"{"ssid":"HomeNetwork","password":"secret"}"#).unwrap();
        assert_eq!(req.ssid, "HomeNetwork");
        assert_eq!(req.password, "secret");
    }

    #[test]
    fn ha_connect_request_deserializes_from_contract_shape() {
        let req: HaConnectRequest = serde_json::from_str(
            r#"{"base_url":"http://homeassistant.local:8123","access_token":"tok"}"#,
        )
        .unwrap();
        assert_eq!(req.base_url, "http://homeassistant.local:8123");
        assert_eq!(req.access_token, "tok");
    }

    #[test]
    fn dashboard_select_request_deserializes_from_contract_shape() {
        let req: DashboardSelectRequest =
            serde_json::from_str(r#"{"url_path":"kiosk-view","title":"Kiosk"}"#).unwrap();
        assert_eq!(req.url_path, "kiosk-view");
        assert_eq!(req.title, "Kiosk");
    }

    #[test]
    fn status_response_omits_instance_name_when_absent() {
        let state = SetupState {
            wifi_configured: true,
            ha_configured: false,
            dashboard_selected: false,
        };
        let json = serde_json::to_string(&StatusResponse::from_state(state, None)).unwrap();
        assert_eq!(
            json,
            r#"{"wifi_configured":true,"ha_configured":false,"dashboard_selected":false}"#
        );
    }

    #[test]
    fn status_response_includes_instance_name_when_present() {
        let state = SetupState {
            wifi_configured: true,
            ha_configured: true,
            dashboard_selected: true,
        };
        let json =
            serde_json::to_string(&StatusResponse::from_state(state, Some("My Home".to_string())))
                .unwrap();
        assert_eq!(
            json,
            r#"{"wifi_configured":true,"ha_configured":true,"dashboard_selected":true,"instance_name":"My Home"}"#
        );
    }

    #[test]
    fn dashboards_response_empty_list_serializes_per_edge_case_contract() {
        let json = serde_json::to_string(&DashboardsResponse { dashboards: vec![] }).unwrap();
        assert_eq!(json, r#"{"dashboards":[]}"#);
    }
}
