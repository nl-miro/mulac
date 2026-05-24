# Review: `test_apps/todo`

Reviewed by: Claude Sonnet 4.6  
Reviewed against: `docs/test_app_todo.md` (functional spec) and the CLAUDE.md module boundary rules.

---

## Summary

The implementation covers all 10 required functionalities and the tests are thorough. The command/event pipeline, inbox deduplication, and outbox write-path all work correctly in the happy path. Three structural issues stand out: a storage type leaking through the public `io` boundary, a port race between command commitment and inbox status write, and the absence of a retry path for failed inbox messages.

---

## Findings

### 1. High — `TodoRow` exposed through the public `io` module

**Files**
- `src/assembly/mod.rs:24` — re-exports `infra_sqlx_pg::entity::TodoRow` through `assembly::io`
- `src/lib.rs:378` — re-exports it again at the crate root via `pub mod io`
- `tests/todo_create.rs:3` — `use test_app_todo::io::TodoRow`
- `tests/todo_complete.rs:3` — same import
- `tests/todo_reopen.rs:3` — same import
- `tests/todo_update.rs:3` — same import
- `tests/todo_due_date.rs:3` — same import

**Problem**  
`TodoRow` is a SQLx `FromRow` struct that mirrors the raw database schema. Exporting it publicly couples every caller (including the integration tests) to the database column layout. Any schema change — adding a column, renaming one, changing a type — becomes a compile-time break for all consumers of the public API, not just the persistence layer.

The module boundary rules in CLAUDE.md state that internal sub-modules (`infra_sqlx_pg`, etc.) must remain private. `TodoRow` is an infra detail and should not cross that boundary.

**Suggested fix**  
Remove `TodoRow` from `assembly::io` and from the crate's `pub mod io`. In the affected tests, replace the direct `sqlx::query_as::<_, TodoRow>` calls with a custom inline struct defined in `utils.rs`, or with plain `sqlx::query_scalar` / `query` calls that assert only the columns under test. `TodoDto` is already public and is sufficient for API-level assertions.

---

### 2. High — inbox handler can mark a message `processed` after state has already changed, masking the real outcome

**File**: `src/lib.rs` (inbox `http` module, lines 244–266)

```
record_received   ← writes inbox row as 'received'
dispatch_inbound_command   ← runs command + event consumer synchronously
    mark_processed / mark_failed   ← updates inbox row
fetch_todo   ← reads back the todo and returns it
```

`dispatch_command` on `MulacState` runs the command consumer and the event consumer inline (`src/assembly/application.rs`, the `commanding` module). If the event consumer fails (e.g. the `OutboxSubscriber` cannot write), `dispatch_command` returns `Err`. The handler then calls `mark_failed` and returns an HTTP error — correct so far. But if `mark_processed` itself fails (transient DB error), the handler returns a 500 even though the command was committed and the event was recorded. The inbox row stays as `received`, so a retry of the same `message_id` will be rejected with a 409 conflict. The message is permanently stuck.

**Suggested fix**  
Treat the inbox status update as best-effort or wrap it in a transaction that spans the full command dispatch. At minimum, document that `mark_processed` failures do not roll back the command, and add a test for that scenario.

---

### 3. Medium — failed inbox messages cannot be retried with the same envelope ID

**File**: `src/lib.rs` (inbox `http::record_received`, lines 144–169)

The `record_received` function uses `ON CONFLICT (id) DO NOTHING` and returns an `AppError::Conflict` when `rows_affected() == 0`. This makes the check entirely ID-based: any row with that ID, regardless of its `status`, blocks re-delivery.

If a message is recorded as `received` and then `mark_failed` is called (or the process crashes before `mark_processed` runs), the inbox row persists with status `received` or `failed`. Any re-delivery of the same `message_id` — whether by the upstream producer or an operator retry — is rejected with 409. There is no code path that allows a failed inbox entry to be replayed.

The test `duplicate_inbox_message_id_returns_409` only covers the success-then-duplicate case and does not exercise failure-then-retry.

**Suggested fix**  
Change the conflict guard to allow re-delivery when the existing row has status `failed` (or `received` after a timeout). One approach: replace `ON CONFLICT DO NOTHING` with a check that reads the current status and returns conflict only if the status is `processed`. Add a test that delivers a message, simulates failure, and confirms re-delivery with the same ID succeeds.

---

### 4. Low — test isolation relies on a single global Mutex rather than per-test schema isolation

**File**: `tests/utils.rs:103–110`

`start_test_app` acquires a process-wide `OwnedMutexGuard` to serialize all integration tests, then truncates shared tables. This works but means tests run fully serially, and any test that accidentally holds the guard while panicking will deadlock subsequent tests. A common alternative is per-test PostgreSQL schemas or database names, which would allow parallelism and eliminate the global lock.

This is a quality-of-life issue, not a correctness bug, and is acceptable for a test application. Noting it here in case the suite grows large enough that serial execution becomes a bottleneck.

---

### 5. Low — `create_todo` helper is copy-pasted across six test files

**Files**: `tests/todo_complete.rs`, `tests/todo_delete.rs`, `tests/todo_reopen.rs`, `tests/todo_update.rs`, `tests/todo_due_date.rs`, `tests/inbox.rs`

Each file defines its own `async fn create_todo(base_url, title) -> Uuid` with identical bodies. This is test duplication with no variation. Moving the helper to `tests/utils.rs` would reduce noise and make future signature changes easier.

---

## Positive observations

- The module structure inside each `task_*.rs` file (`mod models`, `mod handler`, `mod infra_sqlx_pg`, `mod http`, `pub mod io`) correctly follows the CLAUDE.md boundary rules. Only `io` is public; all internal sub-modules are private.
- The `dispatch_command` path in `MulacState` runs the full command-then-event pipeline synchronously within the HTTP request, which makes the integration tests deterministic without any polling or sleep.
- Outbox entries are verified in every mutating test (`assert_outbox_pending`), and command/event journal entries are also checked (`assert_command_completed`, `assert_event_completed`). This gives good coverage of the full write path.
- Validation errors (`blank title`) are exercised at both the HTTP and inbox entry points.
- The inbox test for unknown command type (`UnknownCommand`) correctly expects a 4xx rather than a 500, and the implementation satisfies this through `poem-openapi` deserialization rejection before the handler runs.
