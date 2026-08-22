/// Whether the onboard Ethernet PHY currently reports link-up.
///
/// STUB: this board's Ethernet PHY driver (chip + RMII pin assignment) has not been verified
/// against real hardware in this codebase yet - getting it wrong risks the same class of
/// silent boot crash the chip-revision bug caused (see specs/001-wifi-ha-provisioning's
/// project memory). Until that's done, always report "not up" so the device safely falls
/// back to WiFi AP/captive-portal setup (data-model.md's documented fallback), which is fully
/// verified. Revisit once the PHY/pin config is confirmed.
pub fn link_up() -> bool {
    false
}
