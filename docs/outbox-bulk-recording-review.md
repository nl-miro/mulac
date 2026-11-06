# Outbox bulk recording implementation review

Reviewed commit: `3dde3da` (`docs(outbox): record delivery status and add make targets`).
Review date: 2026-11-06.
Specification: [locked implementation plan](outbox-bulk-recording-plan.md).

## Verdict

**Changes requested before delivery.** The current implementation is substantially
more complete than the initial `8291cf8` commit: it now has a shared chunked write
engine, caller-owned transaction support, typed outcomes, database suites,
feature checks, a retry example, measurements and a commit-acknowledgment fault
check. The original handoff's missing implementation list is no longer current.

However, the database-suite preflight rejects legitimate PostgreSQL version
strings, transaction eligibility depends on a fixed session-setting value, and
several acceptance claims exceed what the tests actually exercise. Owner signoff
and the required GitHub status check also remain open according to the current
delivery-status document.

This is a source and local-build review, not independent certification of the
reported PostgreSQL measurements or previous database test runs.

## Findings

### R1 High priority: database preflight splits version strings on whitespace

**Location:** `scripts/run_outbox_database_suites.py:54–63`, especially line 62.

The query returns two output rows, but `version, database = probe.stdout.split()`
expects exactly two whitespace-delimited words. A legitimate response such as:

```text
16.15 (Debian 16.15-1.pgdg13+1)
outbox
```

raises `ValueError: too many values to unpack (expected 2, got 4)` before any suite
runs. Database names containing spaces cause the same class of failure. This
makes the required PostgreSQL gate dependent on server packaging and naming,
not on database availability or correctness.

**Evidence:** reproduced by importing the actual script and mocking only the
`subprocess.run` result with the response above. This was not a live PostgreSQL
run and does not establish the exact version string of the GitHub service image.

**Fix:** parse the two rows, not words, or return a machine-readable record with
explicit fields. Check row count and report a controlled preflight error. Add
script tests for plain versions, distribution-suffixed versions, database names
with spaces, failed commands and malformed output. Then execute the exact
workflow against its configured PostgreSQL image.

### R2 Medium priority: a preexisting probe value misidentifies an idle connection

**Location:** `libs/outbox/src/infra_diesel/recording.rs:107–115`.

Eligibility sets the transaction-local setting `mulac_outbox.probe` to the fixed
value `1`, then treats seeing `1` on the following statement as proof of an open
transaction. It does not distinguish that value from a preexisting session,
role, database or connection-option setting.

Reproduction scenario to add to `recording_caller`:

1. On an idle connection, execute `SET mulac_outbox.probe = '1'`.
2. Call `record_batch_on_connection(conn, &[])` without `BEGIN`.
3. The local probe expires after its statement, exposing the session value `1`;
   the empty request returns success instead of `NoOpenTransaction`.
4. A nonempty request instead reaches `SAVEPOINT` and produces a database error
   rather than the required distinct no-transaction error.

This does not demonstrate an unauthorized committed write: the nonempty path
still needs a savepoint. It is a caller-visible eligibility/error-contract defect.
The scenario follows from the probe implementation and setting lifetime; it was
not rerun against PostgreSQL in this review environment.

**Fix:** make the probe distinguish its own transaction-local value from any
baseline value, for example with a fresh per-call marker, and test idle and active
connections with preexisting settings. Preserve the raw-`BEGIN`, aborted-session,
lost-connection and empty-batch cases. Account for any caller setting changed by
the probe rather than silently assuming a pristine session.

### R3 Medium priority: the shared Unicode corpus contains self-comparisons

**Locations:** `libs/outbox/tests/common/corpus.rs:40–42`, the `surrogate pair`
accepted case, and `equivalent()` at lines 111–114.

Several cases no longer contain the escapes their labels or intended behavior
require:

- `escaped duplicate key` is another literal `{"a":1,"a":2}`.
- `duplicate across escapes` repeats literal `é` twice.
- `surrogate pair` contains a literal emoji, not a JSON surrogate escape pair.
- Several equivalence cases compare byte-identical strings, including the
  accented-character and emoji pairs.

