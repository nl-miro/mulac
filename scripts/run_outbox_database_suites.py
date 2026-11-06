#!/usr/bin/env python3
"""Runs every outbox suite that needs PostgreSQL, from an explicit inventory.

A suite that is missing, discovers no cases, or executes fewer cases than it
discovered (for example because they stayed ignored) fails the run, and so does
a database suite that exists in `libs/outbox/tests` but is not in the
inventory. Needs OUTBOX_TEST_DATABASE_URL pointing at a disposable database.
"""
import csv
import io
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
LIBS = ["--manifest-path", str(ROOT / "libs" / "Cargo.toml"), "-p", "mulac-outbox"]
SUBSCRIBER = ["--manifest-path", str(ROOT / "test_apps" / "outbox_subscriber_behavior" / "Cargo.toml")]

# (label, cargo package arguments, test selector, libtest name filter, minimum
# cases it must discover). A suite that discovers fewer fails the run.
INVENTORY = [
    ("lib: infra_diesel (engine, fault injection, deletion races)", LIBS, ["--lib"], ["infra_diesel::"], 20),
    ("diesel_storage", LIBS, ["--test", "diesel_storage"], [], 4),
    ("lifecycle", LIBS, ["--test", "lifecycle"], [], 5),
    ("reservation_isolation", LIBS, ["--test", "reservation_isolation"], [], 7),
    ("timeout_sweep", LIBS, ["--test", "timeout_sweep"], [], 3),
    ("recording_pooled", LIBS, ["--test", "recording_pooled"], [], 4),
    ("recording_caller", LIBS, ["--test", "recording_caller"], [], 9),
    ("recording_concurrency", LIBS, ["--test", "recording_concurrency"], [], 9),
    ("recording_replay", LIBS, ["--test", "recording_replay"], [], 7),
    ("recording_validation", LIBS, ["--test", "recording_validation"], [], 6),
    ("recording_failures", LIBS, ["--test", "recording_failures"], [], 7),
    ("handler_example (documented retry loop)", LIBS, ["--test", "handler_example"], [], 8),
    ("subscriber_behavior (TwoPhased and Direct)", SUBSCRIBER, ["--test", "subscriber_behavior"], [], 3),
]

# Database suites that are deliberately run by another script.
RUN_ELSEWHERE = {"test_apps/outbox_serde_matrix/tests/matrix.rs": "scripts/check_outbox_serde_features.sh"}


def fail(message):
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def cargo_test(package, selector, filters, extra):
    return subprocess.run(
        ["cargo", "test", *package, *selector, "--", *filters, *extra],
        capture_output=True,
        text=True,
    )


PREFLIGHT_QUERY = "SELECT current_setting('server_version'), current_database()"


def parse_preflight(output):
    """Returns (server version, database name) from one CSV record.

    The version string may hold spaces and parentheses, for example
    `16.15 (Debian 16.15-1.pgdg13+1)`, and a database name may hold anything
    PostgreSQL allows, so the record is parsed as CSV, never split on spaces.
    """
    rows = list(csv.reader(io.StringIO(output)))
    if len(rows) != 1 or len(rows[0]) != 2 or not all(rows[0]):
        raise ValueError(f"expected one record with a version and a database name, got {output!r}")
    version, database = rows[0]
    return version, database


def preflight(url):
    try:
        probe = subprocess.run(
            ["psql", url, "--csv", "-t", "-c", PREFLIGHT_QUERY],
            capture_output=True,
            text=True,
        )
    except OSError as error:
        fail(f"database preflight could not run psql: {error}")
    if probe.returncode != 0:
        fail(f"database preflight failed:\n{probe.stderr}")
    try:
        version, database = parse_preflight(probe.stdout)
    except ValueError as error:
        fail(f"database preflight returned unexpected output: {error}")
    print(f"database preflight ok: PostgreSQL {version}, database {database}")


def main():
    url = os.environ.get("OUTBOX_TEST_DATABASE_URL")
    if not url:
        fail("OUTBOX_TEST_DATABASE_URL is not set")
    preflight(url)

    # Every file that talks to the database must be inventoried.
    inventoried = {sel[1] for _, _, sel, _, _ in INVENTORY if sel[0] == "--test"}
    suites = list((ROOT / "libs" / "outbox" / "tests").glob("*.rs"))
    suites += (ROOT / "test_apps").glob("outbox_*/tests/*.rs")
    for path in sorted(suites):
        relative = str(path.relative_to(ROOT))
        if "OUTBOX_TEST_DATABASE_URL" not in path.read_text() or relative in RUN_ELSEWHERE:
            continue
        if path.stem not in inventoried:
            fail(f"database suite {relative} is not in the required-suite inventory")
    for name in sorted(inventoried):
        if not list(ROOT.glob(f"libs/outbox/tests/{name}.rs")) + list(ROOT.glob(f"test_apps/outbox_*/tests/{name}.rs")):
            fail(f"inventoried suite {name} does not exist")

    totals = []
    for label, package, selector, filters, minimum in INVENTORY:
        listing = cargo_test(package, selector, filters, ["--list"])
        if listing.returncode != 0:
            fail(f"{label}: cannot discover cases:\n{listing.stderr}")
        discovered = len(re.findall(r": test$", listing.stdout, re.M))
        if discovered == 0 or discovered < minimum:
            fail(f"{label}: discovered {discovered} cases, expected at least {minimum}")

        run = cargo_test(package, selector, filters, ["--include-ignored"])
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
