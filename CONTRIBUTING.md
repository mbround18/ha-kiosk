# Contributing

Development notes for working on this repo. If you're just looking to use a device, see
[README.md](README.md) instead.

## Repo layout

```
firmware/     ESP32-P4 firmware (Rust, esp-idf-svc/std) + portal-ui/ (the setup page, React+Vite)
flasher/      Host-side CLI: build+flash+auto-provision (see README.md's "flasher" section)
tools/        Local dev tooling not needed to build/flash (e.g. debug-logging/'s syslog-ng compose)
specs/        spec-kit feature specs/plans for this project's formal requirements process
Cargo.toml    Workspace root for host-side tools (flasher) - see the note below on firmware
```

**`firmware` is intentionally not a workspace member.** It's an ESP-IDF (embedded,
`riscv32imafc-esp-espidf` target) crate pinned to its own toolchain via
`firmware/rust-toolchain.toml`, with a heavy native (CMake/ESP-IDF) build process and its own
`Cargo.lock`. Folding it into the root workspace would force a single unified lockfile/toolchain
across a host-target CLI tool and an embedded target - real risk of dependency-resolution or
toolchain drift for very little benefit. Build it independently, as documented below (or via
`flasher`, which just shells out to the same commands).

## Prerequisites

- [`espflash`](https://github.com/esp-rs/espflash) on your `PATH` (`cargo install espflash`)
- The Rust toolchain pinned in `firmware/rust-toolchain.toml` (rustup will fetch it automatically
  the first time you build inside `firmware/`, given `rustup`'s default auto-install behavior)
- `pnpm` (for `portal-ui`; `firmware/build.rs` runs `pnpm install`/`pnpm build` automatically as
  part of `cargo build`, so you don't need to build it separately)
- A real `libclang` shared library for `bindgen` (used by `esp-idf-sys`) - if your system doesn't
  already have one on its default search path, `flasher flash` will look for the one bundled by
  the Python `clang` package under `firmware/.tools-env/.venv` and set `LIBCLANG_PATH`
  automatically; building manually, you may need to set it yourself

## Building and flashing

The easiest path is the `flasher` tool (see README.md), which wraps everything below:

```sh
cargo run -p flasher -- flash
```

Manually, from `firmware/`:

```sh
cd firmware
cargo build --release
espflash flash --bootloader target/riscv32imafc-esp-espidf/release/bootloader.bin \
  target/riscv32imafc-esp-espidf/release/firmware --port /dev/ttyACM0
```

A `Makefile` also exists in `firmware/` with `make build`/`make flash`/`make monitor` targets from
earlier in this project's development - it hardcodes a machine-specific `LIBCLANG_PATH` default,
so adjust that if you use it.

**Always flash with `--bootloader ... <elf>` together** (not just the app image alone) - this
board's chip revision requires bootloader-level sdkconfig options
(`CONFIG_ESP32P4_SELECTS_REV_LESS_V3`/`CONFIG_ESP32P4_REV_MIN_100`, already set in
`firmware/sdkconfig.defaults`) that only take effect if the bootloader itself is reflashed too.

## Debugging on-device

The serial monitor is unreliable on this board (attaching resets it, and captured logs can
silently truncate). Instead, the firmware keeps an in-memory ring buffer of every log line
(`firmware/src/debug_log.rs`) exposed over HTTP - reachable during setup mode *and* kiosk mode:

```sh
curl http://<device-ip>/api/debug
```

`tools/debug-logging/` has an optional `syslog-ng` Docker Compose setup if you want to forward
logs somewhere more persistent instead.

## Auto-provisioning during development

Repeatedly re-flashing and manually re-typing WiFi/Home Assistant credentials on a touchscreen
gets old fast. Copy `.env.example` to `.env`, fill in your details, and:

```sh
cargo run -p flasher -- flash --configure
```

flashes the device, then (once your computer is joined to the device's own setup WiFi network)
replays the exact same requests the setup web page makes to fully provision it - see
`specs/001-wifi-ha-provisioning/contracts/captive-portal-api.md` for the API contract this
implements, and `flasher/src/configure.rs` for the implementation.

## Project status

`firmware/mvp.md` tracks what's currently working, in progress, and explicitly out of scope -
check there before assuming something is or isn't implemented. `specs/001-wifi-ha-provisioning/`
has the original formal spec/plan/tasks this project's first phase was built against.

## A note on hardware quirks

This board's ESP32-P4 silicon revision (v1.3, an early/engineering-sample revision) and its
reliance on the onboard ESP32-C6 co-processor for WiFi (via ESP-Hosted over SDIO) both required
non-obvious workarounds already baked into `firmware/sdkconfig.defaults` and
`firmware/src/net/wifi_sta.rs` - if you're bringing up a fresh checkout on new hardware and hit a
boot crash or WiFi/SDIO timeout, that's the first place to look before re-deriving the fix from
scratch.
