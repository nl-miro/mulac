# Outbox bulk recording plan

## Why

Applications often produce many outbox events from one piece of work. Recording currently handles one event at a time, which leaves three problems:

- **Partial batches.** A failure partway through a group of events leaves some recorded and some not. The caller has to work out which is which and recover.
- **No shared transaction with business data.** Recording uses its own pooled connection, so business writes and their outbox events can't commit or roll back together. That is the guarantee a transactional outbox exists to provide.
- **Silent replay conflicts.** Recording an existing event ID always succeeds and keeps the stored row, even if the new content differs. A producer bug or a reused ID goes unnoticed, and the changed message is never published.

Storage errors are also reduced to text, so callers can't reliably tell these failures apart.

## What

### Batch recording

- Batches can be recorded through the recorder (from envelopes), the storage port (from entries), and a caller's Diesel connection (from envelopes).
- A batch is all-or-nothing: its new rows commit together or not at all.
- Success means every event in the batch is stored with matching content, including when a concurrent writer stored it first. There is no per-event result, so a new insert and a matching replay look the same.
- An empty batch succeeds and does nothing, except that the no-open-transaction error below still applies.
- Single-event recording behaves as a batch of one, under all the same rules.
- Batch size must not be limited by database statement limits.
- No maximum batch size is enforced. The documentation states the costs of a very large batch (memory use, one long transaction, a connection held throughout, and a delivery backlog, since consumers take the oldest scheduled first) and suggests a practical size.
- At `READ COMMITTED`, batches that hold no other locks must not deadlock with each other, whatever order their events are supplied in. This does not extend to transactions that already hold locks, including from earlier recording calls.

### Recording in a caller's transaction

- Available with the `diesel` feature. All its work happens on the caller's connection, and it needs no outbox pool.
- It requires an open transaction, however it was opened, and never begins or commits one. With none open, it writes nothing and returns a distinct error. An aborted transaction or lost connection is a database failure, never this error.
- Nothing is durable until the caller commits. A failed batch's new rows are rolled back without undoing the caller's earlier work, provided that rollback succeeds. This holds however the transaction was opened.

### Validation

These rules apply on every recording path. An invalid event rejects the whole batch, and nothing is written.

| Input                                                                                    | Result   |
| ---------------------------------------------------------------------------------------- | -------- |
| The same event ID twice in one batch, even if identical                                  | Rejected |
| Entry ID differs from its metadata event ID                                              | Rejected |
| Payload that isn't exactly one RFC 8259 JSON object, with only JSON whitespace around it | Rejected |
| Duplicate key within one object, after decoding escapes                                  | Rejected |
| Lone surrogate escape, or more than 127 nested containers counting the outermost         | Rejected |
| Numbers of any magnitude, and `\u0000` escapes in a payload                              | Accepted |
| NUL character in a metadata string                                                       | Rejected |
| Blank routing key                                                                        | Rejected |
| Sequence outside 1 to `i64::MAX`                                                         | Rejected |
| No sequence                                                                              | Accepted |

Accepted payloads are stored exactly as supplied. `content_type` grants no exemption: every payload must be a JSON object, whatever its content type says.

### Replay

Event ID remains the identity of a row. Message ID remains delivery metadata, so a new event ID creates a new row even if the message ID is reused.

When a batch contains an event ID that is already stored:

| Field                                                       | Rule                                                     |
| ----------------------------------------------------------- | -------------------------------------------------------- |
| Stored metadata                                             | Must be readable, and its event ID must equal the row ID |
| Payload                                                     | Must be equivalent (below)                               |
| Message ID, routing key, event type, content type, sequence | Must match                                               |
| Correlation ID, causation ID, source                        | May differ; the stored values are kept                   |
| Timestamps and delivery state                               | Ignored                                                  |

**Payload equivalence:** object keys compare decoded, in any order. Array order matters. Strings compare decoded, without Unicode normalization. Numbers compare by exact written form, so `1` and `1.0`, `1e2` and `1E2`, `1e2` and `1e+2`, and `0` and `-0` all conflict. Whitespace between tokens doesn't matter. Validation and comparison give the same results whatever `serde_json` features an application enables, and don't change how other code parses JSON.

- A matching replay succeeds and leaves the stored row untouched, whatever its delivery state. It does not trigger another publication.
- A conflicting replay fails the whole batch and never overwrites stored data.
- Replaying, even in a long open caller transaction, must not delay consumers or the sweep on that row.
- Replay protection lasts only while the row is stored. After deletion, the same event ID inserts and publishes again.
- Existing rows that break the new payload rules, have unreadable metadata, or have mismatched event IDs keep their current reservation and delivery behavior. Replaying one with a valid batch reports a conflict; an invalid batch is rejected by validation first.

### Failures

