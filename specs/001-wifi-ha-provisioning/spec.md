# Feature Specification: WiFi & Home Assistant Self-Service Provisioning

**Feature Branch**: `001-wifi-ha-provisioning`

**Created**: 2026-08-22

**Status**: Draft

**Input**: User description: "Create our own wifi network the user connects to it, we give them a wifi login portal that they then use to connect to a network and enter home assistant creds and we connect then ask the user which dashboard to display."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - First-time network setup (Priority: P1)

A user takes the kiosk device out of the box, powers it on, and it is not yet connected to any
network. Without a phone app, a computer, or any technical knowledge beyond "connect to WiFi like
normal," the user gets the device onto their home network.

**Why this priority**: Nothing else in the product works until the device is on the user's
network. This is the first thing every user does and the single point where a confusing
experience loses the user entirely.

**Independent Test**: Power on a factory-reset device, observe it broadcasting its own network,
connect a phone/laptop to that network, complete the guided steps, and confirm the device joins
the user's home network and the temporary network disappears.

**Acceptance Scenarios**:

1. **Given** a device with no network configured, **When** it powers on, **Then** it broadcasts
   its own WiFi network with a name that clearly identifies it as this product (not a generic
   default).
2. **Given** a user connected to the device's own network, **When** they open a browser, **Then**
   they are automatically shown a setup page without needing to know or type an address.
3. **Given** the setup page listing nearby networks, **When** the user selects their home network
   and enters its password, **Then** the device attempts to join it and reports success or
   failure back on the same page.
4. **Given** a wrong password was entered, **When** the connection attempt fails, **Then** the
   user is told plainly that it failed and is returned to the network selection step without
   needing to restart the whole process.
5. **Given** the device has successfully joined the home network, **When** setup continues,
   **Then** the device's own temporary network is no longer broadcast.

---

### User Story 2 - Connecting to Home Assistant (Priority: P1)

Immediately after joining the home network, the user provides what the device needs to talk to
their Home Assistant instance, and the device confirms it can actually reach it before moving on.

**Why this priority**: This is the second hard-blocking step — without it the device has no data
to show. It is equal in priority to network setup because a device that's "on the network but
can't reach Home Assistant" is just as unusable as one with no network at all.

**Independent Test**: With the device already on a network, run only this step in isolation:
enter a Home Assistant address and access credential, and confirm the device reports a successful
or failed connection accurately.

**Acceptance Scenarios**:

1. **Given** the device just joined the home network, **When** setup continues, **Then** the user
   is prompted for their Home Assistant address and an access credential, with a plain-language
   explanation of where to find the credential.
2. **Given** valid Home Assistant details, **When** the user submits them, **Then** the device
   confirms a successful connection and shows the name of the Home Assistant instance it found.
3. **Given** an incorrect address or credential, **When** the user submits them, **Then** the
   device reports the failure in plain language (e.g., "couldn't reach that address" vs. "address
   reached but credential was rejected") and lets the user correct just that field.
4. **Given** Home Assistant is temporarily unreachable (e.g., still booting on the same network),
   **When** the connection attempt fails, **Then** the user can retry without re-entering
   already-correct information.

---

### User Story 3 - Choosing what to display (Priority: P2)

Once connected to Home Assistant, the user picks which dashboard/view the kiosk should show,
rather than the device guessing or defaulting to something arbitrary.

**Why this priority**: Important for the device to be useful and trusted, but the device is
already functionally "set up" after Stories 1-2; this step personalizes rather than unblocks.

**Independent Test**: With a device already connected to a live Home Assistant instance, run only
this step: present the available dashboards, let the user pick one, and confirm the kiosk begins
displaying that exact dashboard.

**Acceptance Scenarios**:

1. **Given** a successful Home Assistant connection, **When** setup reaches this step, **Then**
   the user is shown a list of the dashboards available on their Home Assistant instance, by name.
2. **Given** the list of dashboards, **When** the user selects one, **Then** the device confirms
   the choice and begins showing that dashboard.
3. **Given** the user wants a different dashboard later, **When** they revisit setup, **Then**
   they can change the selection without repeating the network or Home Assistant connection steps.

---

### Edge Cases

- What happens if the user closes the browser or walks away mid-setup? The device MUST remain in
  setup mode (still broadcasting its own network) until each step is actually completed, so the
  user can resume where they left off.
- What happens if the home WiFi network's password changes later, or the network is no longer in
  range? The device MUST offer a way to re-enter setup mode (e.g. a physical action on the device)
  without erasing the already-configured Home Assistant connection and dashboard choice.
- What happens if no dashboards exist on the Home Assistant instance yet? The user MUST be told
  this plainly rather than shown a silently empty list, with guidance to create one and retry.
- What happens if two of these devices are being set up in the same home at once? Each device's
  own temporary network name MUST be distinguishable (e.g. includes a unique suffix) so the user
  can tell them apart.
- What happens if the user's home network requires something beyond a simple password (e.g.
  enterprise WiFi, a captive portal of its own)? Out of scope for this feature — see Assumptions.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The device MUST broadcast its own WiFi network whenever it has no working home
  network connection, so a user can always reach setup without external tools.
