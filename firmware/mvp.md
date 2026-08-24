# ha-kiosk MVP status

Project-level docs (customer-facing README, CONTRIBUTING, compatibility matrix, the `flasher`
host tool) now live at the repo root - see [`../README.md`](../README.md) and
[`../CONTRIBUTING.md`](../CONTRIBUTING.md). This file stays focused on firmware feature status.

**Tests**: hardware-independent logic (JSON contract types, weather/climate formatting, room-grid
layout math) now lives in `../firmware-core/` and is covered by 27 unit tests, runnable normally
via `cargo test --workspace` from the repo root - see CONTRIBUTING.md's "Testing" section for why
that split exists (`firmware`'s own `[[bin]]` can't actually run `#[test]`s at all).

## Working
- Current weather + whole-home average temperature + average humidity shown above the room list (fetched via HA `render_template`, refreshed every 15s along with rooms)
  - Average temp/humidity excludes non-ambient readings: values outside 32-120°F (0-49°C) or 0-100% are dropped, and sensors named like an appliance (oven, grill, stove, smoker, dryer, washer, fridge, freezer, dishwasher, kettle, toaster, microwave) are excluded by name too
- Room list is a reactive 2-column grid of room name + iOS-style On/Off toggle (reflecting each room's actual current light state, not just an action button) - row size (and text size) shrinks automatically if there are enough rooms that the large size wouldn't fit them all, so the goal is "show every room" over "always biggest text"
- Button-mash guard: tapping a room's toggle flips it immediately (optimistic) and shows a desaturated/muted color for a 2s cooldown during which further taps on that room are dropped (not queued) - a full HA WebSocket connect+auth+call happens at most once per room per cooldown window no matter how fast a kid mashes it. If the HA call fails/times out, the toggle no longer guesses a revert (a timed-out call may have actually succeeded server-side - guessing "opposite" caused a real observed state-mismatch bug after a long run) - instead it forces an immediate authoritative refresh and stays muted until that resolves it.
- Rooms whose lights are all currently `unavailable` in HA (e.g. a physical power switch cut) are grayed out with a distinct flat/centered-knob toggle and excluded from tap targets entirely, instead of showing a potentially-fictional on/off state.
- Header (weather line) and footer (climate/AC line) text auto-shrink to fit the screen width instead of running off the edge - both are built from live HA data (condition names, mode breakdowns) whose length isn't under our control, so a fixed text scale would eventually overflow.
- Auto-dim: backlight drops to 15% after 60s with no touch, restores to 100% instantly on any tap anywhere on screen (not just a toggle)
- HA calls (`light.turn_on`/`turn_off`) now run on a dedicated background worker thread instead of inline on the UI loop, which used to freeze touch polling/dimming for the whole WebSocket connect+auth+call round trip (1-2s+) on every tap. Touch poll cadence also dropped from 150ms to 50ms now that it's no longer competing with network I/O. Toggles confirm (un-mute) as soon as the real result comes back rather than always waiting out the full 2s cooldown.
- All on-screen text renders at 2x size (crisp pixel-doubling of the largest built-in font) when room count allows it, for readability in the dark
- Climate/AC summary in the bottom-right corner: whole-home average of every `climate.*` entity's sensed (current, not target) temperature, plus a compact HVAC-mode breakdown (e.g. "AC 74F | 2xCool 1xOff" - a count prefix only appears for a mode with more than one unit). Fetched in the same 15s refresh/WS session as rooms/weather; best-effort like weather (a home with no climate entities just shows nothing there).
- Color theme applied, dark-mode variant (text/background swapped from the originally given light palette per request): background `#0F172A` (dark slate), text `#F3F4F6` (light gray), toggle-on `#4F46E5` (primary), toggle knob `#FFFFFF` (secondary), "Rooms" heading `#F59E0B` (accent). The toggle's inactive/off-track color isn't part of the given palette and was derived as a neutral gray (Tailwind gray-300/200).
- WiFi captive-portal setup flow (scan, join, show/hide password toggle)
- Home Assistant connection setup (URL + long-lived token, show/hide password toggle)
- TLS trust: cert bundle -> stored pinned CA -> bundled CA -> ISRG X1 -> TOFU on-screen "tap to trust" fallback
- `/api/debug` HTTP log endpoint, available during setup AND kiosk mode
- Reboot from captive portal into kiosk mode once setup is complete
- Kiosk mode: fetches HA areas with lights, renders per-room On/Off buttons on the LCD
- Touch input (GT911) with debounce, tapping On/Off actually calls `light.turn_on`/`light.turn_off` via HA WebSocket and updates the room
- Room/weather/humidity list auto-refreshes every 15 seconds

## In progress
- **Click sound on button press (audio feedback)**: codec (ES8311) opens successfully, I2S write returns no error, but no audible sound on the device yet. Added `esp_codec_dev_set_out_mute(codec, false)` after suspecting the speaker-amp enable GPIO wasn't being toggled — flashed, confirmed device boots cleanly, but still no sound. Root cause not yet found; needs further investigation (volume level, PA GPIO wiring, I2S clock/format mismatch, or codec output routing).
- **Long-run stability (touch unresponsive + light-state mismatch after ~1hr)**: user report - after ~1hr uptime, touch stopped registering, and a room showed off on-device while genuinely on in HA. Leading theory: every 15s refresh (and every tap) opens a brand-new WebSocket+TLS connection, and the device consistently logs `Connection terminated while waiting for clean TCP close` on teardown - a plausible source of a slow internal-SRAM leak (only ~225KB free right after boot, vs. ~29MB total once PSRAM is counted - a leak invisible in the total figure could still exhaust the internal pool within an hour at ~4 cycles/min). A second, previously-flagged risk: the GT911 touch controller (new `i2c_master` driver) shares its I2C bus with the ES8311 audio codec (legacy I2C driver, `CONFIG_I2C_SKIP_LEGACY_CONFLICT_CHECK=y`) - untested over long runs until now. Shipped so far (not yet confirmed as the fix, since reproducing takes ~1hr): `Display::poll_tap` now logs on transitions into/out of a read-failure state instead of silently returning `None` either way; every refresh cycle logs both total and internal free heap to `/api/debug`, to watch for a downward trend; the state-mismatch half is separately addressed by no longer guessing a revert on a failed/timed-out `set_room_lights` call (see above). Next step: another long run, then check `/api/debug` for a `touch: ... started failing` line and/or a shrinking internal-heap trend.

## Not started
- "Select area" option (nice-to-have, dashboard selection became unnecessary once kiosk mode went room-based)
- Dimming/color control (explicit stretch goal, deferred)
- **Camera-based undim on motion (stretch goal, currently out of scope)**: the vendored BSP's header declares `bsp_camera_start()`, but the implementation `.c` file was never copied into `vendor/esp32_p4_function_ev_board/` (only display/touch/audio are) — confirmed via a build attempt that failed at link time with `undefined reference to bsp_camera_start`. Would need: (1) porting the real BSP camera source + its `esp_video`/`esp_cam_sensor` component deps from upstream, (2) driving ESP-IDF's V4L2-style capture API to actually read frames, (3) a frame-differencing motion-detection loop feeding the existing auto-dim/undim logic in `net::kiosk`. Also still unconfirmed which sensor (OV5647 vs SC2336) is physically attached.