Consequently, the shared database corpus and downstream serde feature matrix can
pass even if escaped-key uniqueness or escaped Unicode replay equivalence is
broken. The private parser unit tests do contain an escaped duplicate and a
surrogate pair, but they do not replace the required every-path and feature-matrix
evidence.

**Fix:** preserve literal JSON escapes in Rust raw strings. Add explicit pairs
such as `"a"` versus `"\u0061"`, `"é"` versus `"\u00e9"`, and `"😀"` versus
`"\ud83d\ude00"`, including duplicate object keys and replayed keys/values. Add a
sanity assertion that cases intended to test different encodings actually have
different bytes, then rerun the shared corpus under each feature configuration.

### R4 Medium priority: batch-of-one tests do not exercise the single-event APIs

**Locations:** `libs/outbox/tests/common/mod.rs:215–223`,
`libs/outbox/tests/recording_validation.rs`, and the single-event acceptance row
in `docs/outbox-bulk-recording-handoff.md`.

The `Api::Recorder` and `Api::Port` helper branches always invoke `record_batch`,
even for one-element slices. Most validation, replay and failure tests use these
branches. They therefore do not assert the same rules through `OutboxRecorder::record`
and `OutboxStorePort::record`, as the locked acceptance criteria explicitly require.
There are individual direct single-event tests, and the downstream corpus calls
recorder `record`, but these do not cover the whole required single-event matrix.

The implementation currently delegates correctly; this finding concerns missing
regression protection and an overclaimed acceptance item, not an observed
behavioral mismatch between the methods.

**Fix:** add explicit single-recorder and single-port modes to the reusable
harness, keeping separate batch modes so both are tested. Run applicable input,
replay and failure cases through those modes. Restrict duplicate-in-batch cases
to batch APIs and mismatched-entry-ID cases to entry APIs. Update the evidence
mapping to reference tests that actually call each public method.

## Additional acceptance gaps and review concerns

These are distinct from the four findings above and should be closed or explicitly
tracked before claiming the entire locked plan is implemented and verified.

- **Rollback after database failure:** engine fault tests inject failed rollback
  after a healthy replay conflict, and test a database failure followed by
  successful rollback. They do not explicitly combine an original database
  failure with a failing rollback on both pooled and caller paths. Add that
  combination and assert that the rollback step's source wins and the outcome
  remains uncertain. A dead connection test exercises one useful case, but does
  not replace this targeted recovery-step assertion.
- **Two-statement eligibility probe:** `fail_at = "PROBE"` fails before either
  statement. Add targeted failure of the second diagnostic statement too, since
  the plan requires failed diagnostic steps to retain their observed error and
  never become `NoOpenTransaction`.
- **Feature combinations:** `check_outbox_serde_features.sh` runs baseline, three
  individual features and all three together. It omits the three two-feature
  combinations despite describing “every downstream combination.” Run all eight
  configurations, or at least make the stated scope accurate and resolve the
  expected coverage against the locked acceptance text.
- **Suite inventory detection:** discovery identifies database suites by the
  literal text `OUTBOX_TEST_DATABASE_URL` in each top-level test file. A new suite
  using only `common::TestDb` without that text can evade the unlisted-suite
  check. Prefer inventorying every test file with explicit non-database
  exclusions, or another enforceable classification.
- **Retry example safety:** the example retains IDs and uncertainty history, but
  its public `handle` API accepts an arbitrary business callback and retries an
  uncertain commit automatically. The success branch's comment says business
  writes must be repeat-safe. Move that precondition prominently to the public
  API documentation, explain retention requirements beside it, and demonstrate
  an idempotent business operation or reconciliation before retry. Its current
  tests terminate a backend before commit rather than proving the handler avoids
  duplicating business work after a committed-but-unacknowledged attempt. The
  separate manual helper is not a test of this handler loop.
- **Zero retry budget:** `handle(..., max_attempts = 0, ...)` panics at
  `last.expect(...)`. Use `NonZeroU32`, validate the argument with an application
  error, or explicitly document the panic contract and test it.
- **Fault-injection limits:** transaction-control hooks fail before issuing the
  selected command. They verify wrapper classification, not every real driver
  state after a server-side or transport failure. Keep those claims separate
  from real commit failures and the withheld-acknowledgment check.

## Assessment of the implementation

### Recording and validation

