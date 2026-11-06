# Outbox bulk recording plan

## Why

Applications often produce many outbox events from one piece of work. Recording currently handles one event at a time, which leaves three problems:

- **Partial batches.** A failure partway through a group of events leaves some recorded and some not. The caller has to work out which is which and recover.
- **No shared transaction with business data.** Recording uses its own pooled connection, so business writes and their outbox events can't commit or roll back together. That is the guarantee a transactional outbox exists to provide.
- **Silent replay conflicts.** Recording an existing event ID always succeeds and keeps the stored row, even if the new content differs. A producer bug or a reused ID goes unnoticed, and the changed message is never published.

Storage errors are also reduced to text, so callers can't reliably tell these failures apart.

## What

### Batch recording

- Callers can record a list of events as one operation.
- The operation is all-or-nothing on PostgreSQL: if any event is rejected or any write fails, no new row from that batch remains.
- An empty batch succeeds and does nothing.
- Batch size must not be limited by database statement limits. Very large batches must work the same way as small ones.
- No maximum batch size is enforced. The documentation states the costs of a very large batch (memory use and one long transaction) and suggests a practical size.
- Success returns no per-event result. A new insert and a matching replay look the same to the caller.

### Recording inside a caller's transaction

- Callers using Diesel can record a batch on a connection they own, alongside their business writes.
- Inside the caller's transaction, nothing is durable until the caller commits, and the outer transaction is never committed on their behalf.
- A failed batch leaves no rows of its own, and the caller can still choose to handle the error and continue its transaction.
- Above `READ COMMITTED`, a concurrent write of the same event can raise a serialization failure. It reaches the caller as an unchanged database error, and recording does not retry it. The caller retries the whole transaction, because it owns the business writes too.
- Applications that don't use Diesel must not need to depend on it.

### Validation

These rules apply to every recording path. An invalid event rejects the whole request before anything is written.

| Input | Result |
|---|---|
| The same event ID twice in one request, even if identical | Rejected |
| Payload that isn't a JSON object | Rejected |
| Blank routing key | Rejected |
| Sequence outside 1 to `i64::MAX` | Rejected |
| No sequence | Accepted |

Accepted payloads are stored exactly as supplied, without reformatting.

### Replay

Event ID remains the identity of a row. Message ID remains delivery metadata, so a new event ID creates a new row even if the message ID is reused.

When a request contains an event ID that is already stored:

| Field | Rule |
|---|---|
| Payload | Must be equivalent as JSON; whitespace and key order don't matter, but numbers must be written identically |
| Message ID, routing key, event type, content type, sequence | Must match |
| Correlation ID, causation ID, source | May differ; the stored values are kept |
| Timestamps and delivery state | Ignored |

- Comparing numbers by their written form means `1` and `1.0` conflict, and numbers that differ never match, however large. Producers must serialize numbers consistently across retries.
- A matching replay succeeds and leaves the stored row untouched, whatever its delivery state. It does not trigger another publication.
- A conflicting replay fails the whole request and never overwrites stored data.
- Single-event recording follows these same rules, so applications get one recording policy. This changes behavior for existing callers.

### Errors

Callers must be able to distinguish these outcomes programmatically, and database errors must keep their original detail:

- Duplicate event ID within a request
- Invalid payload
- Invalid routing key or sequence
- Conflict with a stored event, identifying one conflicting event ID
- Events that should be stored but can't be found, identifying all of them
- Database failure
- Failure to obtain a connection

### Design constraints

- Bulk recording is part of the storage port, with no fallback that quietly records events one at a time. The port must state what its implementations guarantee.
- Public API is exposed only through the crate's `prelude`.
- No new repository type is needed; the existing storage repository takes on this responsibility.
- The existing `outbox_entries` table stays as it is, with no migration or rewriting of existing rows. Existing rows that fail the new payload rule stay publishable, but can't be replayed.

## Out of scope

- Publishing to the broker in batches. Delivery stays at-least-once, per entry.
- Automatic outbox recording from the kernel.
- Moving the Todo and Twitter test apps onto the core recorder.
- Guaranteeing that events are published in batch order. The outbox spec already disclaims ordering across entries, and callers should not infer it from a batch.

## Compatibility

This is a breaking change, to be called out in the changelog:

- Custom storage implementations must support bulk recording.
- New error cases break exhaustive matches on the error type.
- Payloads that aren't JSON objects, and conflicting replays, previously succeeded and now fail.

## Done when

**All of these are shown against a real PostgreSQL database:**

- Batches below, at, and well above any internal statement boundary are recorded completely.
- A failure late in a large batch leaves none of that batch's new rows.
- Replaying a batch succeeds and changes nothing, including on rows that are reserved, failed, or already delivered.
- Each compared field rejects a change on its own, and each tolerated field leaves the stored value intact.
- Payloads differing only in whitespace or key order replay successfully. `1` vs `1.0`, and large numbers that differ only beyond floating-point precision, both conflict.
- A batch mixing new events with a conflicting event leaves no new rows.
- With a separate business table: outer commit keeps both, outer rollback removes both, and a handled recording failure keeps earlier business writes.
- Concurrent overlapping batches, submitted in opposite orders, all finish without deadlock and with correct final content. This includes running alongside consumers and the stale-reservation sweep.
- A concurrent write of the same event inside a `REPEATABLE READ` caller transaction surfaces as a database error the caller can identify as a serialization failure, and retrying the transaction succeeds.

**Also required:**

- Invalid input reaches no storage, and each error case above is asserted.
- The database tests run in CI. Today no workflow runs them, so a green build says nothing about these guarantees.
- Default, no-default-feature, and all-feature builds pass.
- All-new, all-replay, and mixed batches have been measured at realistic sizes, so that any optimization work rests on data.
- `docs/outbox-spec.md` and the crate README describe bulk recording, the replay rules including number comparison, when a successful call becomes durable, batch-size guidance, and retrying serialization failures.
