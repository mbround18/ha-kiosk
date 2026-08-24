//! Hardware-independent logic shared with `firmware/` (the ESP32-P4 kiosk binary).
//!
//! `firmware`'s `[[bin]]` has `harness = false` (an esp-idf-template default, needed to avoid a
//! rust-analyzer issue and because there's no meaningful way to run a std test harness process
//! on the embedded target anyway) - which means `cargo test` inside `firmware/` never actually
//! runs anything, regardless of how many `#[test]` functions exist there. Rather than fighting
//! that, the logic that doesn't actually need ESP-IDF/hardware at all (JSON contract types,
//! text formatting, layout math) lives here instead, in a plain host-target crate that's a
//! member of the root workspace - so `cargo test -p firmware-core` just works normally.

pub mod home;
pub mod layout;
pub mod portal_api;
pub mod screen;
