---

description: "Task list for WiFi & Home Assistant Self-Service Provisioning"
---

# Tasks: WiFi & Home Assistant Self-Service Provisioning

**Input**: Design documents from `/specs/001-wifi-ha-provisioning/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/captive-portal-api.md, quickstart.md

**Tests**: Not explicitly requested in the spec; no dedicated test-first phase per story. A
lightweight host-testable unit-test task is included in Polish since plan.md's Technical Context
calls out `cargo test` for pure logic (JSON shapes, failure classification). On-device behavior
is verified via the Quickstart scenarios (serial-log evidence), not automated tests.

**Organization**: Tasks are grouped by user story (spec.md priorities P1/P1/P2) to enable
independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3)
- Paths are relative to repository root; firmware lives under `firmware/`

## Phase 1: Setup

**Purpose**: Project structure and dependencies this feature needs, per plan.md's Project
Structure section.

- [X] T001 Create the module skeleton `firmware/src/net/mod.rs`, `firmware/src/ha/mod.rs`,
      `firmware/src/storage/mod.rs`, `firmware/src/portal_api/mod.rs` (empty modules wired into
      `firmware/src/main.rs`) and the `firmware/assets/portal/` directory for static setup-page
      assets
- [X] T002 [P] Add `serde` and `serde_json` to `firmware/Cargo.toml` dependencies for the portal
      JSON API in contracts/captive-portal-api.md
- [X] T003 [P] Evaluate and add a WebSocket client dependency to `firmware/Cargo.toml` for the
      one-shot `lovelace/dashboards/list` call (research.md: check `esp-idf-svc`'s own WS support
      first before adding a new crate)
- [X] T004 [P] Create a minimal static setup-page skeleton (`index.html`, one JS/CSS file) in
      `firmware/assets/portal/`, embedded into the firmware binary via `include_bytes!` from
      `firmware/src/net/ap_portal.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared infrastructure every user story's tasks build on — storage, boot-time
routing decision, and the HTTP server skeleton.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [X] T005 Implement NVS read/write/clear helpers for the `kiosk_cfg` namespace and the three
      entities in data-model.md (`WifiCredentials`, `HomeAssistantConnection`,
      `DashboardSelection`) in `firmware/src/storage/mod.rs`
- [X] T006 Implement `SetupState` computation (`wifi_configured`, `ha_configured`,
      `dashboard_selected` from NVS presence; `ethernet_link_up` sampled live) in
      `firmware/src/storage/mod.rs` (depends on T005)
- [X] T007 [P] Implement Ethernet link-up check against the onboard PHY in
      `firmware/src/net/ethernet.rs`
- [X] T008 Implement the boot-time decision logic (AP/portal mode vs. skip straight to STA/HA
      per the Ethernet-present assumption in spec.md) in `firmware/src/main.rs` (depends on T006,
      T007)
- [X] T009 [P] Implement the JSON request/response types for every endpoint in
      contracts/captive-portal-api.md in `firmware/src/portal_api/mod.rs`
- [X] T010 Implement AP mode bring-up, DNS-hijack server (resolve every hostname to the device's
      own AP IP), and an `esp_http_server` instance with routing wired to (stubbed) handlers, in
      `firmware/src/net/ap_portal.rs` (depends on T008, T009)
- [X] T011 Implement the captive-portal OS-detection endpoints (`/generate_204`,
      `/hotspot-detect.html`, `/connecttest.txt`/`/ncsi.txt` per contracts/captive-portal-api.md)
      plus a catch-all redirect to `/`, in `firmware/src/net/ap_portal.rs` (depends on T010)

**Checkpoint**: Foundation ready — user story implementation can now begin.

---

## Phase 3: User Story 1 - First-time network setup (Priority: P1) 🎯 MVP

**Goal**: A user with no configured network can join their home WiFi through the device's own
captive portal.

**Independent Test**: Power on a factory-reset device, connect to its broadcast network,
complete the WiFi step in the portal, and confirm the device joins the home network and its own
AP disappears (Quickstart Scenario 1 steps 2-4, Scenario 2).

### Implementation for User Story 1

- [X] T012 [US1] Implement WiFi network scanning (backing `GET /api/networks`) in
      `firmware/src/net/wifi_sta.rs`
