# Outbox bulk recording measurements

Recorded 2026-11-06 by `scripts/measure_outbox_recording.py`. These numbers inform the
batch-size guidance in the outbox documentation. They are a record, not a performance threshold:
nothing in CI asserts them, and they describe this machine, not yours.

## Setup

- Host: AMD RYZEN AI MAX+ 395 w/ Radeon 8060S, 32 logical CPUs, 30 GiB RAM, Linux 7.0.0-42-generic
- PostgreSQL 16.15 (container `mulac-outbox-impl-pg`), on the same host over loopback TCP. Settings:
  - `checkpoint_timeout = 300s`
  - `fsync = on`
  - `full_page_writes = on`
  - `max_connections = 300`
  - `max_wal_size = 1024MB`
  - `shared_buffers = 163848kB`
  - `synchronous_commit = on`
  - `wal_level = replica`
  - `work_mem = 4096kB`
- Build: `cargo build --release`, `examples/measure_recording.rs`, default features.
- Samples: 3 per cell, each in a fresh process against a fresh schema built from
  `docs/outbox_entries.sql`.
- Payloads: JSON objects cycling over 20 events: 14 of about 256 bytes, 5 of about 2 KiB and
  1 of about 16 KiB (about 1.5 KiB on average). Event IDs are sequential and rows are otherwise
  identical in shape.

## Boundaries

- **Time** is the single `OutboxRecorder::record_batch` call: validation, conversion, all statements,
  replay verification and commit, on a pooled connection that was already open. Process start,
  pool creation and **input construction are outside it**.
- **all new**: the table is empty. **all replay**: the same batch was recorded beforehand by a
  separate process, so every event is already stored with identical content.
- **Client memory** is the measuring process's `VmHWM` over its whole life, so it **includes the
  input batch**. "Added" is the peak minus the resident size just before the call, which is what
  recording itself added. Database memory is not part of it.
- **Database memory** is the growth of the database container's cgroup `memory.current` while the
  sample ran, polled continuously (each poll is a `docker exec`, tens of milliseconds). It includes page cache, so treat it as an upper bound on
  working memory, and it can read as zero for small batches.

## Results

| mode | events | payload MiB | time median ms | min–max ms | events/s | client peak MiB | client added MiB | database growth MiB |
| --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: |
| new | 1 | 0.0 | 2 | 2–5 | 435 | 10 | 0 | 4 |
| new | 100 | 0.1 | 7 | 6–8 | 13514 | 11 | 1 | 10 |
| new | 1000 | 1.5 | 52 | 45–53 | 19305 | 20 | 7 | 16 |
| new | 5000 | 7.3 | 203 | 194–203 | 24655 | 64 | 40 | 82 |
| new | 10000 | 14.6 | 394 | 393–397 | 25349 | 87 | 50 | 92 |
| new | 50000 | 73.0 | 2266 | 1924–3292 | 22060 | 268 | 125 | 152 |
| new | 100000 | 145.9 | 3805 | 3799–3899 | 26283 | 493 | 217 | 255 |
| new | 200000 | 292.0 | 7645 | 7480–7747 | 26162 | 944 | 402 | 430 |
| replay | 1 | 0.0 | 3 | 3–4 | 357 | 10 | 0 | 15 |
| replay | 100 | 0.1 | 7 | 7–8 | 13699 | 11 | 1 | 15 |
| replay | 1000 | 1.5 | 39 | 38–41 | 25445 | 20 | 7 | 15 |
| replay | 5000 | 7.3 | 192 | 186–202 | 26055 | 64 | 40 | 82 |
| replay | 10000 | 14.6 | 374 | 366–391 | 26752 | 87 | 51 | 107 |
| replay | 50000 | 73.0 | 1903 | 1869–1918 | 26280 | 267 | 124 | 108 |
| replay | 100000 | 145.9 | 3819 | 3678–3832 | 26183 | 493 | 217 | 105 |
| replay | 200000 | 292.0 | 7112 | 7002–7162 | 28121 | 946 | 404 | 108 |