- **FR-002**: The device's own network name MUST be human-recognizable as belonging to this
  product and MUST include a distinguishing identifier when multiple such devices may be nearby.
- **FR-003**: A user connecting to the device's own network MUST be automatically directed to a
  setup page without manually navigating to a specific address.
- **FR-004**: The setup flow MUST let the user choose their home WiFi network from a list of
  networks the device can detect, and enter its password.
- **FR-005**: The device MUST attempt to join the selected home network and report clear
  success/failure feedback within a reasonable wait, distinguishing "wrong password" from "network
  not found" from "other failure" wherever the underlying failure reason allows it.
- **FR-006**: After successfully joining a home network, the device MUST stop broadcasting its own
  setup network.
- **FR-007**: The setup flow MUST prompt for a Home Assistant address and an access credential,
  with guidance on what each is and where to find it.
- **FR-008**: The device MUST verify the Home Assistant connection actively (not just save the
  values) before treating that step as complete, and report clear success/failure feedback.
- **FR-009**: On a successful Home Assistant connection, the device MUST retrieve and present the
  list of dashboards available to that account, by name.
- **FR-010**: The user MUST be able to select one dashboard from that list, and the device MUST
  begin displaying the selected dashboard once setup is complete.
- **FR-011**: The device MUST persist the home network credentials, Home Assistant connection
  details, and dashboard selection so that setup does not need to be repeated after a normal
  power cycle.
- **FR-012**: The user MUST have a way to re-enter setup mode after initial setup is complete
  (e.g. to change WiFi network, Home Assistant instance, or dashboard), without needing to erase
  configuration steps they don't intend to change.
- **FR-013**: Any step that fails MUST let the user correct and retry just that step, without
  forcing them to redo already-successful earlier steps.
- **FR-014**: The device MUST NOT display any credential (WiFi password, Home Assistant access
  credential) back to the user in plain text after it has been entered, beyond the immediate entry
  field during input.

### Key Entities

- **Device Setup State**: Whether the device currently has a working home network connection, a
  working Home Assistant connection, and a chosen dashboard. Drives whether the device is
  broadcasting its own setup network or operating normally.
- **Home Network Credentials**: The selected home WiFi network's identifier and password, entered
  once and persisted.
- **Home Assistant Connection**: The address and access credential used to reach the user's Home
  Assistant instance, plus the display name of that instance once verified.
- **Dashboard Selection**: The specific dashboard, out of those available on the connected Home
  Assistant instance, that the user has chosen for the kiosk to display.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A first-time user with no prior knowledge of this product can go from unboxing to
  the device showing their chosen Home Assistant dashboard in under 10 minutes, without
  consulting external instructions.
- **SC-002**: At least 95% of setup attempts with correct home WiFi and Home Assistant credentials
  complete successfully without the user needing to restart setup from the beginning.
- **SC-003**: 100% of setup failures (wrong password, unreachable Home Assistant, invalid
  credential) present the user with a plain-language explanation, not a generic or blank error.
- **SC-004**: A user can change which dashboard is displayed in under 2 minutes without
  re-entering WiFi or Home Assistant details.
- **SC-005**: A user can recover a device that lost its home WiFi connection (e.g. router replaced)
  back to fully working in under 10 minutes, without factory-resetting the Home Assistant
  connection or dashboard choice.

## Assumptions

- The user's home network is a standard consumer WiFi network (SSID + password); enterprise
  authentication (802.1X) and networks requiring their own captive-portal login are out of scope
  for this feature.
- "Home Assistant access credential" refers to a long-lived access token the user generates from
  their own Home Assistant profile; the flow to generate that token happens on the user's own
  phone/computer, outside this device, and this feature only needs the user to paste it in.
- The device has a way to detect "no working home network connection" at boot (e.g. no saved
  credentials, or a saved network that fails to connect) that triggers setup mode; the exact
  detection logic is a design detail for the implementation plan, not this spec.
- Wired network connection (if the device is plugged into Ethernet) is a reasonable substitute for
  WiFi setup in Story 1; if Ethernet is already connected and working, the device MAY skip
  straight to Story 2 rather than forcing WiFi setup. This is a reasonable default, not confirmed
  with the user, and can be revisited.
- "Dashboards" means Home Assistant's own named dashboard/view concept as exposed by its API;
  no custom dashboard-building is in scope for this feature.