Recording has its own error type, separate from reservation and publishing, with the same cases in every build. Backend errors don't convert into it implicitly, so a failure of the caller's own transaction is never reported as a recording error. It reports exactly these cases:

| Case                                                                           | Identifies                                     | Outcome of this attempt | Next step                                |
| ------------------------------------------------------------------------------ | ---------------------------------------------- | ----------------------- | ---------------------------------------- |
| Duplicate event ID                                                             | Event ID                                       | No rows added           | Fix input                                |
| Mismatched event ID                                                            | Both IDs                                       | No rows added           | Fix input                                |
| Invalid payload                                                                | Event ID                                       | No rows added           | Fix input                                |
| Invalid metadata                                                               | Event ID, field                                | No rows added           | Fix input                                |
| Replay conflict                                                                | Event ID, differing field or stored-row defect | No rows added           | Fix input or investigate                 |
| No open transaction                                                            |                                                | No rows added           | Fix the call                             |
| Events neither inserted nor found afterwards, for example deleted concurrently | All event IDs                                  | No rows added           | May retry; they insert and publish again |
| No connection                                                                  | Source error                                   | No rows added           | May retry                                |
| Database failure, rolled back                                                  | Source error                                   | No rows added           | May retry                                |
| Database failure, uncertain                                                    | Source error                                   | Uncertain               | Retry with the same event IDs            |

**Outcomes**

- "No rows added" requires that nothing was written or that rollback succeeded. Anything else, including any failed commit or failed rollback, is uncertain. A false "uncertain" costs only a replay, while a false "no rows added" invites replacement IDs and duplicate publication.
- Outcomes describe one attempt. After an uncertain attempt on pooled recording, the batch may already be committed and published, and stays uncertain until a retry succeeds, whatever later attempts report. Inside a caller's transaction, how that transaction ends decides.
- If a retry after an uncertain attempt reports a conflict, or retries run out, "Fix input" doesn't apply: stop, keep the IDs and content, and escalate. The outcome remains uncertain.

**Retrying**

- Choose event IDs and replay-compared content once per logical operation, before the first attempt. Never mint replacement IDs while the outcome is uncertain.
- Recording never retries internally. Callers may retry a limited number of times where the table allows it. In a caller's transaction, retrying means ending it and retrying all of it. Deadlocks can't be told apart from other database errors, so this rule covers all of them. Stable event IDs make retries safe for outbox rows; business writes need their own safety.

**Inside a caller's transaction**

- Validation errors and replay conflicts should propagate by default. Continuing is an explicit exception: business writes commit without those events, giving up the outbox guarantee for them.
- Any other failure, apart from no open transaction, means abandoning the whole transaction, even if the database would allow it to continue. Ignoring this can make a commit report success after the database rolled back. Retry only after the failed transaction has ended, and retry all of it.
- Deadlocks can occur at `READ COMMITTED` when transactions already hold locks. A concurrent write of the same event can raise a serialization failure above `READ COMMITTED`.
- The caller owns the outcome of its own commit. A backend error after its transaction closure returned success is an uncertain commit; if the closure failed, the transaction didn't commit. Replaying outbox events doesn't make business writes idempotent, so after an uncertain commit the caller must reconcile or safely retry the complete business operation.

**Error content**

- Text and fields the crate writes never contain payload content. A retained source is the backend's error, unchanged and never reduced to text, so it may carry whatever the database reports. Error text includes the source's message.
- Error text shows the event ID, field, and outcome where they apply, and stays bounded at any batch size. Complete ID lists remain available as data.

### Constraints

- Bulk recording is part of the storage port.
- Recording is supported through the Diesel PostgreSQL repository only for now. Custom recording stores aren't supported, and the documentation says so. Custom implementations of the other ports are unaffected.
- Without `diesel`, the recorder, port, and recording error remain, but no supported store exists. Reservation, delivery, and HTTP surfaces keep working, and Diesel is absent from the dependency tree.
- The kernel is unchanged. Applications convert the recording error themselves.
- Public API is exposed only through the crate's `prelude`.
- The existing `outbox_entries` table stays as it is, with no migration or rewriting of existing rows.

## Out of scope

- Recording without Diesel, including custom stores and other database libraries.
- Publishing to the broker in batches. Delivery stays at-least-once, per entry.
- Automatic outbox recording from the kernel.
- An HTTP route for recording.
- Moving the Todo and Twitter test apps onto the core recorder.
- Guaranteeing that events are published in batch order. The outbox spec already disclaims ordering across entries, and callers should not infer it from a batch.

## Compatibility

This is a breaking change. `CHANGELOG.md` must cover every item below.

