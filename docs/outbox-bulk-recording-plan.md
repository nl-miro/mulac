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
- The operation is all-or-nothing on PostgreSQL: new rows from a batch commit together or not at all. Validation rejection writes nothing; a confirmed rollback leaves no new row from that batch. An error alone is not proof of rollback (see failure outcomes below).
- An empty batch succeeds and does nothing.
- Batch size must not be limited by database statement limits. Very large batches must work the same way as small ones.
- No maximum batch size is enforced. The documentation states the costs of a very large batch (memory use and one long transaction) and suggests a practical size.
- Success returns no per-event result. A new insert and a matching replay look the same to the caller.

### Recording inside a caller's transaction

- Callers using Diesel can record a batch on a connection they own, alongside their business writes.
- Inside the caller's transaction, nothing is durable until the caller commits, and the outer transaction is never committed on their behalf.
- A failed batch leaves no new rows of its own after successful rollback to its savepoint. The caller can handle a recoverable error and continue only if transaction recovery succeeds. A broken connection or failed savepoint rollback carries no such guarantee.
- Above `READ COMMITTED`, a concurrent write of the same event can raise a serialization failure. It reaches the caller as an unchanged database error, and recording does not retry it. The caller retries the whole transaction, because it owns the business writes too.
- Caller-owned transactions can already hold locks from business writes or earlier recording calls. They are not covered by the deadlock-free requirement for isolated recording calls. Deadlocks can occur even at `READ COMMITTED`; they reach the caller as unchanged database errors without an internal retry. The caller retries the whole transaction, not just the recording call.
- Applications that don't use Diesel must not need to depend on it.

### Failure outcomes

- Validation rejection or confirmed transaction/savepoint rollback establishes that the batch added no rows. If rollback or connection recovery fails, report that failure without promising that the caller's transaction remains usable.
- Losing the acknowledgment of a pooled recording transaction's commit can return an error even though the entire batch committed and may already be published. Report the outcome as uncertain, not as a confirmed rollback, while preserving the original database error.
- Retry recording after an uncertain outcome with the same event IDs and the same replay-compared content. Never generate replacement event IDs to recover from that error. A matching replay safely accepts an already committed batch without adding rows.
- When the caller owns the transaction, it also owns commit-outcome recovery. Replaying outbox events alone does not make business writes idempotent; the caller must reconcile or safely retry the complete business operation after an uncertain commit.

### Validation

These rules apply to every recording path. An invalid event rejects the whole request before anything is written.

| Input | Result |
|---|---|
| The same event ID twice in one request, even if identical | Rejected |
| Entry `id` differs from `meta.event_id` on an entry-based recording path | Rejected |
| Payload that isn't a JSON object | Rejected |
| Blank routing key | Rejected |
| Sequence outside 1 to `i64::MAX` | Rejected |
| No sequence | Accepted |

Accepted payloads are stored exactly as supplied, without reformatting.

Envelope conversion establishes `id == meta.event_id`; direct storage inputs must be checked rather than trusting callers to preserve that invariant. Single-event, bulk, and caller-owned-connection entry points must not bypass it.

### Replay

Event ID remains the identity of a row. Message ID remains delivery metadata, so a new event ID creates a new row even if the message ID is reused.

When a request contains an event ID that is already stored:

| Field | Rule |
|---|---|
| Stored `meta.event_id` | Must equal the row ID and the incoming event ID; inconsistent legacy rows cause a conflict |
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
- Mismatched entry ID and metadata event ID, identifying both values
- Invalid payload
- Invalid routing key or sequence
- Conflict with a stored event, identifying one conflicting event ID
- Events that should be stored but can't be found, identifying all of them
- Database failure, retaining the original error and distinguishing confirmed absence of new batch rows from an uncertain outcome
- Rollback or transaction-recovery failure, retaining both the original failure and the recovery error; callers must not be told that continuing the transaction is safe
- Failure to obtain a connection

### Design constraints

- Bulk recording is part of the storage port, with no fallback that quietly records events one at a time. The port must state what its implementations guarantee.
- Public API is exposed only through the crate's `prelude`.
- No new repository type is needed; the existing storage repository takes on this responsibility.
- The existing `outbox_entries` table stays as it is, with no migration or rewriting of existing rows. Existing rows that fail the new payload rule or have inconsistent stored event identities retain their existing delivery behavior, but can't be replayed.

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
- Direct storage inputs with differing entry and metadata event IDs are rejected; replay of an existing row with inconsistent event identity now conflicts.

## Done when

**All of these are shown against a real PostgreSQL database:**

- Batches below, at, and well above any internal statement boundary are recorded completely.
- A failure late in a large batch followed by a confirmed rollback leaves none of that batch's new rows.
- Direct single-event and bulk storage calls reject mismatched entry/metadata IDs without inserting any rows. Replaying a legacy row whose stored metadata event ID differs from its row ID reports a conflict and leaves it unchanged.
- Replaying a batch succeeds and changes nothing, including on rows that are reserved, failed, or already delivered.
- Each compared field rejects a change on its own, and each tolerated field leaves the stored value intact.
- Payloads differing only in whitespace or key order replay successfully. `1` vs `1.0`, and large numbers that differ only beyond floating-point precision, both conflict.
- A batch mixing new events with a conflicting event leaves no new rows.
- With a separate business table: outer commit keeps both, outer rollback removes both, and a handled recoverable recording failure followed by successful savepoint rollback keeps earlier business writes and permits further work in that transaction.
- Concurrent overlapping batches, submitted in opposite orders at `READ COMMITTED`, each in an otherwise idle transaction with no pre-existing conflicting locks, all finish without deadlock and with correct final content. This includes running alongside consumers and the stale-reservation sweep; it is not a guarantee for arbitrary caller-owned transactions.
- Opposing locks from earlier recording calls in two caller-owned transactions can cause a deadlock. The recording call surfaces the database error without retrying internally; retrying the failed outer transaction succeeds with correct business and outbox content.
- A concurrent write of the same event inside a `REPEATABLE READ` caller transaction surfaces as a database error the caller can identify as a serialization failure, and retrying the transaction succeeds.
- Fault injection that loses a commit acknowledgment after PostgreSQL commits reports an uncertain outcome, not a confirmed rollback. Retrying the same batch with the original event IDs and compared content succeeds without adding or changing rows.
- Connection loss or failed savepoint recovery is surfaced without promising a usable outer transaction or confirmed rollback; the original failure and any recovery error are preserved.

**Also required:**

- Invalid input reaches no storage, and each error case above is asserted. Identity validation is exercised on every public entry-based recording path before database access.
- The database tests run in CI. Today no workflow runs them, so a green build says nothing about these guarantees.
- Default, no-default-feature, and all-feature builds pass.
- All-new, all-replay, and mixed batches have been measured at realistic sizes, so that any optimization work rests on data.
- `docs/outbox-spec.md` and the crate README describe bulk recording, event identity validation, the replay rules including number comparison, when a successful call becomes durable, batch-size guidance, the scope of deadlock avoidance, whole-transaction retries after deadlocks or serialization failures, and recovery from uncertain commit outcomes using stable event IDs and caller-owned business-operation recovery.
