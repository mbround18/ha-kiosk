use std::time::Duration;

use embedded_svc::ws::FrameType;
use esp_idf_svc::ws::client::{EspWebSocketClient, EspWebSocketClientConfig, WebSocketEventType};

use crate::portal_api::DashboardInfo;

#[derive(serde::Deserialize)]
struct HaWsMessage<'a> {
    #[serde(rename = "type")]
    msg_type: &'a str,
    #[serde(default)]
    success: bool,
    #[serde(default)]
    result: Option<Vec<HaDashboardEntry>>,
}

#[derive(serde::Deserialize)]
struct HaDashboardEntry {
    url_path: String,
    title: String,
}

/// research.md: dashboard listing is only available over Home Assistant's WebSocket API, as a
/// one-shot authenticated call - not a persistent runtime connection (that's a later milestone).
pub fn list_dashboards(base_url: &str, access_token: &str) -> anyhow::Result<Vec<DashboardInfo>> {
    let ws_url = to_ws_url(base_url);

    let config = EspWebSocketClientConfig {
        task_stack: 8192,
        buffer_size: 4096,
        ..Default::default()
    };

    let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let received_cb = received.clone();

    let mut client = EspWebSocketClient::new(&ws_url, &config, Duration::from_secs(10), move |event| {
        if let Ok(evt) = event {
            if let WebSocketEventType::Text(text) = evt.event_type {
                received_cb.lock().unwrap().push(text.to_string());
            }
        }
    })?;

    // 1. Wait for `auth_required`, then send our token.
    wait_for_message(&received, |msg| msg.msg_type == "auth_required", Duration::from_secs(10))?;
    let auth_msg = serde_json::json!({ "type": "auth", "access_token": access_token });
    client.send(FrameType::Text(false), auth_msg.to_string().as_bytes())?;

    // 2. Wait for `auth_ok` (or bail on `auth_invalid`).
    let auth_result = wait_for_message(
        &received,
        |msg| msg.msg_type == "auth_ok" || msg.msg_type == "auth_invalid",
        Duration::from_secs(10),
    )?;
    if auth_result.msg_type != "auth_ok" {
        anyhow::bail!("Home Assistant WebSocket auth rejected");
    }

    // 3. Ask for the dashboard list.
    let list_msg = serde_json::json!({ "id": 1, "type": "lovelace/dashboards/list" });
    client.send(FrameType::Text(false), list_msg.to_string().as_bytes())?;

    let result_msg = wait_for_message(&received, |msg| msg.success, Duration::from_secs(10))?;
    let entries = result_msg.result.unwrap_or_default();

    Ok(entries
        .into_iter()
        .map(|e| DashboardInfo {
            url_path: e.url_path,
            title: e.title,
        })
        .collect())
}

fn wait_for_message(
    received: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    matches: impl Fn(&HaWsMessage) -> bool,
    timeout: Duration,
) -> anyhow::Result<OwnedHaWsMessage> {
    let deadline = std::time::Instant::now() + timeout;
    let mut checked = 0usize;
    loop {
        {
            let buf = received.lock().unwrap();
            while checked < buf.len() {
                let raw = &buf[checked];
                checked += 1;
                if let Ok(parsed) = serde_json::from_str::<HaWsMessage>(raw) {
                    if matches(&parsed) {
                        return Ok(OwnedHaWsMessage {
                            msg_type: parsed.msg_type.to_string(),
                            result: parsed.result,
                        });
                    }
                }
            }
        }
        if std::time::Instant::now() > deadline {
            anyhow::bail!("timed out waiting for Home Assistant WebSocket message");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct OwnedHaWsMessage {
    msg_type: String,
    result: Option<Vec<HaDashboardEntry>>,
}

fn to_ws_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}/api/websocket")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}/api/websocket")
    } else {
        format!("ws://{trimmed}/api/websocket")
    }
}