- [X] T013 [US1] Implement WiFi join-and-verify with failure-reason classification
      (`wrong_password` / `network_not_found` / `other` per contracts/captive-portal-api.md) in
      `firmware/src/net/wifi_sta.rs` (depends on T012, same file)
- [X] T014 [US1] Wire the `POST /api/wifi` handler in `firmware/src/portal_api/mod.rs`,
      persisting `WifiCredentials` via T005's storage helpers only on a successful join (depends
      on T013, T005)
- [X] T015 [US1] Wire the `GET /api/networks` handler in `firmware/src/portal_api/mod.rs`
      (depends on T012)
- [X] T016 [US1] Implement the AP-teardown-on-success transition (stop the AP/portal, switch to
      STA-only) in `firmware/src/net/ap_portal.rs` (depends on T014)
- [X] T017 [P] [US1] Build the WiFi-picker step of the setup page (network list, password entry,
      plain-language error display per failure reason) in `firmware/assets/portal/`
- [X] T018 [US1] Populate `wifi_configured` in the `GET /api/status` handler in
      `firmware/src/portal_api/mod.rs` (depends on T006)

**Checkpoint**: User Story 1 independently functional — a device can be taken from unconfigured
to joined-to-home-WiFi entirely through the portal.

---

## Phase 4: User Story 2 - Connecting to Home Assistant (Priority: P1)

**Goal**: Once on a network, the user provides Home Assistant details and the device verifies it
can actually reach that instance before proceeding.

**Independent Test**: With a device already on a network, run only this step: submit a Home
Assistant URL and token, and confirm accurate success/failure reporting (Quickstart Scenario 1
steps 5-6, Scenario 3).

### Implementation for User Story 2

- [X] T019 [P] [US2] Implement the REST verification call (`GET {base_url}/api/config` with
      bearer token), classifying `unreachable` vs. `unauthorized` per
      contracts/captive-portal-api.md, in `firmware/src/ha/rest.rs`
- [X] T020 [US2] Wire the `POST /api/home-assistant` handler in `firmware/src/portal_api/mod.rs`,
      persisting `HomeAssistantConnection` (including `instance_name` from the verification
      response) only on success (depends on T019, T005)
- [X] T021 [P] [US2] Build the Home Assistant step of the setup page (URL + token fields,
      plain-language guidance on where to find the token, error display distinguishing
      unreachable vs. unauthorized) in `firmware/assets/portal/`
- [X] T022 [US2] Populate `ha_configured` and `instance_name` in the `GET /api/status` handler in
      `firmware/src/portal_api/mod.rs` (depends on T020)

**Checkpoint**: User Stories 1 and 2 together deliver a device that can reach the user's actual
Home Assistant instance — the hard-blocking half of setup is complete.

---

## Phase 5: User Story 3 - Choosing what to display (Priority: P2)

**Goal**: The user picks which Home Assistant dashboard the kiosk shows, from a real list of
their dashboards by name.

**Independent Test**: With a device already connected to a live Home Assistant instance, fetch
the dashboard list, select one, and confirm the device records that exact choice (Quickstart
Scenario 1 steps 7-8, Scenario 6).

### Implementation for User Story 3

- [X] T023 [US3] Implement the one-shot authenticated WebSocket call to
      `lovelace/dashboards/list` in `firmware/src/ha/dashboards.rs` (depends on T020's persisted
      connection details being readable via T005)
- [X] T024 [US3] Wire the `GET /api/dashboards` handler, including the empty-list case per
      contracts/captive-portal-api.md, in `firmware/src/portal_api/mod.rs` (depends on T023)
- [X] T025 [US3] Wire the `POST /api/dashboard` handler, persisting `DashboardSelection`, in
      `firmware/src/portal_api/mod.rs` (depends on T005)
