# heap-monitor

Polls the kiosk's `/api/debug` endpoint every 30s and appends every new heap data point to
`data/heap.csv` - for watching a slow internal-heap decline over many hours (8-12+) unattended,
which manual `curl` polling doesn't scale to.

## Run it

```sh
cd tools/heap-monitor
# Confirm/update KIOSK_URL in compose.yaml first if the device's IP has changed.
docker compose up -d
```

Runs until stopped (`docker compose down`) or the host reboots - `restart: unless-stopped` brings
it back after a Docker daemon restart, but not after the machine itself powers off.

## Read the data back

`data/heap.csv` columns: `device_ts_ms` (the device's own epoch-ms timestamp, not the poll time -
so gaps in polling don't distort the trend), `kind` (`fetched` or `heap_internal`),
`free_heap_total`, `free_heap_internal`, `allocated_blocks`, `free_blocks`, `largest_free_block`
(the latter three only populated on `heap_internal` rows).

Quick trend check without any extra tooling:

```sh
# Internal heap over time, one point every ~15-25s
awk -F, 'NR>1 && $2=="fetched" {print $1, $4}' data/heap.csv | tail -50

# Whether allocated_blocks is trending up (a real per-cycle leak) or just noisy
awk -F, 'NR>1 && $2=="heap_internal" {print $1, $5}' data/heap.csv
```

Or load the CSV into a spreadsheet / `pandas.read_csv` for a real plot once there's enough data
to see the shape of the trend clearly.

## Why this exists

Chasing a real ~150-160 bytes/refresh-cycle internal heap decline on the kiosk (`firmware/`) that
survived three targeted fixes (an HA WebSocket subscription-cleanup bug, hourly session
recycling, and the debug log ring buffer's own entry-count-vs-byte-size cap) without slowing down
measurably. A few hand-checked samples of `allocated_blocks` didn't show a clean "+1 block every
cycle" signature, but a handful of points over a few minutes isn't enough data to be confident
either way - this collects enough of a real, long run to actually tell.
