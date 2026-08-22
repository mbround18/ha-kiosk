# Data Model: WiFi & Home Assistant Self-Service Provisioning

## SetupState (derived, not separately persisted)

Computed at boot from which of the three persisted entities below are present and valid. Not
itself stored — it's the union of "is WifiCredentials present", "is HomeAssistantConnection
present and last known-good", "is DashboardSelection present".

| Field | Type | Meaning |
|---|---|---|
| `wifi_configured` | bool | A `WifiCredentials` record exists in NVS |
| `ha_configured` | bool | A `HomeAssistantConnection` record exists in NVS |
| `dashboard_selected` | bool | A `DashboardSelection` record exists in NVS |
| `ethernet_link_up` | bool | Onboard Ethernet PHY reports link-up at boot (not persisted, sampled live) |

**Derived behavior**: device enters AP/captive-portal mode iff `!wifi_configured &&
!ethernet_link_up`, OR the physical "re-enter setup" trigger was asserted at boot. The portal
always renders all three steps; steps whose backing entity already exists are shown as
"configured" (with an option to change) rather than blank.

## WifiCredentials

| Field | Type | Validation |
|---|---|---|
| `ssid` | string | 1–32 bytes (WiFi SSID limit) |
| `password` | string (secret) | 0 (open network) or 8–63 bytes (WPA/WPA2 limit) |

**Persistence**: NVS namespace `kiosk_cfg`, keys `wifi_ssid` / `wifi_pass`. Never logged
(Constitution Principle IV).

**Lifecycle**: written only after a successful STA connection test (FR-005) — never persisted on
a failed attempt, so a bad password never becomes a poisoned saved credential.

## HomeAssistantConnection

| Field | Type | Validation |
|---|---|---|
| `base_url` | string | Must parse as `http://` or `https://` URL with host; no path assumed beyond root |
| `access_token` | string (secret) | Non-empty; treated opaquely (Home Assistant long-lived access token format) |
| `instance_name` | string | Populated from `GET /api/config`'s `location_name` on successful verification; display-only |

**Persistence**: NVS namespace `kiosk_cfg`, keys `ha_url` / `ha_token` / `ha_instance_name`.
`access_token` never logged or redisplayed after entry (Constitution Principle IV, FR-014).

**Lifecycle**: written only after `GET /api/config` succeeds with that token (FR-008) — same
poisoned-credential avoidance as WifiCredentials.

## DashboardSelection

| Field | Type | Validation |
|---|---|---|
| `dashboard_url_path` | string | Must be one of the `url_path` values returned by `lovelace/dashboards/list` at selection time |
| `dashboard_title` | string | Corresponding `title` from the same response; display-only |

**Persistence**: NVS namespace `kiosk_cfg`, keys `ha_dash_path` / `ha_dash_title`.

**Lifecycle**: independently changeable (FR-012/SC-004) — updating this entity alone does not
touch `WifiCredentials` or `HomeAssistantConnection`.

## State Transitions

```
[no config] --(WiFi step succeeds)--> [wifi_configured]
[wifi_configured or ethernet_link_up] --(HA step succeeds)--> [+ha_configured]
[ha_configured] --(dashboard chosen)--> [+dashboard_selected]  == setup complete ==

[setup complete] --(re-enter-setup trigger)--> [AP mode, all steps editable,
                                                 existing entities retained until
                                                 their own step is resubmitted]

[wifi_configured] --(saved WiFi network unreachable at boot)--> [AP mode forced,
                                                                  ha_configured and
                                                                  dashboard_selected retained]
```
