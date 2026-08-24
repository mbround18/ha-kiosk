//! Per-room ("area", in Home Assistant's terminology) light on/off control - the kiosk's actual
//! runtime UI (`display::kiosk`), not part of the captive-portal setup flow. Deliberately scoped
//! to just on/off: HA's real dashboards (see research done via `lovelace/config`) lean on the
//! modern "sections" layout, auto-generated `area` cards, `tile` cards with feature sub-widgets,
//! and even third-party HACS cards (`custom:bubble-card`) - natively reproducing all of that
//! isn't tractable (there's no way to keep up with arbitrary custom card JavaScript), so this
//! renders its own simple native UI instead, driven directly by HA's area/entity registries
//! rather than any dashboard's card config.
//!
//! Uses its own minimal one-shot WebSocket request/response helper rather than sharing
//! `ha::dashboards`'s (which is a single fixed three-message exchange, not a general
//! request/response client) - kept self-contained to avoid touching that already-proven code.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use embedded_svc::ws::FrameType;
use esp_idf_svc::ws::client::{EspWebSocketClient, EspWebSocketClientConfig, WebSocketEventType};
use serde_json::Value;

/// `Room`/`Weather`/`Climate` are plain data with no ESP-IDF dependency, so they're defined in
/// `firmware_core::home` (a host-testable crate) instead of here - this module is just the
/// WebSocket fetching logic that populates them.
pub use firmware_core::home::{Climate, Clock, Room, Weather};

/// A request/response HA WebSocket session - connects, authenticates, and lets the caller make
/// any number of `call` requests. Home Assistant assigns meaning to the numeric "id" field
/// itself (matching each response to its request), not to any per-connection state, which is
/// exactly what makes it safe for `net::kiosk` to keep one of these open and reuse it for the
/// kiosk's whole runtime (see that module's doc comment) rather than reconnecting every refresh
/// cycle - reconnecting that often turned out to be the actual cause of a real heap leak
/// (confirmed via on-device serial logs measuring internal heap loss on every refresh cycle).
pub(crate) struct HaWsSession {
    client: EspWebSocketClient<'static>,
    received: Arc<Mutex<Vec<String>>>,
    next_id: u32,
}

impl HaWsSession {
    pub(crate) fn connect(base_url: &str, access_token: &str) -> anyhow::Result<Self> {
        let ws_url = to_ws_url(base_url);
        let config = EspWebSocketClientConfig {
            task_stack: 8192,
            buffer_size: 8192,
            crt_bundle_attach: ws_url
                .starts_with("wss://")
                .then_some(esp_idf_svc::sys::esp_crt_bundle_attach),
            ..Default::default()
        };

        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let received_cb = received.clone();
        let client = EspWebSocketClient::new(&ws_url, &config, Duration::from_secs(10), move |event| {
            if let Ok(evt) = event {
                if let WebSocketEventType::Text(text) = evt.event_type {
                    received_cb.lock().unwrap().push(text.to_string());
                }
            }
        })?;

        let mut session = Self {
            client,
            received,
            next_id: 1,
        };

        session.wait_for(Duration::from_secs(10), |msg| msg["type"] == "auth_required")?;
        session.send_raw(&serde_json::json!({ "type": "auth", "access_token": access_token }))?;
        let auth = session.wait_for(Duration::from_secs(10), |msg| {
            msg["type"] == "auth_ok" || msg["type"] == "auth_invalid"
        })?;
        anyhow::ensure!(auth["type"] == "auth_ok", "Home Assistant WebSocket auth rejected");

        Ok(session)
    }

