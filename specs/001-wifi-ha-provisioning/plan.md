# Implementation Plan: WiFi & Home Assistant Self-Service Provisioning

**Branch**: `001-wifi-ha-provisioning` | **Date**: 2026-08-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-wifi-ha-provisioning/spec.md`

## Summary

Device ships with no network configured. On boot with no working connection, it starts its own
WiFi access point and a captive-portal HTTP server that guides the user through three
independently-completable steps: (1) join the home WiFi network (or skip if Ethernet is already
link-up), (2) enter and verify Home Assistant connection details, (3) pick a dashboard from that
Home Assistant instance to display. Each step's result is persisted to NVS only once verified,
so a failed attempt never poisons saved state, and the user can later re-enter setup to change
any one step without disturbing the others.

## Technical Context

**Language/Version**: Rust (std), nightly toolchain pinned to `nightly-2026-07-01`
(`rust-toolchain.toml`), per prior session's toolchain validation on this hardware.

**Primary Dependencies**: `esp-idf-svc` (WiFi AP/STA, `esp_http_server`, NVS, Ethernet link
status, HTTP client for the Home Assistant REST call), a lightweight WebSocket client capable of
running over `esp-idf-svc`'s TLS/TCP stack for the one-shot `lovelace/dashboards/list` call
(exact crate choice deferred to implementation — evaluate `esp-idf-svc`'s own WS support first
before adding a new dependency), `serde`/`serde_json` for the portal's JSON API.

**Storage**: ESP-IDF NVS, namespace `kiosk_cfg` (see data-model.md).

**Testing**: `cargo test` for any host-testable pure logic (e.g. JSON (de)serialization, failure
-reason classification); on-device verification via the Quickstart scenarios over serial log,
per Constitution Principle V (no on-device automated test harness in scope for this feature).

**Target Platform**: ESP32-P4-Function-EV-Board (`esp32p4`, `riscv32imafc-esp-espidf` target),
already validated buildable/flashable in this repository's `firmware/` project.

**Project Type**: Embedded firmware, single project (`firmware/`) — no separate
frontend/backend split; the captive portal's HTML/JS is served as static assets embedded in the
firmware binary.

**Performance Goals**: Not throughput-sensitive; goals are the spec's user-facing time budgets
(SC-001, SC-004, SC-005) rather than technical req/s figures.

**Constraints**: Must not depend on any cloud service (Constitution Principle III); must not log
secrets (Principle IV); portal must work from an ordinary phone browser with no app install.

**Scale/Scope**: Single device, single household network, one Home Assistant instance, one
active setup session at a time (no concurrent-setup-sessions requirement in the spec).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Check | Result |
|---|---|---|
| I. std Rust on ESP-IDF | Plan uses `esp-idf-svc` targeting `esp32p4`, no bare-metal | PASS |
| II. Self-service provisioning, no companion app | Captive portal only, no app/cloud setup service | PASS |
| III. Local-only communication | Portal traffic is device↔phone on device's own AP; HA calls go direct to the user's own instance on the home network; no third-party service involved | PASS |
| IV. Credentials handled deliberately | NVS-only persistence, never logged, cleared via re-entering setup per step, POST bodies carrying secrets are same-origin only | PASS |
| V. Incremental, serial-verified milestones | This feature is itself scoped as milestone 3 of Principle V's ordering (after network bring-up and HA API connectivity, both of which this feature's own steps 1–2 effectively *are*); Quickstart scenarios are serial-log verified | PASS — this feature spec's own steps double as milestones 1–3, display/UI (milestones 4–5) explicitly out of scope here |
| VI. Reproducible build environment | No new toolchain changes introduced by this feature; existing pinned versions carried forward | PASS |

No violations; Complexity Tracking section not needed.

## Project Structure

### Documentation (this feature)

```text
specs/001-wifi-ha-provisioning/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md         # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── captive-portal-api.md
└── tasks.md             # Phase 2 output (/speckit-tasks — not yet generated)
```

### Source Code (repository root)

```text
firmware/
├── src/
│   ├── main.rs              # boot sequence: check Ethernet link / saved WiFi -> decide AP vs STA
│   ├── net/
│   │   ├── mod.rs
│   │   ├── ap_portal.rs     # AP mode bring-up, DNS hijack, esp_http_server routes
│   │   ├── wifi_sta.rs      # STA join/verify logic, scan
│   │   └── ethernet.rs      # link-up check
│   ├── ha/
│   │   ├── mod.rs
│   │   ├── rest.rs          # GET /api/config verification call
│   │   └── dashboards.rs    # WebSocket lovelace/dashboards/list call
│   ├── storage/
│   │   └── mod.rs           # NVS read/write for WifiCredentials, HomeAssistantConnection, DashboardSelection
│   └── portal_api/
│       └── mod.rs           # JSON request/response handlers matching contracts/captive-portal-api.md
├── assets/
│   └── portal/              # static HTML/JS/CSS for the setup page, embedded via include_bytes!
├── Cargo.toml
└── rust-toolchain.toml
```

**Structure Decision**: Single embedded firmware project (already scaffolded at `firmware/` from
the `esp-idf-template`). No frontend/backend split despite the portal having a browser-facing UI
— the "frontend" is static assets embedded in and served by the same firmware binary, so Option 1
(single project) applies, organized into modules by responsibility (`net`, `ha`, `storage`,
`portal_api`) rather than by generic `models`/`services`/`cli` boilerplate, since those generic
names don't fit an embedded firmware project.

## Complexity Tracking

*No Constitution Check violations — section not needed.*
