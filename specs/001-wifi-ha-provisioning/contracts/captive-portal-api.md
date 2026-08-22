# Contract: Device Captive Portal HTTP API

Served by the device's own `esp_http_server` while in AP/setup mode, on the device's AP IP
(conventionally `192.168.4.1`), port 80. All endpoints are same-origin with the setup page
itself; no CORS handling needed. All request/response bodies are JSON unless noted.

## Captive portal detection endpoints

OS connectivity-check probes must get a response that triggers the OS's captive-portal UI
(FR-003). Exact per-OS paths (subject to change by OS vendors, verify at implementation time):

| OS | Probe path | Expected portal behavior |
|---|---|---|
| Android | `/generate_204` | Respond with something other than bare 204 (e.g. redirect or 200 with body) |
| iOS/macOS | `/hotspot-detect.html` | Respond with content that does NOT match Apple's expected "Success" body |
| Windows | `/connecttest.txt` or `/ncsi.txt` | Respond with content that does NOT match the expected string |

All other unmatched paths: redirect (`302`) to `/` (the setup page), which the DNS hijack layer
makes resolvable for literally any hostname.

## `GET /`

Setup page: a single-page React app (`firmware/portal-ui`, Vite + TypeScript + shadcn/ui),
built ahead of time and embedded into the firmware binary as static assets (see
`firmware/build.rs` and `firmware/src/net/ap_portal.rs`'s `EMBEDDED_ASSETS` table — no
filesystem partition involved). Must reflect current `SetupState` so already-configured steps
show as such rather than blank. The app's first screen is a splash view showing two QR codes —
one to join this device's own WiFi AP directly, one to open this same URL — before proceeding
into the WiFi/HA/dashboard steps; see `GET /api/portal-info`.

## `GET /api/portal-info`

Returns this device's own AP network name and IP, so the splash screen can render QR codes for
them (a `WIFI:` URI for the SSID, a plain URL for the IP) without hardcoding either.

**Response 200**:
```json
{ "ssid": "HA-Kiosk-1BFD", "ip": "192.168.71.1" }
```

## `GET /api/networks`

Returns nearby WiFi networks for the picker (FR-004).

**Response 200**:
```json
{
  "networks": [
    { "ssid": "HomeNetwork", "rssi": -52, "secure": true }
  ]
}
```

## `POST /api/wifi`

Attempt to join a home network (FR-005).

**Request**:
```json
{ "ssid": "HomeNetwork", "password": "..." }
```

**Response 200** (join succeeded, credentials now persisted):
```json
{ "status": "connected" }
```

**Response 4xx** (join failed, nothing persisted):
```json
{ "status": "failed", "reason": "wrong_password" }
```
`reason` is one of `wrong_password`, `network_not_found`, `other`, matching FR-005's requirement
to distinguish failure causes where the underlying stack allows it.

## `POST /api/home-assistant`

Verify and save Home Assistant connection details (FR-007/FR-008).

**Request**:
```json
{ "base_url": "http://homeassistant.local:8123", "access_token": "..." }
```

**Response 200** (token verified against `GET {base_url}/api/config`, now persisted):
```json
{ "status": "connected", "instance_name": "My Home" }
```

**Response 4xx** (verification failed, nothing persisted):
```json
{ "status": "failed", "reason": "unreachable" }
```
`reason` is one of `unreachable` (couldn't reach `base_url` at all) or `unauthorized` (reached it,
token rejected), matching FR-008's plain-language distinction requirement.

## `GET /api/dashboards`

Lists dashboards from the already-verified Home Assistant connection (FR-009), by opening a
one-shot authenticated WebSocket connection server-side and issuing `lovelace/dashboards/list`.

**Response 200**:
```json
{
  "dashboards": [
    { "url_path": "lovelace", "title": "Overview" },
    { "url_path": "kiosk-view", "title": "Kiosk" }
  ]
}
```

**Response 200, empty list** (Edge Case: no dashboards exist yet):
```json
{ "dashboards": [] }
```
Caller (the setup page) is responsible for showing the "no dashboards yet" guidance from the
spec's Edge Cases — this endpoint just reports what Home Assistant returned.

## `POST /api/dashboard`

Save the chosen dashboard (FR-010).

**Request**:
```json
{ "url_path": "kiosk-view", "title": "Kiosk" }
```

**Response 200**:
```json
{ "status": "saved" }
```

## `GET /api/status`

Returns current `SetupState` (used by the setup page to decide what to render, and by the "setup
complete" transition to know when to stop broadcasting the AP per FR-006).

**Response 200**:
```json
{
  "wifi_configured": true,
  "ha_configured": true,
  "dashboard_selected": false,
  "instance_name": "My Home"
}
```
