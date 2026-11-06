# Outbox bulk recording implementation handoff

## Objective and current state

Continue implementing [outbox-bulk-recording-plan.md](outbox-bulk-recording-plan.md).
That locked plan is the authoritative specification, including its complete
acceptance criteria. This handoff does not revise or waive any requirement.

The implementation is **partial and not ready for release**. Commit `8291cf8`
(`feat(outbox): add pooled batch recording and replay validation`) contains the
initial work. The branch at handoff creation is `feat/bulk-outobx-inserts`
(spelling intentional). The working tree was clean before adding this handoff.
The user requested a commit of the initial implementation, then this handoff.
No commit of this document has been requested yet.

## Repository instructions

- Read root `AGENTS.md` and any applicable nested instructions.
- Feature modules expose only `pub mod prelude`; implementation modules stay
  private. Callers import through preludes.
- Commits use Conventional Commits and the repository's LLM attribution rules.
- The Rust library workspace is **`libs/`**, not the repository root. Kernel
  and test applications have separate manifests.
- Keep the kernel unchanged; application code explicitly maps recording errors.
- Do not change the table schema or rewrite retained rows to repair conflicts.
- Do not relax the locked plan without explicit owner-approved plan revision.

## What the initial commit contains

### Public recording API

- `OutboxRecorder::record_batch(&[NewOutboxEnvelope])` converts envelopes into
  entries, validates the complete input, and calls the storage port once.
- `OutboxStorePort::record_batch(&[NewOutboxEntry])` is required and object-safe.
  Its default `record` delegates to a batch of one, not the reverse.
- Recorder `record` also delegates to a batch of one.
- Both paths return the new feature-independent `RecordingError`.
- Valid empty batches return without calling the store or acquiring a pool
  connection on the implemented recorder and pooled storage paths.
- Public exports use the existing prelude chain. Test doubles were updated.

### Validation and payload comparison

`libs/outbox/src/application/recording.rs` contains:

- `RecordingError` variants for duplicate ID, mismatched ID, invalid payload,
  invalid metadata, replay conflict, no transaction, missing events, connection
  acquisition, rolled-back database failure, and uncertain database failure.
- Boxed retained source errors with no implicit Diesel conversion.
- Whole-batch validation for duplicate IDs, metadata identity, payload syntax,
  routing, metadata NULs, and sequence bounds.
- A private recursive payload parser preserving exact numeric token spelling,
  decoding strings with `serde_json::from_str::<String>`, storing object keys in
  a `BTreeMap`, and rejecting duplicate decoded keys and excessive nesting.
  It does not enable `serde_json/arbitrary_precision`.
- Four focused parser tests: equivalence, invalid inputs, accepted unusual
  values, and the 127 versus 128 container boundary.

The parser and validator need the full acceptance corpus, feature-isolation tests,
and review. Existing focused tests are not proof of all planned guarantees.

### Pooled Diesel storage

`libs/outbox/src/infra_diesel/repository.rs` currently:

1. Validates before pool acquisition and returns immediately for empty input.
2. Acquires one pooled connection and issues raw SQL `BEGIN`.
3. Sorts references globally by event ID.
4. Inserts **one row per statement**, using `ON CONFLICT(id) DO NOTHING`.
5. For skipped inserts, reads payload and `meta::text` without `FOR UPDATE`,
   deserializes metadata, and compares identity, payload, message ID, routing,
   event type, content type, and sequence. Other fields are ignored.
6. Accumulates absent replay rows and fails the batch if any are missing.
7. Rolls back failures; a successful rollback changes a database error from
   uncertain to rolled back. Rollback failure retains the rollback error.
8. Commits on success; commit failure is uncertain.

One-row statements avoid the bind-count ceiling but are not an optimized bulk
insert. There is no internal retry or batch-size limit.

### Documentation and existing tests

- `CHANGELOG.md` has an initial, incomplete breaking-change summary.
- `libs/outbox/README.md` has an explicit partial-implementation status section.
- Recorder fake-store tests and prelude compile smoke tests use the new API.
- Existing invalid-sequence assertions were changed from `OutboxError::Routing`
  to `RecordingError::InvalidMetadata`.
- No new PostgreSQL acceptance suite or CI job was added.

## Verification already performed

Commands ran from `libs/`, using the local offline dependency cache:

```sh
cargo test -p mulac-outbox --lib --offline
cargo test -p mulac-outbox --no-default-features --offline
cargo test -p mulac-outbox --all-features --no-run --offline
cargo fmt --all
```

Observed results: 50 library tests passed; the no-default-features test command
passed; all-feature test executables compiled. `git diff --check` passed before
committing. An all-feature library check was also invoked during development;
the later all-feature test compilation is the stronger recorded compile result.

**No real PostgreSQL acceptance runs, concurrency tests, connection-fault tests,
performance measurements, or manual owner checks were performed.** Compiling
ignored database tests does not execute them. Rerun verification after changes.

## Review risks before extending the code

