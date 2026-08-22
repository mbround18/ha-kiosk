<!--
Sync Impact Report
Version change: (none) → 1.0.0
Modified principles: n/a (initial ratification)
Added sections: Core Principles (I-VI), Hardware & Toolchain Constraints, Development Workflow, Governance
Removed sections: none
Follow-up TODOs: TODO(RATIFICATION_DATE) — no prior formal adoption date exists; set to today's date as the
  effective ratification date since this is the first constitution for the project.
-->

# ha-kiosk Constitution

## Core Principles

### I. std Rust on ESP-IDF (NON-NEGOTIABLE)
Firmware MUST be written as std Rust targeting `esp32p4` via `esp-idf-svc`/`esp-idf-hal`
(ESP-IDF), not bare-metal `no_std` (`esp-hal`/`esp-wifi`). The target board's ESP32-P4 MCU has
no native radio; WiFi and BLE are only reachable through the onboard ESP32-C6 co-processor over
ESP-Hosted (SDIO), and Ethernet, MIPI-DSI display, and capacitive touch drivers are only mature
in ESP-IDF today. Bare-metal support for these peripherals on P4 is not production-ready as of
this constitution's ratification; re-evaluate only via a formal amendment, not ad hoc.

### II. Self-Service Provisioning, No Companion App
The device MUST be configurable standalone by an end user with no separate mobile app, no
build-time hardcoded credentials, and no cloud-hosted setup service. Provisioning MUST work by:
the device starting its own WiFi access point, the user connecting to it, a captive-portal HTTP
page served locally guiding the user to (a) select/enter their home network credentials, then (b)
enter their Home Assistant URL and long-lived access token, then (c) choose which dashboard/view
to display. Each step MUST be resumable/re-enterable without a factory reset if it fails.

### III. Local-Only Communication
The device MUST NOT depend on any cloud service, either Anthropic's, Espressif's, or a
third-party's, for provisioning or runtime operation. All traffic — captive portal, Home
Assistant REST/WebSocket calls — MUST stay on the local network the device is provisioned onto.
Home Assistant's own external/cloud remote-access features are the user's choice and out of this
firmware's scope.

### IV. Credentials Handled Deliberately
WiFi credentials and the Home Assistant long-lived access token are secrets. They MUST be
persisted only in non-volatile storage on-device (e.g. NVS), MUST NOT be logged to serial output
or any diagnostic surface, and MUST be clearable via an explicit user-triggered reset path (e.g. a
button-hold or a portal "forget network" action). Firmware MUST NOT transmit these credentials
anywhere except the WiFi association handshake and direct calls to the user's own configured Home
Assistant instance.

### V. Incremental, Serial-Verified Milestones
Features MUST be built and verified in dependency order, each confirmed working over serial log
output before the next is layered on: (1) WiFi/Ethernet network bring-up, (2) Home Assistant API
connectivity, (3) captive-portal provisioning flow, (4) display/touch bring-up, (5) dashboard
rendering. Display/UI work MUST NOT begin until the network and Home Assistant API milestones are
independently verified, since UI bugs are far harder to diagnose than network bugs without a
screen to trust yet.

### VI. Reproducible Build Environment
Toolchain and SDK versions (Rust nightly channel, ESP-IDF version, `espflash`/`ldproxy`
versions) MUST be pinned in version-controlled files (`rust-toolchain.toml`, `Cargo.toml`/
`Cargo.lock`, `sdkconfig.defaults`) rather than left floating, since bleeding-edge nightly
toolchains have already been observed to break this project's build via std/`libc` mismatches.
Any change to a pinned toolchain version MUST be a deliberate, tested commit, not an incidental
side effect of a fresh environment picking up "latest".

## Hardware & Toolchain Constraints

Target hardware is the official Espressif ESP32-P4-Function-EV-Board: ESP32-P4 MCU (RISC-V,
16MB flash, ~32MB PSRAM), onboard ESP32-C6 co-processor (WiFi 6 + BLE via SDIO/ESP-Hosted),
onboard Ethernet PHY + RJ45 jack, MIPI-DSI display connector with capacitive touch. Firmware
build tooling (host-side Python for ESP-IDF, `libclang` for `bindgen`) MAY be provided via `uv`
rather than system package managers where that avoids a sudo/apt dependency, provided the
resulting environment is reproducible from a documented setup step, not manual ad hoc state.

## Development Workflow

Each milestone in Principle V MUST have its own spec (`/speckit-specify`) and plan
(`/speckit-plan`) before implementation begins. A milestone is not "done" until its serial-log
(or, for later milestones, on-screen) evidence has been captured and reviewed, not merely until
the code compiles and flashes.

## Governance

This constitution supersedes ad hoc technical preferences for this repository. Amendments
require: (1) a proposed diff to this file, (2) an explicit rationale for the change, (3) a
version bump per semantic versioning (MAJOR for incompatible principle removal/redefinition,
MINOR for new/materially-expanded principles, PATCH for clarifications), and (4) an updated Sync
Impact Report comment at the top of this file. All specs and plans produced by spec-kit commands
in this repository MUST be checked against these principles before implementation starts; any
deviation MUST be justified in the plan's Complexity Tracking section or rejected.

**Version**: 1.0.0 | **Ratified**: TODO(RATIFICATION_DATE): no prior formal adoption date exists; using this document's creation date 2026-08-22 as the effective date | **Last Amended**: 2026-08-22