- [X] T026 [P] [US3] Build the dashboard-picker step of the setup page (list by name, "no
      dashboards yet" guidance for the empty case) in `firmware/assets/portal/`
- [X] T027 [US3] Populate `dashboard_selected` in the `GET /api/status` handler in
      `firmware/src/portal_api/mod.rs` (depends on T025)

**Checkpoint**: All three user stories complete — full first-time setup flow (Quickstart Scenario
1) is deliverable end to end.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Requirements that span stories: independent re-entry into setup (FR-012), the
credential-handling guarantee (Constitution Principle IV), and validation evidence (Constitution
Principle V).

- [ ] T028 Implement the physical re-enter-setup trigger (documented boot-time button hold)
      that forces AP/portal mode while retaining already-persisted `HomeAssistantConnection` and
      `DashboardSelection` entities, in `firmware/src/main.rs` (depends on T008; exercised by
      Quickstart Scenarios 4 and 5)
- [X] T029 [P] Audit every log/print call added across `firmware/src/net/`, `firmware/src/ha/`,
      and `firmware/src/portal_api/` to confirm no WiFi password or Home Assistant access token
      value is ever logged (Constitution Principle IV, FR-014)
- [ ] T030 [P] Add `cargo test`-runnable unit tests for the portal API's JSON (de)serialization
      (T009's types) and the WiFi/Home Assistant failure-reason classification (T013, T019) in
      `firmware/src/portal_api/mod.rs` and `firmware/src/net/wifi_sta.rs`
- [ ] T031 Execute Quickstart scenarios 1-6 against the physical ESP32-P4-Function-EV-Board and
      record serial-log evidence for each, per Constitution Principle V

---

## Dependencies & Execution Order

- **Phase 1 (Setup)** → **Phase 2 (Foundational)**: strictly sequential; Phase 2 tasks depend on
  Phase 1's module skeleton and dependencies existing.
- **Phase 2 → Phases 3/4/5 (User Stories)**: Phase 2 must fully complete first (all three
  stories' handlers live in the shared `firmware/src/portal_api/mod.rs` routing built in T010).
- **Phase 3 (US1) and Phase 4 (US2)**: both P1, and largely independent of each other's *logic*
  (WiFi joining vs. HA verification touch different files), but Phase 4's `POST
  /api/home-assistant` step is only reachable in the real user flow after Phase 3's WiFi step
  succeeds (or Ethernet is already up) — implement and test Phase 3 first even though the code
  itself doesn't import from Phase 4.
- **Phase 5 (US3)** depends on Phase 4: T023 needs a persisted `HomeAssistantConnection` to call
  the WebSocket API with.
- **Phase 6 (Polish)**: T028 depends on T008; T029-T031 depend on all of Phases 3-5 being
  implemented.

## Parallel Execution Examples

- Within Phase 1: T002, T003, T004 can run together (different files/concerns).
- Within Phase 2: T007 and T009 can run in parallel with each other (different files); T005→T006
  and T008/T010/T011 are sequential chains.
- Within Phase 3: T017 (setup-page UI) can be built in parallel with T012-T016 (device-side
  logic) since they're different files, converging only at manual/Quickstart testing.
- Within Phase 4: T019 (REST client) and T021 (UI) can run in parallel.
- Within Phase 5: T023 (WebSocket client) and T026 (UI) can run in parallel.
- Phase 6: T029 and T030 can run in parallel with each other.

## Implementation Strategy

**MVP = Phase 1 + Phase 2 + Phase 3 (User Story 1 only)**: delivers a device that can be taken
from unconfigured to "joined the home WiFi network," independently demonstrable even before Home
Assistant or dashboard selection exist. This matches spec.md's own P1 prioritization and
Constitution Principle V's "network bring-up before anything else" ordering.

**Incremental delivery after MVP**: Phase 4 (US2) next — still P1, and the second hard-blocking
half of setup. Phase 5 (US3) after that — P2, personalizes rather than unblocks. Phase 6 last,
since its tasks (re-entry, credential audit, tests, hardware validation) cut across all three
stories and are most meaningful once all three exist.

---

## Implementation Status Notes (2026-08-22)

**Verified on real hardware** (ESP32-P4-Function-EV-Board, via `curl` over a direct WiFi
connection to the device's own captive portal AP from a host machine): T001-T015, T018 — module
scaffolding, NVS storage, boot decision, AP+DNS-hijack+HTTP server bring-up, captive-portal OS
detection redirects, WiFi scan (`GET /api/networks` returned real nearby SSIDs), WiFi join
attempts including both failure paths (`wrong_password`, `network_not_found` - both correctly
classified without crashing the device), and `GET /api/status`.

**Implemented and compiling, not yet live-verified**: T016, T017 (AP teardown-on-success and the
setup page's WiFi step — no real WiFi credentials were available in this environment to complete
a *successful* join), T019-T027 (Home Assistant REST verification and WebSocket dashboard
listing — no live Home Assistant instance was reachable from this environment), T029 (credential
logging audit — checked via `grep` for password/token values reaching any log macro; none found,
but this hasn't been exercised against a live successful HA connection to confirm at runtime).

**Not yet implemented**: T028 (physical re-enter-setup trigger), T030 (unit tests), and the
remaining T031 Quickstart scenarios that require real WiFi credentials and a real Home Assistant
instance (Scenarios 1 full completion, 3, 4, 5, 6).

**Three real bugs were found and fixed during hardware validation** (all now in
`firmware/sdkconfig.defaults` / `firmware/src/`, and documented in project memory):
1. This board's ESP32-P4 chip revision (v1.3) needs explicit sdkconfig flags or every boot
   crashes before any app code runs (esp-rs/esp-idf-sys#376).
2. `esp_hosted`'s default 100Hz FreeRTOS tick causes SDIO RPC jitter/timeouts under any real
   load (e.g. serving an HTTP request) - needs `CONFIG_FREERTOS_HZ=1000`.
3. `BlockingWifi::wait_netif_up()` blocks on `is_connected()`, which never becomes true in an
   AP+STA Mixed configuration where STA is only enabled for scanning (not connected) - blocked
   for its full 15s timeout and crashed the app. Fixed with a custom AP-netif-only wait.
4. `wifi_sta::join_network` was replacing the whole WiFi configuration with `Client`-only during
   a join *attempt* (success or failure), immediately tearing down the captive portal AP before
   the result was even known. Fixed to use `Configuration::Mixed`, preserving the AP.

**Known physical limitation (not a bug)**: this is a single-radio chip. When the STA side
attempts to join a network on a different channel than the AP is broadcasting on, the radio must
briefly leave the AP's channel, which drops any client currently connected to the AP (like a
`curl` session) for the duration of the attempt. A real captive-portal client (a phone's browser)
tolerates and retries through this; a single blocking `curl` call does not. This is inherent to
concurrent AP+STA on one radio, not something this codebase can fix.

## T028 & T030 completion notes (2026-08-22)

**T028 (re-enter-setup trigger)**: Implemented using GPIO0 (the standard Espressif "BOOT"
button, present on this board) read via the raw ESP-IDF C GPIO API at boot
(`firmware/src/main.rs`), rather than the Rust HAL's `PinDriver` - the latter needs to take
ownership of the `gpio0` field out of `Peripherals`, which conflicts with `net::ap_portal::run`
needing the whole `Peripherals` struct afterward. While implementing this, also fixed a real bug
found in the AP-teardown path: it was resetting to a blank `ClientConfiguration::default()`
instead of preserving the actual STA config from the successful join, and the exit condition
would have immediately torn down a button-triggered re-entry (since WiFi is already configured
in that case) instead of keeping the portal open. Compiles; not live-tested (no physical access
to hold the button during a scripted test in this environment - a human needs to verify holding
BOOT at power-on actually re-opens the portal).

**T030 (unit tests)**: Added `#[cfg(test)] mod tests` to `firmware/src/portal_api/mod.rs`
covering JSON (de)serialization shapes against contracts/captive-portal-api.md and failure-reason
string mappings (11 tests). These compile cleanly under `cargo check --tests`. However,
**`cargo test` does not actually execute them on this project**: the `[[bin]]` has
`harness = false` (set by the esp-idf-template specifically "to resolve rust-analyzer errors"),
which means Cargo does not generate the standard test-discovery `fn main` for the test build -
running `cargo test --bin firmware` just reflashes the ordinary production `main()` unchanged
(confirmed via serial log: normal portal boot sequence, no test output). Making `cargo test`
functionally run these tests would require either flipping `harness = true` (risking the
rust-analyzer issue the template's author was avoiding) or a separate test harness/framework
(e.g. `embedded-test`) - both out of scope for this task without an explicit decision from the
user. The tests remain valuable as compiler-checked, precisely-specified documentation of the
JSON contract even though they aren't currently wired into an executable `cargo test` run.
