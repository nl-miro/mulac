# Outbox bulk recording: delivery status

[outbox-bulk-recording-plan.md](outbox-bulk-recording-plan.md) is the locked,
authoritative specification. This document does not revise or waive any of it. It
records what has been built, where the evidence for each **Done when** item is,
what only the repository owner can do, and the judgment calls made inside the
plan's latitude.

**Status: implemented and verified locally; not yet delivered.** Everything the
plan asks of code, tests, CI scripts and documentation exists and passes against
PostgreSQL 16. Delivery still needs the owner steps below. In particular, the new
GitHub workflow has not yet run on GitHub: its scripts were run locally against a
default-configured PostgreSQL 16 container, which is what its service container is.

## Running everything

```sh
export OUTBOX_TEST_DATABASE_URL=postgres://user:pw@localhost:5432/db   # disposable database
make test-outbox-database    # 13 suites, 87 cases; preflight and discovered/executed counts
make check-outbox-serde      # payload corpus under 5 serde_json feature configurations
make check-outbox-features   # default/no/all/API/HTTP builds; diesel absent when disabled
make check test-libs         # database-free gates used by the existing CI
python3 scripts/check_outbox_commit_cut.py   # manual check: withheld COMMIT acknowledgment
```

## Evidence by Done-when item

| Plan item | Where it is shown |
| --- | --- |
| Batches above 65,535 events; empty batches (pool unavailable; caller path needs no outbox pool) | `recording_pooled`: `batch_above_the_bind_parameter_limit…_on_every_path` (65,536 events on all three paths), `empty_batches_succeed_without_a_connection`; `recording_caller`: empty batches in idle, active, aborted, killed sessions |
| Every validation and equivalence rule, on every path | `recording_validation` (corpus in `tests/common/corpus.rs`; rejections run with no reachable database), `recording_replay` (each compared field alone, tolerated fields, default message IDs) |
| `serde_json` feature independence | `test_apps/outbox_serde_matrix` via `scripts/check_outbox_serde_features.sh`: baseline plus `arbitrary_precision`, `preserve_order`, `raw_value` and all three; resolved features printed; the baseline is asserted free of the three |
| Single-event recording as a batch of one | The corpus and replay suites record through `Api::Recorder`, `Api::Port` and `Api::Caller`; `OutboxStorePort::record` and `OutboxRecorder::record` delegate to `record_batch` |
| Each Failures-table case, with data and outcome | `recording_failures::every_failures_table_case…`, `no_connection…`; `MissingEvents` in the lib `fault_tests` deletion-race tests |
| Accepted payloads read back byte-identical | `recording_validation::accepted_payloads_are_stored_exactly_as_supplied` |
| Database failure after an earlier insertion chunk succeeded; conflict after new-row writes | lib `fault_tests::database_failure_after_an_earlier_chunk…` (the trigger reports the rows it could see, proving the earlier chunk was written); `recording_pooled::replay_conflict_after_new_writes…` |
| Replay changes nothing; rule-breaking stored rows | `recording_replay` (all delivery states; corrupt rows conflict, stay unchanged, keep reservation behavior) |
| Caller's transaction | `recording_caller` (commit/rollback with a business table, handled errors, no-open-transaction, raw `BEGIN` first, nested savepoints including the crate's own name, idle/active/aborted/killed) |
| Concurrency | `recording_concurrency` (same-ID writers; opposite-order batches across a chunk boundary with observed blocking; unsorted control that really deadlocks; deadlock and `REPEATABLE READ` serialization failure with whole-transaction retry); `recording_replay` (reservation, completion and sweep proceed while a recording transaction is open, all-replay and mixed) |
| Controlled deletion during verification | lib `fault_tests::row_deleted_between_insert_and_verification…`, `…_after_verification…` |
| Connection loss on every path | `recording_failures::connection_loss_during_a_batch…` |
| Failed rollback, savepoint establishment, release, probe, commit | lib `fault_tests` (a hook fails exactly one control step); `recording_failures::a_failed_pooled_commit…` (a real failing `COMMIT`) |
| Diagnostics: no payload, bounded `Display`, sources inspectable | `recording_failures::crate_authored_diagnostics…`, `display_is_bounded…` |
| Uncertain attempt then conflict or exhaustion stays uncertain | `handler_example` |
| Dedicated PostgreSQL job, inventory, counts | `.github/workflows/outbox-database.yml`, `scripts/run_outbox_database_suites.py` (fails on a missing suite, zero cases, skipped cases, or a database suite not in the inventory) |
| Isolation per test | `tests/common/mod.rs`: a schema per test from `docs/outbox_entries.sql`; the four older suites were converted and their departures from the baseline are explicit `ALTER`s |
| Manual: withheld `COMMIT` acknowledgment | `scripts/check_outbox_commit_cut.py` (see below) |
| Feature builds; no Diesel when disabled; prelude-only downstream tests | `scripts/check_outbox_features.sh` (with a positive control), `tests/prelude_smoke.rs` |
| Deadlock reasoning written down | `docs/outbox-recording-deadlock-reasoning.md` (**review pending**) |
| Batch size from measurements | `docs/outbox-bulk-recording-measurements.md` (`scripts/measure_outbox_recording.py`) |
| Docs: handler example, timeouts, pooled-in-transaction warning, conflict recovery, subscribers | `docs/outbox-spec.md` (Recording), `libs/outbox/README.md`, `libs/outbox/examples/record_with_retry.rs` |
| Subscriber behavior test | `test_apps/outbox_subscriber_behavior` (in the inventory) |
| No stale claims in the listed documents | Greps of all ten documents and the port comments; `docs/components.md`, `docs/contracts.md`, `docs/outbox-spec.md` and the crate README were corrected |
| `CHANGELOG.md` | `[Unreleased]`, item by item against Compatibility |