    /// Sends `{"id": <next>, "type": msg_type, ...extra}` and waits for the response with that
    /// same id, returning its `result` field.
    fn call(&mut self, msg_type: &str, extra: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let mut payload = extra;
        payload["id"] = Value::from(id);
        payload["type"] = Value::from(msg_type);
        self.send_raw(&payload)?;

        let response = self.wait_for(Duration::from_secs(10), move |msg| {
            msg["id"].as_u64() == Some(id as u64)
        })?;
        anyhow::ensure!(
            response["success"].as_bool().unwrap_or(true),
            "Home Assistant rejected {msg_type}: {:?}",
            response["error"]
        );
        Ok(response
            .get("result")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Evaluates a one-shot Jinja template server-side via HA's `render_template` command and
    /// returns its result. Unlike `call`, this is a two-message exchange: an immediate
    /// `{"type": "result", ...}` ack that the subscription was accepted, followed by a separate
    /// `{"type": "event", "event": {"result": ...}}` carrying the actual rendered value (HA
    /// treats every `render_template` as a live subscription that would keep re-sending on
    /// change).
    ///
    /// Explicitly unsubscribes once the one value this is actually used for has arrived, rather
    /// than just letting every one of these subscriptions (one per room, plus weather/climate/
    /// clock - several per session) ride until the whole connection drops at the end of
    /// `fetch_dashboard_data`. Leaving them open was the cause of a real heap leak, confirmed via
    /// on-device serial logs: every session's `esp_websocket_client_close` was logging
    /// "Connection terminated while waiting for clean TCP close" / "unexpected data readable on
    /// socket" - HA pushing an unread update on one of these still-live subscriptions right as
    /// the connection was closing, which the underlying C client doesn't handle cleanly. Measured
    /// on hardware: internal heap dropping ~750 bytes on almost every 15s refresh cycle, which
    /// exhausts this board's ~220KB internal heap in roughly 2 hours - the "state goes stale
    /// after a few hours" bug this was written to fix.
    fn render_template(&mut self, template: &str) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        self.send_raw(&serde_json::json!({
            "id": id,
            "type": "render_template",
            "template": template,
        }))?;

        let ack = self.wait_for(Duration::from_secs(10), move |msg| {
            msg["id"].as_u64() == Some(id as u64) && msg["type"] == "result"
        })?;
        anyhow::ensure!(
            ack["success"].as_bool().unwrap_or(false),
            "render_template rejected: {:?}",
            ack["error"]
        );

        let event = self.wait_for(Duration::from_secs(10), move |msg| {
            msg["id"].as_u64() == Some(id as u64) && msg["type"] == "event"
        })?;
        let result = event["event"]["result"].clone();

        // Fire-and-forget: HA acks this too, but that ack isn't worth an extra round trip since
        // there's nothing left to do with this subscription either way - the goal is just to
        // stop it from pushing anything more once this session closes.
        let unsubscribe_id = self.next_id;
        self.next_id += 1;
        if let Err(e) = self.send_raw(&serde_json::json!({
            "id": unsubscribe_id,
            "type": "unsubscribe_events",
            "subscription": id,
        })) {
            log::warn!("render_template: failed to unsubscribe from subscription {id}: {e}");
        }

        Ok(result)
    }

    fn send_raw(&mut self, payload: &Value) -> anyhow::Result<()> {
        self.client
            .send(FrameType::Text(false), payload.to_string().as_bytes())?;
        Ok(())
    }

    /// Scans `received` for the first message matching `matches`, blocking (polling every 50ms)
    /// until one arrives or `timeout` elapses.
    ///
    /// Every match found - and everything that arrived before it, matched or not - is drained
    /// out of `received` before returning. That's safe because calls are made and awaited
    /// strictly in increasing id order (nothing is ever awaited "out of turn"), so anything
    /// sitting before a just-matched message can only be: an earlier call's response we already
    /// consumed and moved past, or a fire-and-forget message (e.g. `render_template`'s
    /// `unsubscribe_events`) nobody was ever going to wait for. Either way it's dead weight.
    /// Without this, `received` would simply grow for as long as the session stays open - fine
    /// for the old one-connection-per-fetch design where a session only lived a few seconds, but
    /// unbounded (and, worse, rescanned from index 0 on every single call, so also
    /// O(session-lifetime) work per call) on a session meant to live for the kiosk's entire
    /// runtime.
    fn wait_for(
        &mut self,
        timeout: Duration,
        matches: impl Fn(&Value) -> bool,
    ) -> anyhow::Result<Value> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            {
                let mut buf = self.received.lock().unwrap();
                let mut found_at = None;
                for (i, raw) in buf.iter().enumerate() {
                    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                        if matches(&parsed) {
                            found_at = Some((i, parsed));
                            break;
                        }
                    }
                }
                if let Some((i, parsed)) = found_at {
                    buf.drain(0..=i);
                    return Ok(parsed);
                }
            }
            if std::time::Instant::now() > deadline {
                anyhow::bail!("timed out waiting for Home Assistant WebSocket message");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Rooms, weather, climate, and clock over an already-connected session (see `net::kiosk`'s
/// persistent `HaWsSession`) rather than opening a fresh connection - the kiosk's main loop
/// refreshes all of these together on the same cadence. Weather/climate/clock are best-effort: a
/// failure in any one of them (e.g. no `weather.*` entity configured) shouldn't take down the
/// room list, which is the one part of this that actually has to succeed for the screen to show
/// anything meaningful.
pub(crate) fn fetch_dashboard_data(
    session: &mut HaWsSession,
) -> anyhow::Result<(Vec<Room>, Option<Weather>, Option<Climate>, Option<Clock>)> {
    let rooms = fetch_rooms(session)?;
    let weather = match fetch_weather(session) {
        Ok(w) => Some(w),
        Err(e) => {
            crate::debug_log::event(format!("fetch_dashboard_data: weather fetch failed: {e:?}"));
            None
        }
    };
    let climate = match fetch_climate(session) {
        Ok(c) => Some(c),
        Err(e) => {
            crate::debug_log::event(format!("fetch_dashboard_data: climate fetch failed: {e:?}"));
            None
        }
    };
    let clock = match fetch_clock(session) {
        Ok(c) => Some(c),
        Err(e) => {
            crate::debug_log::event(format!("fetch_dashboard_data: clock fetch failed: {e:?}"));
            None
        }
    };
    Ok((rooms, weather, climate, clock))
}

/// Every room with at least one `light.*` entity, sorted by name.
///
/// Deliberately does NOT use `config/entity_registry/list` (or `device_registry/list`) to work
/// this out - on a real, long-lived HA instance those can be enormous (confirmed against a real
/// instance: the full entity registry alone was **1.3MB**, the device registry 134KB), utterly
/// impractical to buffer over a WebSocket on this hardware. Instead, for each area (that list is
/// small - typically a few KB), asks HA itself to resolve the area's light entities via a Jinja
/// `render_template` call (`area_entities(...)`) - HA does the registry lookups server-side and
/// hands back just a short list of entity ids per area, plus (via the same call, to keep this to
/// one round trip per area) whether any of them is currently on.
fn fetch_rooms(session: &mut HaWsSession) -> anyhow::Result<Vec<Room>> {
    let areas = session.call("config/area_registry/list", serde_json::json!({}))?;

    let mut rooms = Vec::new();
    for area in areas.as_array().into_iter().flatten() {
        let Some(area_id) = area["area_id"].as_str() else {
            continue;
        };
        let name = area["name"].as_str().unwrap_or(area_id).to_string();

        // `unavailable` is true when every light in the area is currently `unavailable` in HA
        // (e.g. its physical power switch was cut, or it lost network) - HA itself is telling us
        // it can't confirm any state at all for this room, so on/off is meaningless here and
        // `display::kiosk` grays the toggle out instead of showing a potentially-fictional state.
        let result = session.render_template(&format!(
            "{{% set lights = area_entities('{}') | select('match', '^light\\\\.') | list %}}\
             {{{{ {{'lights': lights, 'any_on': lights | select('is_state', 'on') | list | length > 0, \
             'unavailable': lights | reject('is_state', 'unavailable') | list | length == 0}} | tojson }}}}",
            escape_jinja_string(area_id)
        ))?;
        let parsed: Value = match result {
            Value::String(s) => serde_json::from_str(&s)?,
            other => other,
        };
        let has_lights = parsed["lights"].as_array().is_some_and(|a| !a.is_empty());
        if has_lights {
            rooms.push(Room {
                area_id: area_id.to_string(),
                name,
                is_on: parsed["any_on"].as_bool().unwrap_or(false),
                unavailable: parsed["unavailable"].as_bool().unwrap_or(false),
            });
        }
    }
    rooms.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(rooms)
}

/// Area ids come from HA's own registry (not user-typed input reaching us directly), so this is
/// a defensive minimum rather than a real injection concern - just enough to keep a literal `'`
/// from breaking out of the Jinja string it's embedded in above.
fn escape_jinja_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Current outdoor weather (from the instance's first `weather.*` entity, if any) plus a
/// whole-home average of every `sensor.*` entity whose `device_class` is `temperature` AND
/// whose reading falls within a plausible ambient-room range. Both are computed server-side in
/// a single Jinja `render_template` call (rather than e.g. `get_states` and averaging
/// client-side) since a real HA instance can have hundreds of sensors - same rationale as
/// `fetch_rooms` avoiding the full entity registry. HA's `average` template filter does the
/// mean; `tojson` turns the whole result into a single JSON string so this stays one round trip
/// instead of several separate template calls.
///
/// The ambient-range clamp and appliance denylist exist because `device_class: temperature`
/// isn't just room sensors - a real home can have oven/appliance probes, grill thermometers,
/// etc. tagged the same way, and one 350°F oven reading is enough to badly skew a "whole home"
/// average (confirmed on a real instance: an appliance sensor pulled the average up to 103°F).
/// The range bound (120°F/49°C) is generous - well above normal room temperature but below
/// typical cooking-appliance readings - specifically to only reject clearly-not-ambient
/// readings, not to be a precise "is this a room sensor" classifier; the name-based denylist is
/// the same idea from the other direction, catching appliance sensors whose *current* reading
/// happens to look ambient (e.g. an oven that's currently off).
fn fetch_weather(session: &mut HaWsSession) -> anyhow::Result<Weather> {
    const TEMPLATE: &str = r#"
{% set w = states.weather | list | first %}
{% set unit = w.attributes.temperature_unit if w and w.attributes.temperature_unit is defined else '°F' %}
{% set lo, hi = (32, 120) if unit == '°F' else (0, 49) %}
{% set appliance_pattern = '(?i)(oven|grill|stove|smoker|dryer|washer|fridge|freezer|dishwasher|kettle|toaster|microwave)' %}
{% set indoor = states.sensor
    | selectattr('attributes.device_class', 'defined')
    | selectattr('attributes.device_class', 'eq', 'temperature')
    | rejectattr('entity_id', 'search', appliance_pattern)
    | rejectattr('name', 'search', appliance_pattern)
    | map(attribute='state')
    | select('is_number')
    | map('float')
    | select('ge', lo)
    | select('le', hi)
    | list %}
{% set indoor_humidity = states.sensor
    | selectattr('attributes.device_class', 'defined')
    | selectattr('attributes.device_class', 'eq', 'humidity')
    | rejectattr('entity_id', 'search', appliance_pattern)
    | rejectattr('name', 'search', appliance_pattern)
    | map(attribute='state')
    | select('is_number')
    | map('float')
    | select('ge', 0)
    | select('le', 100)
    | list %}
{{ {
    'condition': w.state if w else none,
    'temp': (w.attributes.temperature | float) if w and w.attributes.temperature is defined else none,
    'unit': w.attributes.temperature_unit if w and w.attributes.temperature_unit is defined else none,
    'avg_indoor': (indoor | average) if indoor | length > 0 else none,
    'avg_humidity': (indoor_humidity | average) if indoor_humidity | length > 0 else none
} | tojson }}
"#;

    let result = session.render_template(TEMPLATE)?;
    // HA's `render_template` command tries to coerce a rendered string that looks like JSON
    // back into a native JSON value before sending it over the WebSocket - so despite this
    // template ending in `| tojson`, `result` can arrive either as that JSON text (a string) or
    // already as the parsed object, depending on HA version/behavior. Handle both.
    let parsed: Value = match result {
        Value::String(s) => serde_json::from_str(&s)?,
        other => other,
    };

    Ok(Weather {
        condition: parsed["condition"].as_str().unwrap_or("Unknown").to_string(),
        temp: parsed["temp"].as_f64(),
        unit: parsed["unit"].as_str().unwrap_or("°").to_string(),
        avg_home_temp: parsed["avg_indoor"].as_f64(),
        avg_home_humidity: parsed["avg_humidity"].as_f64(),
    })
}

/// Every `climate.*` entity's current HVAC mode (the entity's `state`) and sensed temperature
/// (`attributes.current_temperature`) - one `render_template` call, same rationale as
/// `fetch_weather` for keeping this to a single round trip regardless of how many units exist.
/// Mode counting happens client-side here (rather than in the template) since it's a handful of
/// short strings at most - simpler than building a Jinja `groupby` expression for it.
fn fetch_climate(session: &mut HaWsSession) -> anyhow::Result<Climate> {
    const TEMPLATE: &str = r#"
{% set climates = states.climate | list %}
{% set temps = climates
    | map(attribute='attributes.current_temperature')
    | select('is_number')
    | map('float')
    | list %}
{{ {
    'avg_temp': (temps | average) if temps | length > 0 else none,
    'modes': climates | map(attribute='state') | list
} | tojson }}
"#;

    let result = session.render_template(TEMPLATE)?;
    let parsed: Value = match result {
        Value::String(s) => serde_json::from_str(&s)?,
        other => other,
    };

    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for mode in parsed["modes"].as_array().into_iter().flatten() {
        if let Some(mode) = mode.as_str() {
            *counts.entry(mode.to_string()).or_insert(0) += 1;
        }
    }
    let mut mode_counts: Vec<(String, u32)> = counts.into_iter().collect();
    mode_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    Ok(Climate {
        avg_current_temp: parsed["avg_temp"].as_f64(),
        mode_counts,
    })
}

/// Current local time/date, pre-formatted by HA's own `now()` (which already resolves to the
/// instance's configured timezone) rather than the device's own clock - see `Clock`'s doc
/// comment for why. `%I`/`%d` (not `%-I`/`%-d`) deliberately keep the leading zero: the
/// no-leading-zero glibc extensions aren't guaranteed to exist on every platform HA might be
/// running on (e.g. a musl-based container), and a screensaver clock reading "08:45 AM" instead
/// of "8:45 AM" isn't worth risking a template error over.
fn fetch_clock(session: &mut HaWsSession) -> anyhow::Result<Clock> {
    const TEMPLATE: &str = r#"
{{ {
    'time': now().strftime('%I:%M %p'),
    'date': now().strftime('%A, %B %d')
} | tojson }}
"#;

    let result = session.render_template(TEMPLATE)?;
    let parsed: Value = match result {
        Value::String(s) => serde_json::from_str(&s)?,
        other => other,
    };

    Ok(Clock {
        time: parsed["time"].as_str().unwrap_or("").to_string(),
        date: parsed["date"].as_str().unwrap_or("").to_string(),
    })
}

/// Turns every light in the given area on or off via HA's area-targeted `light.turn_on`/
/// `light.turn_off` service call - HA resolves the area to its member light entities itself, so
/// this doesn't need to enumerate them. Takes an already-connected session for the same reason
/// as `fetch_dashboard_data` above.
pub(crate) fn set_room_lights(session: &mut HaWsSession, area_id: &str, on: bool) -> anyhow::Result<()> {
    session.call(
        "call_service",
        serde_json::json!({
            "domain": "light",
            "service": if on { "turn_on" } else { "turn_off" },
            "target": { "area_id": [area_id] },
        }),
    )?;
    Ok(())
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
