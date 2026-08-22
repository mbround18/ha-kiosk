# Research: WiFi & Home Assistant Self-Service Provisioning

## Decision: Rust std toolchain via ESP-IDF, no bare-metal alternative considered

**Rationale**: Constitution Principle I mandates this already (`esp-idf-svc`/`esp-idf-hal` on
`esp32p4`), driven by the P4's lack of native radio (WiFi/BLE only via the onboard ESP32-C6 over
ESP-Hosted/SDIO) and ESP-IDF's mature Ethernet/`esp_http_server`/NVS support. Already validated
working on this hardware (build + flash + boot confirmed in prior session work).

**Alternatives considered**: `esp-hal`/`esp-wifi` no_std — rejected per constitution, P4 WiFi
support there is not production-ready as of this writing.

## Decision: AP+STA captive portal using ESP-IDF's `esp_http_server` + DNS hijack

**Rationale**: This is the standard, well-proven ESP-IDF pattern (used by ESP-IDF's own
`wifi_provisioning` component and countless "WiFiManager"-style projects): the device starts in
AP mode with its own SSID, runs a DNS server that resolves every hostname to the device's own AP
IP address, and runs an HTTP server on port 80. Most OSes (Android, iOS, Windows) then
auto-detect a captive portal via their own connectivity-check probes and pop the setup page
automatically (FR-003). `esp-idf-svc` exposes both WiFi AP/STA mode control and
`esp_http_server` bindings directly, so no additional crates are needed for this part.

**Alternatives considered**:
- BLE-based provisioning (ESP-IDF's `wifi_provisioning` BLE transport) — rejected: requires a
  companion app, which Constitution Principle II explicitly rules out.
- `improv-wifi` (serial/BLE) — same rejection, and less mature browser-based UX than a plain
  captive portal for this use case.

## Decision: WiFi scan via `esp-idf-svc`'s `EspWifi::scan()`, AP list refreshed on portal load

**Rationale**: Needed for FR-004 (network picker instead of free-text SSID entry, which is more
error-prone for non-technical users). `esp-idf-svc` wraps the underlying ESP-IDF scan API
directly; no extra dependency needed.

**Alternatives considered**: Free-text SSID entry only — rejected as a worse default UX per the
spec's non-technical-user framing, though it remains a fallback for hidden networks (a text entry
option alongside the scanned list, not a replacement for it).

## Decision: Home Assistant verification via REST `GET /api/config`, dashboard listing via WebSocket `lovelace/dashboards/list`

**Rationale**: Home Assistant's REST API supports token-authenticated calls to `/api/config`,
which both confirms the token is valid and returns the instance's configured `location_name` —
sufficient for FR-008's "confirm connection and show instance name." However, the *list of
Lovelace dashboards* (FR-009) is not exposed over REST; it is only available via Home Assistant's
WebSocket API (`lovelace/dashboards/list` command after authenticating the same long-lived
token over the WS handshake). The plan therefore needs both a minimal REST client call (used
once, for verification) and a minimal WebSocket client (used once, for dashboard listing) —
both are one-shot request/response calls during setup, not a persistent runtime connection (that
persistent connection is out of scope for this feature; runtime dashboard *rendering* is a later
milestone per Constitution Principle V).

**Alternatives considered**: Skipping dashboard enumeration and asking the user to type a
dashboard path/URL directly — rejected, worse UX and error-prone for non-technical users (FR-009
explicitly requires presenting dashboards "by name"); kept as an edge-case fallback only if the
list comes back empty (per spec's Edge Cases section).

## Decision: Credentials persisted in ESP-IDF NVS (non-volatile storage), a dedicated namespace

**Rationale**: NVS is ESP-IDF's standard on-flash key-value store, wear-leveled and already
provisioned by the default partition table. `esp-idf-svc`'s `EspNvs` wraps it directly. Satisfies
Constitution Principle IV ("persisted only in non-volatile storage on-device") and FR-011.
Flash encryption/secure storage of NVS is a real hardening option but is out of scope for this
feature (device targets trusted home networks; revisit if threat model changes).

**Alternatives considered**: A file on the SPIFFS/FAT partition — rejected, NVS is the
purpose-built mechanism for small key-value config data and is simpler to wear-level correctly.

## Decision: Re-entering setup via a dedicated captive-portal "edit" mode, not a full factory reset

**Rationale**: FR-012 requires changing one piece of configuration (e.g. WiFi) without erasing
others (e.g. Home Assistant connection). The portal's steps are each independently re-enterable:
a persisted `SetupState` (see data-model.md) tracks which of the three steps are currently valid,
and the portal always shows all three steps with already-configured ones pre-filled/summarized
rather than blank. A physical trigger (documented as a design detail: e.g. holding the board's
user button during boot) forces AP mode even when a working WiFi connection exists, so the user
can always get back into the portal.

**Alternatives considered**: A single "erase everything" reset only — rejected, directly
contradicts FR-012 and Edge Cases ("without erasing the already-configured Home Assistant
connection and dashboard choice").

## Decision: Ethernet-present skips straight to Home Assistant step (per spec Assumption)

**Rationale**: Already documented as an assumption in spec.md; implementation just needs to
check link-up state on the onboard Ethernet PHY at boot before deciding whether to start the AP.
`esp-idf-svc` exposes Ethernet driver bindings for this check.

**Alternatives considered**: Always requiring WiFi setup even when Ethernet is wired — rejected,
unnecessary friction the spec's assumption already ruled out (open to revisiting if the user
disagrees, per the assumption's own caveat).