Mutation checks were run while writing the tests, and each caught what it should:
removing the global sort fails the two no-deadlock tests; locking replayed rows
fails the consumer test; letting a rollback error replace the callback's error
fails the handler test. All changes were reverted.

## What only the repository owner can do

1. **Make the job a required status check on `main`.** `main` has no branch
   protection today. The job is named `outbox database`. Do it once the workflow
   has run on a pull request, so the check exists.
2. **Close the manual check.** `scripts/check_outbox_commit_cut.py` was run by the
   implementer against a local PostgreSQL 16 and all six checks passed (pooled
   recording reported `DatabaseUncertain`; the caller's `COMMIT` failed as an
   application error; business and outbox rows were really committed; same-ID
   retries added no rows). That is evidence, not a closure: record the tested commit
   and the result on the delivering pull request yourself.
3. **Review the deadlock reasoning** (`docs/outbox-recording-deadlock-reasoning.md`)
   and the **measurements**, which were taken on one machine and are a record, not a
   threshold.
4. **Review documentation coverage** against the plan's list of caller-visible rules.

## Judgment calls inside the plan's latitude

- **Insert chunks of at most 5,000 rows** (nine bound columns, well under the
  65,535 limit). Internal; tests depend on it only for "more than one chunk".
- **Open-transaction detection** sets and reads back a transaction-local setting
  (`mulac_outbox.probe`) in two statements, using a value unique to each call so no
  preexisting session or role setting can be mistaken for it, and restoring the
  caller's own value. It is visible only inside the caller's
  transaction and ends with it. It never reads Diesel's depth or error text.
- **One savepoint, `mulac_outbox_record`.** A caller savepoint with the same name is
  shadowed during the call and intact afterwards (tested).
- **Failed `RELEASE` after a confirmed rollback** reports a rolled-back database
  failure with the release error, replacing a conflict that would otherwise have been
  returned. The plan does not say; the rows are known to be gone, and the caller must
  abandon the transaction on any database failure anyway.
- **Pooled recording uses Diesel's transaction manager**, not raw `BEGIN`/`COMMIT`,
  so a connection with an unconfirmed commit or rollback is flagged broken and never
  returns to the pool (tested).
- **Public additions beyond the plan's text:** `RecordingOutcome` and
  `RecordingError::outcome()`, so callers and tests can read the outcome column of the
  Failures table; the function name `record_batch_on_connection`.
- **Test-only hooks** (`cfg(test)`) in the recording engine create the deletion races
  and fail single control steps deterministically. They do not exist in release builds.
- **No change to the kernel, the table schema, or the Todo and Twitter apps.**

## Known limits

- The workflow file has not run on GitHub; only its scripts have been run locally.
- Test timing: the concurrency tests observe blocked backends and use watchdogs
  rather than sleeps, but they have only been run on one fast machine.
- The `docker exec` memory sampler in the measurement script is coarse; database
  growth includes page cache and is an upper bound.
- `docs/item-failure-isolation.md` and other historical plans were not edited and are
  exempt under the plan.