- Recording returns its own error type, so existing matches stop compiling, and applications that convert recording failures into the kernel's outbox error must convert them themselves.
- Custom recording stores are unsupported from this release.
- New validation rejections apply to single-event callers too, with no opt-out. They previously succeeded, except: sequence bounds and, through the recorder, blank routing keys (already rejected), and NUL in metadata or raw NUL in payloads (previously database errors). Duplicate IDs within a batch apply only to the new batch calls.
- Producers of non-object payloads have no direct path: unchanged they are rejected, and wrapped they conflict with their stored rows and change the bytes consumers receive. Drain in-flight redeliveries and coordinate with consumers before upgrading.
- Conflicting replays previously succeeded and now fail, including on existing rows with mismatched event IDs or unreadable metadata.
- Producers that rebuild an event on retry with regenerated payload fields (such as a timestamp), fresh message IDs or sequences, or differently written numbers will now conflict on every retry. Audit them before upgrading. Any later deploy changing an event's payload shape, routing, or content type does the same to in-flight redeliveries. A fresh event ID on retry causes no conflict but publishes a duplicate.
- Event subscribers that record must expect permanent conflicts. On the persistent path, propagating one fails the event and retries it: earlier subscribers re-run, later ones never run, and the exhausted event stays Failed. On the direct path, the error returns to the caller with no retry.
- During a rolling upgrade, old instances accept conflicts that new ones reject, so failures look intermittent. Rollback needs no data change.

## Done when

**Shown by CI tests on real PostgreSQL, with tables from `docs/outbox_entries.sql`. Concurrency tests must show that contention actually occurred.**

Recording

- Batches above 65,535 events record completely, and empty batches write nothing, on every path.
- Each validation and equivalence rule is asserted individually on its own, including the nesting limit, on every path whose input can express it (the mismatched-ID rule only through the storage port); rejections write nothing. The crate doesn't enable `serde_json`'s `arbitrary_precision`, and these tests also pass with it enabled.
- Each case in the Failures table is produced and asserted with its identifying data and outcome.
- Accepted payloads read back byte-identical.
- A failure on a large batch's last event, or a conflict in a mixed batch, leaves no new rows.

Replay

- Replay changes nothing in any delivery state. Each compared field conflicts alone, and tolerated fields keep their stored values. A new event ID with a reused message ID creates a row.
- Existing rows that break the rules conflict on replay with a valid batch, stay unchanged, and keep their current reservation and delivery behavior.

Caller's transaction

- On a connection from a `DbPool`, with a business table: commit keeps both, rollback removes both, and a handled validation error or conflict keeps earlier writes and permits further work.
- With no open transaction, the call is rejected and writes nothing, even for an empty batch. An aborted transaction or lost connection isn't reported as this error.
- Inside a transaction begun with raw SQL, a handled conflict keeps earlier writes, which then commit, and a later rollback removes the batch.

Concurrency

- Same-ID writers at `READ COMMITTED`: matching writers succeed with one row; differing writers give one commit and conflicts.
- Overlapping batches in opposite orders at `READ COMMITTED` don't deadlock, and leave correct content. The scenario must deadlock when events are written in supplied order.
- While a replay stays open, reservation returns that row, and completion and the sweep finish.
- A deadlock forced through recording calls on both sides gives the victim a database error with its original source, and the other commits. Retrying the whole transaction succeeds, and likewise after a `REPEATABLE READ` serialization failure.

Failures

- Connection loss during a batch returns an error reporting uncertain, on every path.
- A rollback that fails after a healthy conflict or database failure reports uncertain, including inside a caller's transaction. A failed commit reports uncertain.
- Crate-written error text and fields contain no payload content, and stay bounded for a large batch.

**Also required**

- CI runs all outbox database tests, including existing suites, on every pull request. A missing database or zero tests executed fails the run, and results don't depend on test order.
- Manual check, with procedure and result recorded: a commit succeeds and the connection is cut, the outcome reports uncertain, and retrying adds no rows.
- CI builds `mulac-outbox` with default, no, and all features, and the no-default build has no Diesel in its dependency tree.
- The reasoning for the deadlock guarantee is written down and reviewed.
- The suggested batch size rests on one recorded measurement of time and peak memory for all-new and all-replay batches, stating environment and payload size. No CI threshold applies.
- `docs/outbox-spec.md` and the crate README cover every caller-visible rule above, plus:
  - a compiled example of a handler with a bounded retry loop, covering every error case and a failed caller commit, logging the batch's event IDs on an uncertain outcome
  - recording can wait without limit on a concurrent uncommitted write of the same event; timeouts are the caller's, as is whether recording shares a pool with consumers
  - pooled recording inside a caller's transaction commits independently, so a business rollback leaves its events published
  - recovering from a permanent replay conflict and what each option gives up, re-driving an exhausted event, and resolving an uncertain batch by checking stored rows by event ID
- No project doc or port comment still claims that the dispatcher is the only caller, that conflicts are always accepted, that success always means durable, or that custom recording stores are supported.
