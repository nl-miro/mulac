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
- The operation is all-or-nothing on PostgreSQL: new rows from a batch commit together or not at all. See [Failure outcomes](#failure-outcomes) for what an error does and doesn't establish.
- An empty batch succeeds and does nothing.
- Batch size must not be limited by database statement limits. Very large batches must work the same way as small ones.
- No maximum batch size is enforced. The documentation states the costs of a very large batch (memory use and one long transaction) and suggests a practical size.
- Success returns no per-event result. A new insert and a matching replay look the same to the caller.
- At `READ COMMITTED`, batch calls that hold no other locks must not deadlock with each other, whatever order their events are supplied in, or with consumers and the stale-reservation sweep. This does not extend to caller transactions that already hold locks.

### Recording on a caller's connection

- Callers using Diesel can record a batch on a connection they own, alongside their business writes.
- **In an open transaction:** nothing is durable until the caller commits, and the transaction is never committed on the caller's behalf. A failed batch's new rows are undone without undoing the caller's earlier work, provided recovery succeeds.
- **With no open transaction:** recording commits by itself and behaves exactly like pooled recording, including uncertain commit outcomes.
- Applications that don't use Diesel must not need to depend on it.

### Failure outcomes

**What a failure establishes**

| Failure | What the caller knows |
|---|---|
| Validation rejection | Nothing was written |
| Failure with confirmed rollback | The batch added no rows |
| Commit acknowledgment lost while recording was committing | Uncertain: the whole batch may have committed and may already be published |
| Rollback or connection recovery failed | No rollback is promised, and the caller must not treat its transaction as usable |

**Retrying after an uncertain outcome**

- Retry with the same event IDs and the same replay-compared content. Never generate replacement event IDs to recover from this error.
- A matching replay accepts an already committed batch without adding rows.

**Failures inside a caller's open transaction**

| Failure | What the caller does |
|---|---|
| Validation error or replay conflict, with recovery succeeded | May continue or abandon the transaction. Continuing commits its business writes without those events, so the caller must decide that is acceptable. |
| Any other failure, including deadlocks and serialization failures | Must abandon the whole transaction, even if the database would allow it to continue. |

- Recording never retries internally.
- After any database failure, callers may retry the whole transaction a limited number of times. Stable event IDs make this safe for outbox rows, and it applies to business writes only if they are safe to retry too.
- This rule covers all database failures because deadlocks can't be reliably identified: the database driver reports them only as generic errors. Serialization failures can be identified.
- Deadlocks and serialization failures are possible here. Deadlocks can occur at `READ COMMITTED` when transactions already hold locks, and a concurrent write of the same event can raise a serialization failure above `READ COMMITTED`.
- The caller owns the outcome of its own commit. Replaying outbox events doesn't make business writes idempotent, so after an uncertain commit the caller must reconcile or safely retry the complete business operation.

### Validation

These rules apply to every recording path. An invalid event rejects the whole request before anything is written.

| Input | Result |
|---|---|
| The same event ID twice in one request, even if identical | Rejected |
| Entry ID differs from its metadata event ID | Rejected |
| Payload that isn't a JSON object | Rejected |
| Payload with a duplicate key at any level | Rejected |
| Blank routing key | Rejected |
| Sequence outside 1 to `i64::MAX` | Rejected |
| No sequence | Accepted |

Accepted payloads are stored exactly as supplied, without reformatting.

Recording from an envelope always produces matching IDs. Direct storage inputs must be checked rather than trusted, and no recording path may bypass that check.

### Replay

Event ID remains the identity of a row. Message ID remains delivery metadata, so a new event ID creates a new row even if the message ID is reused.

When a request contains an event ID that is already stored:

| Field | Rule |
|---|---|
| Stored metadata | Must be readable, and its event ID must equal the row ID |
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
- Rollback or connection recovery failure, retaining the recovery error
- Failure to obtain a connection

### Design constraints

- Bulk recording is part of the storage port, with no fallback that quietly records events one at a time. The port must state what its implementations guarantee.
- Public API is exposed only through the crate's `prelude`.
- No new repository type is needed; the existing storage repository takes on this responsibility.
- The existing `outbox_entries` table stays as it is, with no migration or rewriting of existing rows.
- Existing rows that break the new payload rules, have unreadable metadata, or have mismatched event IDs keep their current delivery behavior. Replaying one reports a conflict for that event.

## Out of scope

- Publishing to the broker in batches. Delivery stays at-least-once, per entry.
- Automatic outbox recording from the kernel.
- Moving the Todo and Twitter test apps onto the core recorder.
- Guaranteeing that events are published in batch order. The outbox spec already disclaims ordering across entries, and callers should not infer it from a batch.

## Compatibility

This is a breaking change, to be called out in the changelog:

- Custom storage implementations must support bulk recording.
- New error cases break exhaustive matches on the error type.
- Recording's database errors no longer arrive as `Storage`. Code that matches on `Storage` still compiles but behaves differently, so this needs a specific migration note.
- Payloads that aren't JSON objects or contain duplicate keys, and conflicting replays, previously succeeded and now fail.
- Direct storage calls now also reject blank routing keys and mismatched event IDs.
- Replaying an existing row with mismatched event IDs or unreadable metadata now conflicts.

## Done when

**All of these are shown against a real PostgreSQL database:**

- Batches below, at, and well above any internal statement boundary are recorded completely.
- A failure late in a large batch followed by a confirmed rollback leaves none of that batch's new rows.
- Direct single-event and bulk storage calls reject mismatched event IDs without inserting any rows.
- Replaying a batch succeeds and changes nothing, including on rows that are reserved, failed, or already delivered.
- Each compared field rejects a change on its own, and each tolerated field leaves the stored value intact.
- Payloads differing only in whitespace or key order replay successfully. `1` vs `1.0`, and large numbers that differ only beyond floating-point precision, both conflict.
- Replaying an existing row that breaks the payload rules, has unreadable metadata, or has mismatched event IDs reports a conflict for that event and leaves the row unchanged.
- A batch mixing new events with a conflicting event leaves no new rows.
- With a separate business table in an open transaction: outer commit keeps both, outer rollback removes both, and a handled validation error or replay conflict keeps earlier business writes and permits further work.
- On a caller's connection with no open transaction: a successful batch is committed by the call, and a failed batch leaves no new rows.
- Concurrent overlapping batches, supplied in opposite orders at `READ COMMITTED` and holding no other locks, all finish without deadlock and with correct final content, including alongside consumers and the stale-reservation sweep.
- Two caller transactions holding opposing locks from earlier recording calls can deadlock. Recording surfaces the database error without retrying, and retrying the whole failed transaction succeeds with correct business and outbox content.
- A concurrent write of the same event inside a `REPEATABLE READ` caller transaction surfaces as a database error the caller can identify as a serialization failure, and retrying the whole transaction succeeds.
- Connection loss or failed recovery is surfaced without promising a usable transaction or a confirmed rollback, and the recovery error is preserved.

**Also required:**

- Invalid input reaches no storage, including payloads with duplicate keys at any level, and each error case above is asserted. Event ID validation is exercised on every public path that accepts entries, before database access.
- A lost commit acknowledgment reports an uncertain outcome, and retrying the same batch adds or changes no rows. This depends on cutting a connection right after commit, so it can be a documented manual check rather than a CI test.
- The database tests run in CI. Today no workflow runs them, so a green build says nothing about these guarantees.
- Default, no-default-feature, and all-feature builds pass.
- All-new, all-replay, and mixed batches have been measured at realistic sizes, so that any optimization work rests on data.
- `docs/outbox-spec.md` and the crate README describe:
  - bulk recording and batch-size guidance
  - validation, including event IDs and duplicate payload keys
  - the replay rules, including number comparison
  - when a successful call becomes durable
  - the scope of deadlock avoidance
  - which failures allow a caller to continue its transaction, and when to retry the whole transaction
  - recovery from uncertain commit outcomes
