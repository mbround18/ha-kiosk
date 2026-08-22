# Quickstart: Validating WiFi & Home Assistant Provisioning

## Prerequisites

- ESP32-P4-Function-EV-Board flashed with this feature's firmware (see `firmware/` build/flash
  steps already established: `cargo build` / `espflash flash --monitor` with the pinned
  `nightly-2026-07-01` toolchain, `LIBCLANG_PATH` and `IDF_PYTHON_ENV_PATH` shims in place).
- A phone or laptop able to join a WiFi network and open a browser.
- A live Home Assistant instance reachable from the home network, with a long-lived access token
  generated ahead of time (Settings → your profile → Security tab → long-lived access tokens),
  and at least one dashboard configured (for the non-empty-list path).
- A serial connection to the device (`/dev/ttyACM0` per prior session) for observing boot/log
  state per Constitution Principle V.

## Scenario 1 — First-time setup end to end (validates User Stories 1–3, SC-001)

1. Erase NVS / flash a fresh build so no `WifiCredentials` exists.
2. Power on. Confirm over serial log that the device enters AP mode and starts the captive portal
   (no Ethernet connected for this run).
3. On a phone, connect to the device's broadcast network; confirm the OS auto-opens the setup
   page (or manually browse to `http://192.168.4.1/` if auto-detection doesn't trigger on that
   OS — note as a finding, not a blocker).
4. Select the home WiFi network from the list, enter its password, submit. Confirm success is
   reported and the device's own AP disappears (FR-005, FR-006).
5. Reconnect the phone to the home network. Confirm the setup page (now served from the device's
   new IP on the home network, or continued from the same browser tab if it followed) prompts for
   Home Assistant details.
6. Enter the Home Assistant base URL and access token. Confirm the instance name is shown back
   (FR-008).
7. Confirm the dashboard list appears with real dashboard names (FR-009); select one.
8. Confirm `GET /api/status` (or the equivalent final page state) shows all three steps
   configured, and time the whole flow — target under 10 minutes (SC-001).

## Scenario 2 — Wrong WiFi password (validates FR-005, SC-003)

1. From a fresh/AP-mode state, submit an intentionally wrong password for a real nearby network.
2. Confirm the response distinguishes `wrong_password` from other failure reasons, and the portal
   lets the user retry just the WiFi step without restarting.

## Scenario 3 — Wrong Home Assistant token (validates FR-008, SC-003)

1. With WiFi already configured, submit a valid `base_url` but an invalid token.
2. Confirm the response is `unauthorized`, not `unreachable`, and the portal lets the user retry
   just this step.

## Scenario 4 — Change dashboard only (validates FR-012, SC-004)

1. From a fully-configured device, trigger re-entering setup (documented physical trigger).
2. Confirm WiFi and Home Assistant steps show as already-configured (not blank).
3. Pick a different dashboard. Confirm only `DashboardSelection` changed — WifiCredentials and
   HomeAssistantConnection persisted values are untouched. Time this sub-flow — target under 2
   minutes (SC-004).

## Scenario 5 — Saved WiFi network unreachable at boot (validates FR-012 edge case, SC-005)

1. From a fully-configured device, change the home router's password (or take it offline) so the
   saved credentials no longer work.
2. Power-cycle the device. Confirm it falls back to AP mode automatically, while
   `HomeAssistantConnection` and `DashboardSelection` remain intact in NVS.
3. Reconnect via the portal with corrected WiFi credentials only. Confirm the device returns to
   fully working state without re-entering Home Assistant details. Time this — target under 10
   minutes (SC-005).

## Scenario 6 — Empty dashboard list (validates Edge Case)

1. Point setup at a Home Assistant instance with zero configured dashboards.
2. Confirm `GET /api/dashboards` returns `{"dashboards": []}` and the setup page shows the
   "no dashboards yet" guidance rather than a silent empty list.
