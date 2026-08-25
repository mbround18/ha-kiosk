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
//
// A pure entry-COUNT cap, as this used to be (`const MAX_ENTRIES: usize = 5000`), turned out to
// be a real bug on this hardware, not just a theoretical one: confirmed on-device over a ~75
// minute run, mistaken for hours for a WebSocket/TLS-layer leak before being root-caused here.
// Real log lines from this codebase run ~150-180 bytes once heap-allocated (timestamp + message,
// e.g. "kiosk: fetched 5 room(s): [...] (free heap: ... total, ... internal)"), at roughly one
// new entry per refresh cycle - 5000 of those is ~750KB-900KB, several times this board's entire
// ~220KB internal heap budget. The ring buffer was never going to hit its 5000-entry cap before
// exhausting memory outright; every session/heap-leak investigation on this board that ran long
// enough would eventually run into *this*, regardless of what else was fixed. Capping by total
// buffered bytes instead - evicting oldest entries until back under budget, same as the count cap
// did by count - keeps this diagnostic tool from ever being the largest consumer of the exact
// memory budget it exists to help diagnose. 48KB retains a genuinely useful amount of history
// (comparable to the log windows actually used to diagnose real issues on this device so far)
// while leaving the rest of this board's small internal heap for everything else.
const MAX_BYTES: usize = 48 * 1024;

// Starts at 1, not 0, so `events_since(0)` (the "give me everything" sentinel) actually includes
// the very first event too.
static SEQ: AtomicU32 = AtomicU32::new(1);
static LOG: OnceLock<Mutex<VecDeque<(u32, String)>>> = OnceLock::new();
// Sum of every currently-buffered line's byte length - tracked alongside the ring rather than
// recomputed on every push (an O(n) rescan every single event, on top of the O(n) `events_since`
// scan already run every `/api/debug` poll, wasn't worth paying for something this cheap to
// maintain incrementally).
static TOTAL_BYTES: AtomicU32 = AtomicU32::new(0);

fn log_ring() -> &'static Mutex<VecDeque<(u32, String)>> {
    LOG.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn push(line: String) {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut ring = log_ring().lock().unwrap();
    TOTAL_BYTES.fetch_add(line.len() as u32, Ordering::Relaxed);
    ring.push_back((seq, line));
    while TOTAL_BYTES.load(Ordering::Relaxed) as usize > MAX_BYTES {
        let Some((_, evicted)) = ring.pop_front() else {
            break; // buffer's empty but TOTAL_BYTES is still over budget - shouldn't happen, but
                    // don't spin forever if it somehow does
        };
        TOTAL_BYTES.fetch_sub(evicted.len() as u32, Ordering::Relaxed);
    }
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

/// Diagnostic (see `net::kiosk`'s periodic call to this) for a real, still-unexplained ~150-160
/// bytes/cycle internal heap decline that survived three targeted fixes without slowing down at
/// all (an HA WebSocket subscription-cleanup bug, hourly session recycling, and this ring
/// buffer's own entry-count-vs-byte-size cap) - meaning continuing to guess from aggregate free-
/// heap numbers alone wasn't working. `heap_caps_get_info`'s `allocated_blocks` count (distinct
/// from `total_free_bytes`, which every existing log line already reports) is the key extra
/// signal: if it climbs by ~1 every refresh cycle in lockstep with the byte decline, that's
/// confirmation of one small allocation per cycle that's never freed (a real, narrow leak to keep
/// hunting by code inspection) rather than many allocations each losing a few bytes to
/// fragmentation (`largest_free_block` shrinking faster than `total_free_bytes` would be the
/// tell for that instead - reported here too, for the same reason).
///
/// Deliberately NOT ESP-IDF's `heap_trace_start`/`heap_trace_dump` (true allocation-callsite
/// tracing, which would name the exact leaking line): `heap_trace_record_t`'s C layout has
/// config-conditional fields (a `TAILQ_ENTRY` linked-list node under
/// `CONFIG_HEAP_TRACING_STANDALONE`, absent from `esp-idf-sys`'s bindgen output - unlike
/// `heap_caps_get_info` below, apparently not on its allowlist) that would have to be hand-
/// replicated to pass a correctly-sized buffer to the C library. Getting that ABI even slightly
/// wrong risks the C library writing past whatever buffer this side actually allocated -
/// memory corruption, not just a diagnostic that doesn't work - not a risk worth taking for a
/// debugging tool. This is coarser (a count, not a backtrace) but has zero ABI risk.
pub fn log_heap_block_counts(caps: u32, label: &str) {
    let mut info: esp_idf_svc::sys::multi_heap_info_t = unsafe { std::mem::zeroed() };
    unsafe { esp_idf_svc::sys::heap_caps_get_info(&mut info, caps) };
    event(format!(
        "heap[{label}]: {} free bytes, {} allocated blocks, {} free blocks, largest free block {}",
        info.total_free_bytes, info.allocated_blocks, info.free_blocks, info.largest_free_block
    ));
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
