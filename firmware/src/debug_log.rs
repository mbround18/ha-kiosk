//! A crate-wide, in-memory ring buffer of timestamped debug events, dumped on request via
//! `GET /api/debug` (`net::ap_portal`). Built to diagnose a hard-to-reproduce crash in the
//! trust-confirm flow (`ha::rest`/`display::trust_confirm`) where the on-device serial monitor
//! proved unreliable - attaching it resets the board (a known quirk of this hardware), and even
//! when left attached, its `script`-captured log has been observed to silently truncate
//! mid-session for no code-related reason. Watching over HTTP sidesteps both problems and,
//! unlike NVS-backed breadcrumbs, needs no flash writes.
//!
//! `/api/debug` returns immediately with everything buffered so far rather than holding the
//! connection open - ESP-IDF's httpd here processes one request at a time on a single task, so a
//! handler that never returns would block every other request for as long as it's connected.
//! Poll it repeatedly instead for a live-ish view.
//!
//! Call `event(...)` from anywhere in the firmware; call `events_since(...)` to read entries
//! newer than a given sequence number (used by the endpoint handler).

use core::ffi::{c_char, c_int};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use esp_idf_svc::sys::{esp_log_set_vprintf, va_list};

// Generous: repeatedly polling `/api/debug` itself generates httpd request-handling log lines
// that get captured too (self-referential noise), so a small buffer gets evicted by the very
// act of reading it before anything interesting can be inspected.
const MAX_ENTRIES: usize = 5000;

// Starts at 1, not 0, so `events_since(0)` (the "give me everything" sentinel) actually includes
// the very first event too.
static SEQ: AtomicU32 = AtomicU32::new(1);
static LOG: OnceLock<Mutex<VecDeque<(u32, String)>>> = OnceLock::new();

fn log_ring() -> &'static Mutex<VecDeque<(u32, String)>> {
    LOG.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_ENTRIES)))
}

fn push(line: String) {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut ring = log_ring().lock().unwrap();
    if ring.len() >= MAX_ENTRIES {
        ring.pop_front();
    }
    ring.push_back((seq, line));
}

/// Appends a timestamped event.
pub fn event(msg: impl Into<String>) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    push(format!("[{now_ms}] {}", msg.into()));
}

// `vsnprintf` isn't in esp-idf-sys's bindgen allowlist, but the symbol is always present (newlib
// links it into every ESP-IDF app already) - declaring it ourselves with the matching C ABI is
// the standard way to reach an unbound-but-present libc symbol.
unsafe extern "C" {
    fn vsnprintf(buf: *mut c_char, size: usize, fmt: *const c_char, args: va_list) -> c_int;
}

/// Redirects ESP-IDF's own logging (every `ESP_LOGx`/`log::info!` etc., including the detailed
/// mbedtls/esp-tls failure lines that only ever showed up on serial before) into this ring
/// buffer too - so `/api/debug` shows the same detail the serial monitor would, without needing
/// serial at all (which has proven unreliable: attaching it resets this board, and even left
/// attached its captured log has been observed to silently truncate mid-session). Call once,
/// early in `main()`, before anything else logs.
pub fn install() {
    unsafe {
        esp_log_set_vprintf(Some(capture_vprintf));
    }
}

/// On this target `va_list` is a plain pointer (`__builtin_va_list = *mut c_void`), so unlike the
/// general C case, consuming it once here via `vsnprintf` and not forwarding it to any other
/// consumer needs no `va_copy` dance - we just print the already-formatted bytes ourselves
/// afterward instead of re-invoking a second variadic consumer with the same `args`.
unsafe extern "C" fn capture_vprintf(fmt: *const c_char, args: va_list) -> c_int {
    let mut buf = [0u8; 256];
    let n = unsafe { vsnprintf(buf.as_mut_ptr() as *mut c_char, buf.len(), fmt, args) };
    if n > 0 {
        let len = (n as usize).min(buf.len().saturating_sub(1));
        if let Ok(s) = core::str::from_utf8(&buf[..len]) {
            let trimmed = s.trim_end_matches(['\r', '\n']);
            if !trimmed.is_empty() {
                push(trimmed.to_string());
            }
        }
        use std::io::Write;
        let _ = std::io::stdout().write_all(&buf[..len]);
        let _ = std::io::stdout().write_all(b"\n");
    }
    n
}

/// Returns every event with a sequence number greater than `after`, plus the highest sequence
/// number now in the ring (pass that back in as `after` on the next call to only get what's new).
pub fn events_since(after: u32) -> (Vec<String>, u32) {
    let ring = log_ring().lock().unwrap();
    let mut out = Vec::new();
    let mut max_seq = after;
    for (seq, line) in ring.iter() {
        if *seq > after {
            out.push(line.clone());
            max_seq = max_seq.max(*seq);
        }
    }
    (out, max_seq)
}
