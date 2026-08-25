#!/usr/bin/env python3
"""Polls the kiosk device's `/api/debug` endpoint (`firmware::debug_log`, see
`firmware/src/debug_log.rs`) on a fixed interval and appends every new free-heap data point to a
CSV, for offline analysis of a slow internal-heap decline over many hours (8-12+) - manual `curl`
polling doesn't scale to an observation window that long unattended, and the device's own ring
buffer is capped at 48KB, so nothing this side doesn't actively collect is lost for good.

Captures two kinds of line the firmware already logs:
  - "kiosk: fetched N room(s): [...] (free heap: X total, Y internal)" - every refresh cycle
    (~15-25s cadence, see `net::kiosk::ROOM_REFRESH_INTERVAL`).
  - "heap[internal]: Y free bytes, A allocated blocks, F free blocks, largest free block L" -
    every 4th refresh cycle (`net::kiosk::HEAP_BLOCK_COUNT_LOG_EVERY_N_REFRESHES`), the extra
    block-count/fragmentation signal added while chasing a leak that didn't have an obvious
    single-allocation-site fingerprint from just a few hand-checked samples.

`/api/debug` always returns its FULL current ring buffer (bounded by size, not a "since last
poll" delta) - not an event stream - so this tracks the highest device-embedded timestamp already
written to the CSV and only appends genuinely new lines each poll. Safe to stop/restart this
container at any point without creating duplicate rows or needing to coordinate with the device.
"""

import csv
import os
import re
import sys
import time
import urllib.request
from pathlib import Path

KIOSK_URL = os.environ.get("KIOSK_URL", "http://192.168.1.220/api/debug")
POLL_INTERVAL_SECONDS = int(os.environ.get("POLL_INTERVAL_SECONDS", "30"))
OUT_PATH = Path(os.environ.get("OUT_PATH", "/data/heap.csv"))

FETCHED_RE = re.compile(
    r"^\[(?P<ts>\d+)\] kiosk: fetched \d+ room\(s\).*"
    r"\(free heap: (?P<total>\d+) total, (?P<internal>\d+) internal\)"
)
BLOCK_COUNT_RE = re.compile(
    r"^\[(?P<ts>\d+)\] heap\[(?P<label>\w+)\]: (?P<free_bytes>\d+) free bytes, "
    r"(?P<allocated_blocks>\d+) allocated blocks, (?P<free_blocks>\d+) free blocks, "
    r"largest free block (?P<largest_free_block>\d+)"
)

FIELDNAMES = [
    "device_ts_ms",
    "kind",
    "free_heap_total",
    "free_heap_internal",
    "allocated_blocks",
    "free_blocks",
    "largest_free_block",
]


def load_last_seen_ts() -> int:
    """The CSV itself is the only state this needs - no separate checkpoint file to lose track
    of or get out of sync with what's actually been written."""
    if not OUT_PATH.exists():
        return 0
    last = 0
    with OUT_PATH.open(newline="") as f:
        for row in csv.DictReader(f):
            last = max(last, int(row["device_ts_ms"]))
    return last


def ensure_header() -> None:
    if not OUT_PATH.exists():
        OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
        with OUT_PATH.open("w", newline="") as f:
            csv.DictWriter(f, fieldnames=FIELDNAMES).writeheader()


def parse_new_rows(text: str, last_seen_ts: int) -> list[dict]:
    rows = []
    for line in text.splitlines():
        m = FETCHED_RE.match(line)
        if m:
            ts = int(m["ts"])
            if ts > last_seen_ts:
                rows.append(
                    {
                        "device_ts_ms": ts,
                        "kind": "fetched",
                        "free_heap_total": m["total"],
                        "free_heap_internal": m["internal"],
                        "allocated_blocks": "",
                        "free_blocks": "",
                        "largest_free_block": "",
                    }
                )
            continue
        m = BLOCK_COUNT_RE.match(line)
        if m:
            ts = int(m["ts"])
            if ts > last_seen_ts:
                rows.append(
                    {
                        "device_ts_ms": ts,
                        "kind": f"heap_{m['label']}",
                        "free_heap_total": "",
                        "free_heap_internal": m["free_bytes"],
                        "allocated_blocks": m["allocated_blocks"],
                        "free_blocks": m["free_blocks"],
                        "largest_free_block": m["largest_free_block"],
                    }
                )
    rows.sort(key=lambda r: r["device_ts_ms"])
    return rows


def poll_once(last_seen_ts: int) -> int:
    try:
        with urllib.request.urlopen(KIOSK_URL, timeout=10) as resp:
            text = resp.read().decode("utf-8", errors="replace")
    except Exception as e:
        # Deliberately doesn't crash the container on a single failed poll - a device reboot
        # (new DHCP lease, different IP - this exact device has changed IP more than once during
        # tonight's session) or a transient network blip shouldn't end an 8-12 hour collection
        # run. It just logs and tries again next interval.
        print(f"[scrape] fetch failed: {e}", file=sys.stderr, flush=True)
        return last_seen_ts

    rows = parse_new_rows(text, last_seen_ts)
    if not rows:
        return last_seen_ts

    with OUT_PATH.open("a", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=FIELDNAMES)
        for row in rows:
            writer.writerow(row)

    new_last = max(r["device_ts_ms"] for r in rows)
    print(f"[scrape] appended {len(rows)} new row(s), last_seen_ts={new_last}", flush=True)
    return new_last


def main() -> None:
    ensure_header()
    last_seen_ts = load_last_seen_ts()
    print(
        f"[scrape] starting, polling {KIOSK_URL} every {POLL_INTERVAL_SECONDS}s, "
        f"last_seen_ts={last_seen_ts}",
        flush=True,
    )
    while True:
        last_seen_ts = poll_once(last_seen_ts)
        time.sleep(POLL_INTERVAL_SECONDS)


if __name__ == "__main__":
    main()