1. **Raw transaction management and pooled connection health.** `BEGIN`,
   `ROLLBACK`, and `COMMIT` bypass Diesel's transaction manager. Review pool
   cleanup, broken-connection detection, failure paths, panic unwinding, and
   reuse of a connection after uncertain transaction termination. No tests
   establish safety here. Do not transplant this wrapper into the caller path.
2. **Caller transaction support is entirely absent.** `NoOpenTransaction`
   exists only as an error variant. No caller-connection recording function,
   eligibility check, savepoint implementation, or associated tests exist.
3. **Eligibility detection is unresolved.** Diesel tracked depth alone cannot
   detect raw SQL `BEGIN`. Do not classify errors by localized message text,
   assume an unassigned transaction ID means no transaction, or open/commit an
   outer transaction as a probe. Valid empty calls still require eligibility
   checks; aborted and disconnected sessions must report database failures.
4. **Error contract needs review.** The variants exist, but outcome assertions,
   bounded display requirements, useful field/category data, and all source
   downcasts need tests. `MissingEvents` currently stores IDs with a generic
   display string. Phase-aware classification must survive recovery failures;
   a later retry must never erase an earlier uncertain outcome.
5. **Locking claims remain unproved.** Global sorting and unlocked replay reads
   are implemented, but there are no controlled contention/deletion tests or
   reviewed deadlock reasoning. Do not advertise the guarantee as verified.
6. **Old database tests may need fixture changes.** Suites testing malformed
   stored rows may have seeded those rows through recording. New recording
   validation can reject such fixtures. Seed intentionally corrupt rows using
   explicit test SQL, preserving the old reservation/delivery behavior under
   test; do not weaken validation to make fixtures work.
7. **Documentation remains incomplete.** Existing prose outside the README
   addition was not audited. The changelog does not yet cover every locked
   Compatibility item, notably subscriber and durable-retry implications.

## Suggested continuation order

### 1 Implement and test the transaction core

Design a shared recording engine taking a caller connection, with separate
pooled and caller-owned transaction wrappers. Implement eligibility/savepoint
handling independent of Diesel nesting depth and without parsing backend text.
Expose the caller envelope API only through a prelude with the `diesel` feature;
it must not require or acquire an outbox pool.

Preserve earlier caller work on validation/conflict when recovery succeeds.
Never commit the caller's outer transaction. Retain the actual failing recovery
step's error and conservatively report uncertainty on failed rollback or commit.
Test raw `BEGIN` before any other statement, Diesel transactions, nested caller
savepoints, and idle/active/aborted/disconnected empty requests.

### 2 Build the real PostgreSQL acceptance harness

Use `docs/outbox_entries.sql` in isolated per-test schemas, consistently
configuring pooled, direct, observer, and control connections. Start with the
full validation/replay corpus on every applicable API, including single calls,
then atomic failure after successful writes and batches above 65,535 events.

Add deterministic contention tests with independent control connections,
backend blocker/waiter observations and bounded watchdogs. Include an unsorted
control that actually deadlocks; replay nonblocking tests must observe actual
reservation, finalization, and sweep state changes. Add deletion races,
serialization failures and failure-injection cases exactly as the plan requires.
No simultaneous-start-only or sleep-only concurrency evidence is sufficient.

### 3 Complete integration and feature checks

- Add the compiled/exercised bounded-retry handler example with application-owned
  recording, business, and commit error categories, callback outcome tracking,
  persistent stable input and unresolved uncertainty history.
- Test TwoPhased and Direct subscriber behavior without kernel changes.
- Run the JSON corpus in package-isolated baseline and downstream feature
  combinations; record resolved features to prevent baseline unification.
- Verify default/no/all features, non-Diesel API/HTTP combinations, downstream
  prelude-only use, trait objects, and dependency trees without Diesel.
- Add a dedicated PostgreSQL PR job with explicit suite inventory, database
  preflight, and discovered/executed case counts, including eligible ignored
  database tests but excluding unrelated ignored AMQP cases.

### 4 Close documentation and release evidence

Audit every documentation target listed in the plan. Cover timeout/blocker
operations, independent pooled commits, retention limits, permanent conflict
recovery, uncertainty across retries, subscriber redrive, and all Compatibility
items. Link the handler example and reviewed deadlock reasoning.

Measure release-build all-new/all-replay recording time and client peak memory
with fresh processes and externally prepared fixtures. Do not invent a practical
batch-size recommendation without the specified measurements.

Owner/external work remains required: configure the PostgreSQL job as a required
`main` status check; execute and record the withheld-COMMIT-acknowledgment manual
checks for both pooled and caller-owned transactions; review the reasoning,
measurements, documentation coverage and tested commit on the delivering PR.
An LLM cannot substitute an assertion of success for these checks.

## Completion rule

Use the complete **Done when** section of the locked plan as a checklist and
record evidence item by item. This initial implementation must not be represented
as delivery of the whole plan. Keep this handoff and the README status accurate
as work progresses.
