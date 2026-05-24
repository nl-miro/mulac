# Review: `test_apps/todo`

Reviewed against `docs/codestyle.md` and the current crate structure.

## Summary

The app is in decent shape, but there are a few high-signal issues around API boundaries and failure semantics. The biggest problems are a storage type leaking through the public surface, brittle inbox retry behavior, and request handling that can report failure after state has already changed.

## Findings

1. **High — storage schema leaks into the public API**
   - `test_apps/todo/src/assembly/mod.rs:24`
   - Tests also consume it directly in `tests/todo_create.rs`, `tests/todo_update.rs`, `tests/todo_complete.rs`, `tests/todo_reopen.rs`, `tests/todo_delete.rs`, and `tests/todo_due_date.rs`
   - `TodoRow` is re-exported from the assembly layer and used outside the storage boundary. That couples callers and tests to the database row shape instead of the API/DTO contract.
   - **Suggested fix:** keep row types private and expose DTOs or fetch helpers instead.

2. **High — request flow can report failure after durable state change**
   - `test_apps/todo/src/assembly/application.rs:309-327`
   - `test_apps/todo/src/assembly/application.rs:434-456`
   - `test_apps/todo/src/task_*.rs`
   - Handlers synchronously dispatch a command and then read back the todo. If downstream event consumption or subscriber persistence fails after the write commits, the API can return `500` even though state already changed.
   - **Suggested fix:** separate durable command acceptance from downstream publication, or return a pending/accepted response when publication is not part of the same durable boundary.

3. **Medium — inbox retry semantics are too rigid**
   - `test_apps/todo/src/lib.rs:144-169`
   - `test_apps/todo/src/lib.rs:183-197`
   - `test_apps/todo/tests/inbox.rs:123-178`
   - Re-delivery of the same message ID returns `409` even after a failed attempt. That makes transient failures unrecoverable with the same envelope ID.
   - **Suggested fix:** define behavior for retrying `failed` inbox rows and add a test for failed-then-replayed delivery.
