#!/usr/bin/env python3
"""Measures outbox batch recording and writes the report.

For every batch size and mode (all new events, all replayed events) it runs a
fresh process per sample against a fresh schema, with fixtures and replay seed
data prepared by a separate process beforehand. Timing covers the recording call
only; client memory is the measuring process's peak (whole process, including
its input batch); database memory is reported separately from the database
container's cgroup. There is no pass/fail threshold: this only records numbers.

Needs OUTBOX_TEST_DATABASE_URL (a disposable PostgreSQL), psql, and
OUTBOX_MEASURE_CONTAINER (a Docker container name) for database-side memory.

    scripts/measure_outbox_recording.py --output docs/outbox-bulk-recording-measurements.md
"""
import argparse
import datetime
import os
import pathlib
import platform
import statistics
import subprocess
import sys
import threading
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parent.parent
BINARY = ROOT / "libs" / "target" / "release" / "examples" / "measure_recording"
SCHEMA_SQL = ROOT / "docs" / "outbox_entries.sql"


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, **kwargs)


def psql(url, sql, schema=None):
    env = dict(os.environ)
    if schema:
        env["PGOPTIONS"] = f"-csearch_path={schema}"
    return run("psql", url, "-Atc", sql, env=env).stdout


class MemorySampler(threading.Thread):
    """Polls the database container's cgroup memory while a sample runs."""

    def __init__(self, container):
        super().__init__(daemon=True)
        self.container, self.peak, self.stop = container, 0, threading.Event()

    def current(self):
        out = run("docker", "exec", self.container, "cat", "/sys/fs/cgroup/memory.current").stdout
        return int(out)

    def run(self):
        while not self.stop.is_set():
            try:
                self.peak = max(self.peak, self.current())
            except subprocess.CalledProcessError:
                pass


def sample(base_url, container, mode, size):
    schema = "m_" + uuid.uuid4().hex
    psql(base_url, f"CREATE SCHEMA {schema}")
    sep = "&" if "?" in base_url else "?"
    url = f"{base_url}{sep}options=-csearch_path%3D{schema}"
    env = dict(os.environ, DATABASE_URL=url)
    try:
        env_psql = dict(os.environ, PGOPTIONS=f"-csearch_path={schema}")
        run("psql", base_url, "-q", "-f", str(SCHEMA_SQL), env=env_psql)
        if mode == "replay":
            run(str(BINARY), "prepare", str(size), env=env)  # seed outside the measured process
        sampler = MemorySampler(container)
        before = sampler.current()
        sampler.start()
        out = run(str(BINARY), "measure", str(size), env=env).stdout
        sampler.stop.set()
        sampler.join()
        fields = dict(part.split("=") for part in out.split())
        return {
            "record_ms": float(fields["record_ms"]),
            "payload_bytes": int(fields["payload_bytes"]),
            "peak_kib": int(fields["peak_kib"]),
            "rss_before_kib": int(fields["rss_before_kib"]),
            "db_delta_mib": max(0, sampler.peak - before) / 1048576,
        }
    finally:
        psql(base_url, f"DROP SCHEMA {schema} CASCADE")


