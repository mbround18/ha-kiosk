# ha-kiosk

A dedicated Home Assistant wall panel for the Espressif ESP32-P4-Function-EV-Board — no browser,
no tablet, no third-party app. Point it at your Home Assistant instance and get a always-on,
touch-controlled light switch for every room in your home.

## What it does

- **Guided setup over WiFi.** On first boot the device broadcasts its own WiFi network with a
  simple setup page (scan/join your WiFi, connect your Home Assistant instance, done). No serial
  cable, no config files to hand-edit.
- **A native room-by-room control screen.** Once set up, the device shows every room in your home
  that has lights, each with a large on/off toggle reflecting its real current state — not a
  shrunken web page, a purpose-built screen designed to be read from across a room.
- **Live weather and home temperature/humidity averages**, refreshed automatically.
- **Automatic layout.** Text size and grid density adjust on their own so a 3-room apartment and
  a 15-room house both look right, with nothing to configure.
- **Kid-proof.** Button-mash protection, instant tap feedback, and an auto-dimming screen that
  wakes on any touch.
- **Secure by default.** Works with plain HTTP, publicly-trusted HTTPS, or a private/self-signed
  HTTPS certificate — the last case is approved with a physical tap on the device's own screen,
  never a blind "trust this certificate" checkbox.

## Getting a device set up

1. Power on the device. It broadcasts its own WiFi network (name shown on its screen) and shows a
   QR code for joining it.
2. Connect to that network with your phone or computer, and the setup page opens automatically
   (or scan the second QR code).
3. Pick your home WiFi network and enter its password.
4. Enter your Home Assistant URL and a
   [long-lived access token](https://www.home-assistant.io/docs/authentication/#long-lived-access-tokens)
   (created from your HA user profile page).
5. If this is the first time the device has seen your Home Assistant instance's certificate,
   confirm the fingerprint shown with a tap on the device's own screen.
6. The device reboots into kiosk mode, showing every room with lights, ready to use.

Prefer to skip typing on a touchscreen entirely? See [`flasher`](#flasher) below — it can
provision a device automatically from a config file.

## Compatibility

| Component | Support |
|---|---|
| **Board** | [Espressif ESP32-P4-Function-EV-Board](https://www.espressif.com/en/products/devkits) — the only board this firmware targets. Requires its onboard ESP32-C6 co-processor (ships pre-flashed from the factory; no separate setup needed) for WiFi. |
| **Display / touch** | The board's stock 7", 1024x600, MIPI-DSI panel (EK79007) and GT911 capacitive touch — required for both setup and kiosk mode. |
| **Speaker** | The board's onboard ES8311 codec + speaker, for tap click feedback. *(In progress — see [Known limitations](#known-limitations).)* |
| **Camera** | The board's MIPI-CSI camera connector is not currently supported (planned as a "wake the screen on motion" feature). |
| **Ethernet** | The board's onboard RJ45 jack is not currently supported; WiFi is the only working network path today. |
| **Home Assistant** | Home Assistant Core with its WebSocket API enabled (the default) and at least one Area with lights assigned. Reachable over plain HTTP, public HTTPS, or private/self-signed HTTPS. |
| **Setup-flow client** | Any modern phone or computer's WiFi settings + browser (Android, iOS, macOS, Windows, Linux) — the setup page uses each OS's own captive-portal detection to open automatically. |

## Everyday use

- **New rooms appear automatically.** Add a light to a new Area in Home Assistant and it shows up
  on the device within about 15 seconds — no re-provisioning.
- **Re-running setup.** Hold the board's BOOT button while powering on to force the device back
  into setup mode at any time (useful for switching WiFi networks or Home Assistant instances).
- **Screen dims after a minute of inactivity** and brightens instantly on any tap.

## Known limitations

- Tap click sound doesn't yet play audibly, despite the audio codec initializing without error —
  under investigation.
- Camera-based "wake on motion" isn't implemented — the board's camera connector needs BSP
  support this project doesn't currently vendor.
- Rooms are shown as simple on/off toggles; per-light dimming/color control isn't supported yet.

## flasher

A companion command-line tool (`flasher/`) that automates building, flashing, and (optionally)
provisioning a device without touching its touchscreen at all — handy for reflashing a known
dev/demo unit repeatedly, or setting up several identical devices.

```sh
# Build the firmware and flash it over USB (auto-detects the board's serial port)
cargo run -p flasher -- flash

# ...and also auto-provision it from .env once it reboots into setup mode
# (your computer must already be joined to the device's own setup WiFi network first)
cargo run -p flasher -- flash --configure

# Just provision an already-flashed device that's currently in setup mode
cargo run -p flasher -- configure

# List connected USB serial devices, if auto-detection picks the wrong one
cargo run -p flasher -- ports
```

`configure` reads `HOME_ASSISTANT_URL`, `HOME_ASSISTANT_TOKEN`, `SSID_NAME`, and `SSID_PASSWORD`
from a `.env` file at the repo root (copy `.env.example` to `.env` and fill it in) and replays the
same requests the setup web page would — so a device can be fully provisioned, unattended, from a
config file.

## Building it yourself / contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, build instructions, project
layout, and the hardware/firmware notes accumulated while building this.

## License

No license has been chosen for this project yet.