The required batch port is object-safe and single recording delegates to it.
Validation occurs before connection acquisition or eligibility checks. Empty
pooled and recorder requests avoid pool acquisition. The parser preserves numeric
token spelling, compares decoded strings and keys, rejects duplicate decoded
keys, bounds container depth and preserves original payload bytes for storage.
No `serde_json::Value` or JSONB payload cast is used as the comparison authority.

No additional parser correctness defect was identified in source inspection.
The Unicode corpus finding means the evidence is narrower than the stated
coverage; it is not evidence that the parser itself currently mishandles Unicode.

### Atomicity and replay

The engine globally sorts event IDs before chunking inserts into 5,000 rows,
collects skipped IDs and verifies them with unlocked reads. Compared fields and
allowed differences match the plan on inspection. Stored metadata is decoded
from text so unreadability can become a replay conflict. Missing rows are
collected as identifiers rather than silently accepted.

The pooled wrapper now uses Diesel's transaction manager, addressing the original
raw-transaction pool-health concern. Caller operations use a savepoint and do not
commit the outer transaction. Recovery failure retains the relevant error source.
The remaining eligibility issue is R2. No new partial-commit defect was established.

### Outcomes and diagnostics

`RecordingError` and `RecordingOutcome` are feature-independent. Backend sources
remain boxed and inspectable; authored payload diagnostics use fixed categories.
Display limits source text and missing-ID lists while retaining full source data.
The public metadata-field strings remain caller-constructible, so display bounds
apply to errors the crate constructs, not arbitrary external construction.

The distinction between successful rollback and uncertain commit/rollback is
visible in the code. The documented decision to report no rows added after a
confirmed rollback followed by failed savepoint release is consistent with the
attempt's writes being gone; the release error is retained and the caller must
still abandon the outer transaction.

### Concurrency and delivery isolation

Global ordering across chunks and plain replay reads support the intended
no-deadlock/no-extra-row-lock design. The repository now includes controlled
contention, unsorted-control, deletion and consumer-overlap tests and a written
proof. These are materially stronger than simultaneous-start-only tests.
Their runtime behavior was not independently verified here. Owner review of the
proof is still required, and its conclusions must remain limited to the stated
schema, isolation level and absence of earlier caller locks.

### Documentation and delivery evidence

The README, specification, changelog, retry example, benchmark report and manual
fault helper now address the major topics missing from the initial commit.
However, the delivery-status statement that everything requested exists and
passes should be qualified until the findings and acceptance gaps above are
closed. Historical measurements and manual-run claims were read, not reproduced.
No GitHub run or repository-owner approval is established by this review.

## Verification performed for this review

```sh
cargo test --manifest-path libs/Cargo.toml -p mulac-outbox --lib --offline
cargo test --manifest-path libs/Cargo.toml -p mulac-outbox --no-default-features --offline
cargo test --manifest-path libs/Cargo.toml -p mulac-outbox --all-features --no-run --offline
```

- Library run: **50 passed, 9 ignored**. The ignored cases include database tests.
- No-default-features test command: passed.
- All-feature test compilation: passed; this is not execution of those suites.
- R1 Python reproduction: failed with the expected `ValueError`.
- No production code or tests were modified as part of this review.

Database limitations: `OUTBOX_TEST_DATABASE_URL` was unset; PostgreSQL did not
respond at `127.0.0.1:5432`; Docker socket access was denied. PostgreSQL client tools
were present, but the attempted local `initdb` binary was absent. Consequently,
this review did not run the PostgreSQL inventory, concurrency tests, serde
matrix's database cases, benchmarks or protocol fault check. The complete feature
shell script was not rerun; only the commands above were executed.

## Recommended merge sequence

1. Fix R1 and R2 and add targeted regressions.
2. Correct the shared corpus and single-event coverage, then close the recovery
   and diagnostic-probe coverage gaps.
3. Run the exact PostgreSQL workflow and feature jobs on the delivering PR,
   retaining discovered/executed counts and resolved feature evidence.
4. Reconcile the delivery-status claims with actual tests and record the tested
   commit for measurements/manual checks.
5. Obtain the owner's deadlock-proof and documentation review, close the manual
   acknowledgment-loss checks, and configure the required `main` status check.

Do not waive locked acceptance criteria merely because compilation or the
non-database tests pass.