def host_summary():
    cpu = next(l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name"))
    mem = next(int(l.split()[1]) for l in open("/proc/meminfo") if l.startswith("MemTotal"))
    return f"{cpu}, {os.cpu_count()} logical CPUs, {mem // 1048576} GiB RAM, Linux {platform.release()}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sizes", default="1,100,1000,5000,10000,50000,100000")
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--output")
    args = parser.parse_args()
    base_url = os.environ.get("OUTBOX_TEST_DATABASE_URL") or sys.exit("OUTBOX_TEST_DATABASE_URL is not set")
    container = os.environ.get("OUTBOX_MEASURE_CONTAINER") or sys.exit("OUTBOX_MEASURE_CONTAINER is not set")
    run("cargo", "build", "--release", "--manifest-path", str(ROOT / "libs" / "Cargo.toml"),
        "-p", "mulac-outbox", "--example", "measure_recording")

    version = psql(base_url, "SHOW server_version").strip()
    settings = psql(
        base_url,
        "SELECT name || ' = ' || setting || coalesce(unit, '') FROM pg_settings WHERE name IN "
        "('shared_buffers','synchronous_commit','fsync','full_page_writes','wal_level','max_wal_size',"
        "'checkpoint_timeout','work_mem','max_connections') ORDER BY name",
    ).strip().splitlines()

    rows = []
    for mode in ("new", "replay"):
        for size in (int(s) for s in args.sizes.split(",")):
            samples = [sample(base_url, container, mode, size) for _ in range(args.samples)]
            ms = [s["record_ms"] for s in samples]
            row = {
                "mode": mode,
                "size": size,
                "payload_mib": samples[0]["payload_bytes"] / 1048576,
                "ms_median": statistics.median(ms),
                "ms_min": min(ms),
                "ms_max": max(ms),
                "peak_mib": max(s["peak_kib"] for s in samples) / 1024,
                "added_mib": max(s["peak_kib"] - s["rss_before_kib"] for s in samples) / 1024,
                "db_mib": max(s["db_delta_mib"] for s in samples),
            }
            rows.append(row)
            print(row, flush=True)

    lines = [
        "# Outbox bulk recording measurements",
        "",
        f"Recorded {datetime.date.today()} by `scripts/measure_outbox_recording.py`. These numbers inform the",
        "batch-size guidance in the outbox documentation. They are a record, not a performance threshold:",
        "nothing in CI asserts them, and they describe this machine, not yours.",
        "",
        "## Setup",
        "",
        f"- Host: {host_summary()}",
        f"- PostgreSQL {version} (container `{container}`), on the same host over loopback TCP. Settings:",
        *[f"  - `{line}`" for line in settings],
        "- Build: `cargo build --release`, `examples/measure_recording.rs`, default features.",
        f"- Samples: {args.samples} per cell, each in a fresh process against a fresh schema built from",
        "  `docs/outbox_entries.sql`.",
        "- Payloads: JSON objects cycling over 20 events: 14 of about 256 bytes, 5 of about 2 KiB and",
        "  1 of about 16 KiB (about 1.5 KiB on average). Event IDs are sequential and rows are otherwise",
        "  identical in shape.",
        "",
        "## Boundaries",
        "",
        "- **Time** is the single `OutboxRecorder::record_batch` call: validation, conversion, all statements,",
        "  replay verification and commit, on a pooled connection that was already open. Process start,",
        "  pool creation and **input construction are outside it**.",
        "- **all new**: the table is empty. **all replay**: the same batch was recorded beforehand by a",
        "  separate process, so every event is already stored with identical content.",
        "- **Client memory** is the measuring process's `VmHWM` over its whole life, so it **includes the",
        "  input batch**. \"Added\" is the peak minus the resident size just before the call, which is what",
        "  recording itself added. Database memory is not part of it.",
        "- **Database memory** is the growth of the database container's cgroup `memory.current` while the",
        "  sample ran, polled continuously (each poll is a `docker exec`, tens of milliseconds). It includes page cache, so treat it as an upper bound on",
        "  working memory, and it can read as zero for small batches.",
        "",
        "## Results",
        "",
        "| mode | events | payload MiB | time median ms | min–max ms | events/s | client peak MiB | client added MiB | database growth MiB |",
        "| --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: |",
    ]
    for r in rows:
        lines.append(
            f"| {r['mode']} | {r['size']} | {r['payload_mib']:.1f} | {r['ms_median']:.0f} | "
            f"{r['ms_min']:.0f}–{r['ms_max']:.0f} | {r['size'] / (r['ms_median'] / 1000):.0f} | "
            f"{r['peak_mib']:.0f} | {r['added_mib']:.0f} | {r['db_mib']:.0f} |"
        )
    text = "\n".join(lines) + "\n"
    if args.output:
        pathlib.Path(args.output).write_text(text)
    print(text)


if __name__ == "__main__":
    main()
