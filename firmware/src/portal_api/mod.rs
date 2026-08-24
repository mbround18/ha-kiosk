//! Thin re-export of `firmware_core::portal_api` - the JSON contract types + their tests live
//! there (a plain host-target crate) rather than here, because this crate's `[[bin]]` has
//! `harness = false`, which turns out to silently skip compiling `#[cfg(test)]` code entirely
//! (not just "not run it") - see `firmware-core/src/portal_api.rs`'s doc comment.
pub use firmware_core::portal_api::*;
