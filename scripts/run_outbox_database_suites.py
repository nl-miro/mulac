#!/usr/bin/env python3
"""Runs every outbox suite that needs PostgreSQL, from an explicit inventory.

A suite that is missing, discovers no cases, or executes fewer cases than it
discovered (for example because they stayed ignored) fails the run, and so does
a database suite that exists in `libs/outbox/tests` but is not in the
inventory. Needs OUTBOX_TEST_DATABASE_URL pointing at a disposable database.
"""
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
MANIFEST = str(ROOT / "libs" / "Cargo.toml")
TESTS = ROOT / "libs" / "outbox" / "tests"

# (label, cargo test selector, libtest name filter, minimum cases it must discover)
INVENTORY = [
    ("lib: infra_diesel (engine, fault injection, deletion races)", ["--lib"], ["infra_diesel::"], 19),
    ("diesel_storage", ["--test", "diesel_storage"], [], 4),
    ("lifecycle", ["--test", "lifecycle"], [], 5),
    ("reservation_isolation", ["--test", "reservation_isolation"], [], 7),
    ("timeout_sweep", ["--test", "timeout_sweep"], [], 3),
    ("recording_pooled", ["--test", "recording_pooled"], [], 4),
    ("recording_caller", ["--test", "recording_caller"], [], 6),
    ("recording_concurrency", ["--test", "recording_concurrency"], [], 7),
    ("recording_replay", ["--test", "recording_replay"], [], 7),
    ("recording_validation", ["--test", "recording_validation"], [], 5),
    ("recording_failures", ["--test", "recording_failures"], [], 7),
]


def fail(message):
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def cargo_test(selector, filters, extra):
    return subprocess.run(
        ["cargo", "test", "--manifest-path", MANIFEST, "-p", "mulac-outbox", *selector, "--", *filters, *extra],
        capture_output=True,
        text=True,
    )


def preflight(url):
    probe = subprocess.run(
        ["psql", url, "-Atc", "SHOW server_version; SELECT current_database()"],
        capture_output=True,
        text=True,
    )
    if probe.returncode != 0:
        fail(f"database preflight failed:\n{probe.stderr}")
    version, database = probe.stdout.split()
    print(f"database preflight ok: PostgreSQL {version}, database {database}")


def main():
    url = os.environ.get("OUTBOX_TEST_DATABASE_URL")
    if not url:
        fail("OUTBOX_TEST_DATABASE_URL is not set")
    preflight(url)

    # Every file that talks to the database must be inventoried.
    inventoried = {sel[1] for _, sel, _, _ in INVENTORY if sel[0] == "--test"}
    for path in sorted(TESTS.glob("*.rs")):
        if "OUTBOX_TEST_DATABASE_URL" in path.read_text() and path.stem not in inventoried:
            fail(f"database suite {path.name} is not in the required-suite inventory")
    for name in sorted(inventoried):
        if not (TESTS / f"{name}.rs").exists():
            fail(f"inventoried suite {name} does not exist")

    totals = []
    for label, selector, filters, minimum in INVENTORY:
        listing = cargo_test(selector, filters, ["--list"])
        if listing.returncode != 0:
            fail(f"{label}: cannot discover cases:\n{listing.stderr}")
        discovered = len(re.findall(r": test$", listing.stdout, re.M))
        if discovered == 0 or discovered < minimum:
            fail(f"{label}: discovered {discovered} cases, expected at least {minimum}")

        run = cargo_test(selector, filters, ["--include-ignored"])
        sys.stdout.write(run.stdout)
        if run.returncode != 0:
            sys.stderr.write(run.stderr)
            fail(f"{label}: suite failed")
        result = re.search(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", run.stdout)
        if not result:
            fail(f"{label}: no test result line")
        passed, failed, ignored = map(int, result.groups())
        if failed or ignored or passed != discovered:
            fail(f"{label}: discovered {discovered}, passed {passed}, failed {failed}, skipped {ignored}")
        totals.append((label, discovered, passed))

    print("\nrequired database suites (discovered / executed):")
    for label, discovered, passed in totals:
        print(f"  {label}: {discovered} / {passed}")
    print(f"total: {sum(d for _, d, _ in totals)} cases in {len(totals)} suites")


if __name__ == "__main__":
    main()
