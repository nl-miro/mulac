# Outbox bulk recording plan

## Status and change control

**Locked for implementation.** The requirements below fix the caller-visible behavior, supported scope, and acceptance criteria. This status does not certify implementation or test completion: delivery remains pending until the **Done when** requirements have evidence and the required CI and repository-owner signoff.

Changes to those requirements need an explicit plan revision reviewed and approved by the repository owner before implementation diverges. Internal implementation choices that satisfy the requirements remain open.

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
- Success means each event was newly inserted by this attempt or observed with matching immutable content during replay verification, including when a concurrent writer stored it first. Verification does not prevent concurrent deletion or guarantee simultaneous or continued presence of replayed rows at return or caller commit; new insertions remain atomic. There is no per-event result, so a new insert and a matching replay look the same.
- An empty batch succeeds without acquiring a pooled connection or writing rows. On the caller-connection path, transaction eligibility is still checked.
- Single-event recording behaves as a batch of one, under all the same rules.
- Batch size must not be limited by database statement limits.
- No maximum batch size is enforced. The documentation states the costs of a very large batch (memory use, one long transaction, a connection held throughout, and a delivery backlog, since consumers take the oldest scheduled first) and suggests a practical size.
- At `READ COMMITTED`, batches that hold no other locks must not deadlock with each other, whatever order their events are supplied in. This assumes the documented schema, without application-added locking triggers, constraints, or concurrent schema changes. It does not extend to transactions that already hold locks, including from earlier recording calls. Insertion order must be globally consistent across statement chunks; replay verification must not retain conflicting row locks.

### Recording in a caller's transaction

- Available with the `diesel` feature. All its work happens on the caller's connection, and it needs no outbox pool.
- After input validation, it requires an open transaction, however it was opened, and never begins or commits the outer transaction. With none open, it writes nothing and returns a distinct error. An aborted transaction or lost connection is a database failure, never this error.
- Its savepoint and transaction-eligibility handling must work independently of Diesel's tracked nesting depth, without parsing localized backend error messages. Raw-SQL `BEGIN`, including before any other statement or write, and nested caller savepoints are supported; merely wrapping the operation in Diesel's `transaction()` does not establish this guarantee.
- Nothing is durable until the caller commits. A failed batch's new rows are rolled back without undoing the caller's earlier work, provided that rollback succeeds. This holds however the transaction was opened.

### Validation

These rules apply on every recording path. Validate the complete batch before pool acquisition or database access. Invalid input takes precedence over connection state and stored-row conflicts; if several input defects exist, which applicable validation error is returned is unspecified. A valid caller-connection request, including an empty batch, then checks transaction eligibility. An invalid event rejects the whole batch, and nothing is written.

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

Accepted payloads are stored exactly as supplied. `content_type` grants no exemption: every payload must be a JSON object, whatever its content type says. Blank routing means `str::trim().is_empty()`; accepted routing keys are not trimmed. The metadata-string NUL check covers routing key, event type, source, and content type.

### Replay

Event ID remains the identity of a row. Message ID remains delivery metadata, so a new event ID creates a new row even if the message ID is reused.

Envelope conversion defaults an absent message ID to the event ID before comparison, so omission and an explicit message ID equal to the event ID replay equivalently. Other optional compared fields distinguish absence from presence, including an empty string. Metadata strings compare exactly without trimming or Unicode normalization.

When a batch contains an event ID that is already stored:

| Field                                                       | Rule                                                     |
| ----------------------------------------------------------- | -------------------------------------------------------- |
| Stored metadata                                             | Must be readable, and its event ID must equal the row ID |
| Payload                                                     | Must be equivalent (below)                               |
| Message ID, routing key, event type, content type, sequence | Must match                                               |
| Correlation ID, causation ID, source                        | May differ; the stored values are kept                   |
| Timestamps and delivery state                               | Ignored                                                  |

**Payload equivalence:** object keys compare decoded, in any order. Array order matters. Strings compare decoded, without Unicode normalization. Numbers compare by exact written form, so `1` and `1.0`, `1e2` and `1E2`, `1e2` and `1e+2`, and `0` and `-0` all conflict. Whitespace between tokens doesn't matter. Validation and comparison give the same results whatever `serde_json` features an application enables, and don't change how other code parses JSON. Numeric token spelling and decoded-key uniqueness must survive parsing: a lossy numeric representation, ordinary `serde_json::Value`, or a payload JSONB cast cannot be the authoritative comparison. This validator is not applied to reservation or delivery.

- A matching replay succeeds and leaves the stored row untouched, whatever its delivery state. It neither creates another row nor resets or reschedules delivery; existing publication and retry work proceeds unchanged.
- A conflicting replay fails the whole batch and never overwrites stored data.
- Replay verification introduces no additional row locks that block reservation, finalization, or stale sweeping of already committed replay rows, including in mixed batches and while a caller transaction stays open. It cannot remove locks or visibility restrictions established earlier by the caller. This is a database-lock guarantee, not protection from shared-pool exhaustion or unrelated resource contention; new caller-transaction rows remain invisible until commit.
- Replay protection lasts only while the row is stored. After deletion, the same event ID can insert again and repeat publication.
- Existing rows that break the new payload rules, have unreadable metadata, or have mismatched event IDs keep their current reservation and delivery behavior. Replaying one with a valid batch reports a conflict; an invalid batch is rejected by validation first.

### Failures

Recording has its own error type, separate from reservation and publishing, with the same cases in every build. Backend errors don't convert into it implicitly, so a failure of the caller's own commit or of its other statements is never reported as a recording error. A database failure observed by a recording call, including an aborted transaction or lost connection, is. It reports exactly these cases:

| Case                                                                           | Identifies                                     | Outcome of this attempt | Next step                     |
| ------------------------------------------------------------------------------ | ---------------------------------------------- | ----------------------- | ----------------------------- |
| Duplicate event ID                                                             | Event ID                                       | No rows added           | Fix input                     |
| Mismatched event ID                                                            | Both IDs                                       | No rows added           | Fix input                     |
| Invalid payload                                                                | Event ID                                       | No rows added           | Fix input                     |
| Invalid metadata                                                               | Event ID, field                                | No rows added           | Fix input                     |
| Replay conflict                                                                | Event ID, differing field or stored-row defect | No rows added           | Fix input or investigate      |
| No open transaction                                                            |                                                | No rows added           | Fix the call                  |
| Events neither inserted nor found afterwards, for example deleted concurrently | All event IDs                                  | No rows added           | May retry with the same event IDs; a retry can publish them again |
| No connection                                                                  | Source error                                   | No rows added           | May retry                     |
| Database failure, rolled back                                                  | Source error                                   | No rows added           | May retry                     |
| Database failure, uncertain                                                    | Source error                                   | Uncertain               | Retry with the same event IDs |

**Outcomes**

- "No rows added" requires that nothing was written or that rollback succeeded. Anything else, including any failed commit or failed rollback, is uncertain. Conservative uncertainty requires reconciliation or same-ID replay; a false "no rows added" invites replacement IDs and duplicate publication. Same-ID replay prevents reinsertion only while the original row is retained; retention must cover the supported retry and reconciliation horizon.
- Outcomes describe one attempt. After an uncertain attempt, a later negative result cannot establish that the earlier attempt added no rows. A successful retry while the original rows remain retained satisfies the recording obligation, but does not prove the earlier attempt's commit or publication history. After deletion, retry can insert and publish again; if retention is not assured, reconcile or escalate rather than claim duplicate-safe recovery. Inside a caller's transaction, how that transaction ends decides.
- If a retry after an uncertain attempt reports a conflict, or retries run out, "Fix input" doesn't apply: stop, keep the IDs and content, and escalate. The outcome remains uncertain.

**Retrying**

- Choose event IDs and replay-compared content once per logical operation, before the first attempt, and preserve or reconstruct them, together with unresolved outcome history, across handler reinvocations, delayed manual redrives, and process restarts. Never mint replacement IDs while the outcome is uncertain.
- Recording never retries internally. Callers may retry a limited number of times where the table allows it. In a caller's transaction, retrying means ending it and retrying all of it. Deadlocks can't be told apart from other database errors, so this rule covers all of them. Stable event IDs prevent reinsertion only while original rows are retained; business writes need their own retry safety.

**Inside a caller's transaction**

- Validation errors and replay conflicts should propagate by default. Continuing is an explicit exception: business writes commit without those events, giving up the outbox guarantee for them.
- Any other failure, apart from no open transaction, means abandoning the whole transaction, even if the database would allow it to continue. Ignoring this can make a commit report success after the database rolled back. Retry only after the failed transaction has ended, and retry all of it.
- Deadlocks can occur at `READ COMMITTED` when transactions already hold locks. A concurrent write of the same event can raise a serialization failure above `READ COMMITTED`.
- The caller owns the outcome of its own commit. A backend error after its transaction closure returned success is an uncertain commit; if the closure failed, the transaction didn't commit. Replaying outbox events doesn't make business writes idempotent, so after an uncertain commit the caller must reconcile or safely retry the complete business operation.

**Error content**

- Recording errors implement `Error + Send + Sync + 'static`. Public variants and auxiliary types are feature-independent; retained backend errors remain inspectable through `Error::source()` and downcasting without making Diesel an unconditional dependency.
- If a rollback fails, the uncertain error retains the rollback error, and `source()` exposes it; the original failure need not be retained. Any other failed recovery step or diagnostic probe retains the error that step observed. Outcome classification follows observed transaction phases, not merely the final Diesel error variant.
- Crate-authored diagnostic messages and fields never quote payload content, including offending object keys. Backend-originated diagnostics are an explicit exception: retained source errors are unchanged and never reduced to text, and their bounded rendering in `Display` may contain sensitive database-reported values. Truncation is not redaction; applications must treat source inspection and rendered backend text as sensitive.
- Error text shows the event ID, known metadata field or fixed defect category, and outcome where they apply. Both crate-authored details and rendered backend messages are bounded; long messages and ID lists are truncated in display only. Complete sources and ID lists remain available as data. The bound applies to `Display`, not `Debug`, full source inspection, or complete ID lists; these may be sensitive and unbounded.

### Constraints

- Bulk recording is a required, object-safe storage-port operation. Single-event recording delegates to the same batch primitive with one entry; no default bulk implementation loops over independently committed single-event calls.
- The recording port remains unsealed and implementable for test doubles, but only the supplied Diesel PostgreSQL repository is a supported production recording store. Third-party production recording stores are unsupported; custom implementations of the other ports are unaffected.
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
- Third-party production recording stores are unsupported from this release; the unsealed recording port remains usable for test doubles, which must adopt the required batch operation and recording error type.
- New validation rejections apply to single-event callers too, with no opt-out. They previously succeeded, except: sequence bounds and, through the recorder, blank routing keys (already rejected), and NUL in metadata or raw NUL in payloads (previously database errors). Duplicate IDs within a batch apply only to the new batch calls.
- Producers of non-object payloads have no direct path: unchanged they are rejected, and wrapped they conflict with their stored rows and change the bytes consumers receive. Inventory retryable and exhausted source events and retained requests that may reuse old IDs, not just pending outbox rows. Finish old-format processing before cutover or retain an explicit old-format recovery procedure, and coordinate with consumers before upgrading. Deleting or rewriting existing outbox rows is not a safe conflict repair.
- Conflicting replays previously succeeded and now fail, including on existing rows with mismatched event IDs or unreadable metadata.
- Producers that rebuild an event on retry with regenerated payload fields (such as a timestamp), fresh message IDs or sequences, or differently written numbers will now conflict on every retry. Audit them before upgrading. Any later deploy changing an event's payload shape, routing, or content type does the same to in-flight redeliveries. A fresh event ID on retry causes no conflict but creates another row that can publish a duplicate.
- Event subscribers that record must expect permanent conflicts. With TwoPhased eventing, propagating one fails the event and retries it with the same event ID and payload: earlier subscribers re-run, later ones never run, and the exhausted event stays Failed. With Direct eventing, the error returns to the caller with no retry, and each command execution assigns new event IDs. A subscriber that uses the delivered event ID as its outbox event ID therefore gets replacement IDs if the caller re-dispatches the command: after an uncertain outcome this is not a same-ID retry and can publish a duplicate. Such a subscriber must take its outbox event IDs and replay-compared content from something stable across command retries, or the caller must reconcile or escalate instead of re-dispatching.
- During a rolling upgrade, old instances accept conflicts that new ones reject, so failures look intermittent. A version rollback needs no schema rollback; it does not reverse already published bytes or restore the stricter recording guarantees on old instances.

## Done when

**Shown by CI tests on real PostgreSQL, with tables from `docs/outbox_entries.sql`. Blocking concurrency tests must show actual contention; nonblocking and deletion-race tests must demonstrate their intended overlap and observable state changes.**

Recording

- Batches above 65,535 events record completely, and empty batches write nothing, on every path. On the recorder and storage-port paths, an empty batch succeeds with the pool unavailable. The caller-connection path, including an empty batch, works with no outbox pool available to it.
- Each validation and equivalence rule is asserted individually on every path whose input can express it (the mismatched-ID rule only through the storage port); input-validation rejections reach no database, while replay-equivalence conflicts leave no new rows after confirmed rollback. Include validation before unavailable/no-transaction connections; omitted versus explicit default message IDs; absent versus empty optional fields; escaped-equivalent duplicate keys; valid surrogate pairs and lone halves; 127/128 mixed-container boundaries; extreme integer/exponent spellings and invalid number grammar; BOM/non-JSON whitespace; and object keys resembling parser-internal tokens. The crate does not enable `serde_json`'s `arbitrary_precision`, and the same corpus runs in separate package-isolated invocations for the baseline and downstream combinations enabling `arbitrary_precision`, `preserve_order`, and `raw_value`, with resolved features recorded. Also cover escaped NUL in keys/values, repeated keys in separate sibling objects, and broad shallow objects; feature unification must not make the baseline use the augmented configuration.
- Single-event recording through the recorder and the storage port is asserted against the same validation rules, replay rules, and applicable Failures cases as a batch of one.
- Each case in the Failures table is produced and asserted with its identifying data and outcome.
- Accepted payloads read back byte-identical.
- Force a database failure after an earlier insertion chunk has actually succeeded, and a replay conflict discovered after new-row writes; confirmed rollback leaves no new rows. A preflight validation failure alone does not satisfy this check.

Replay

- Replay changes nothing in any delivery state. Each compared field conflicts alone, and tolerated fields keep their stored values. A new event ID with a reused message ID creates a row.
- Existing rows that break the rules conflict on replay with a valid batch, stay unchanged, and keep their current reservation and delivery behavior.

Caller's transaction

- On a connection from a `DbPool`, with a business table: commit keeps both, rollback removes both, and a handled validation error or conflict keeps earlier writes and permits further work.
- For valid input with no open transaction, the call is rejected and writes nothing, even for an empty batch. An aborted transaction or lost connection isn't reported as this error.
- Inside a transaction begun with raw SQL, a handled conflict keeps earlier writes, which then commit, and a later rollback removes the batch. Test raw `BEGIN` before any other statement or write, nested caller savepoints, and valid empty batches in idle, active, aborted, and disconnected sessions. Transaction detection must not depend on message text or Diesel depth alone.

Concurrency

- Same-ID writers at `READ COMMITTED`: matching writers succeed with one row; differing writers give one commit and conflicts.
- Overlapping batches in opposite orders at `READ COMMITTED`, including across an insertion-chunk boundary, do not deadlock and leave correct content. A test-only unsorted control must demonstrate the intended deadlock; synchronization must not itself force the sorted implementation to deadlock.
- With a caller recording transaction held open, use separate eligible committed fixtures to prove reservation returns the target ID, matching-token completion succeeds, and a stale sweep actually transitions its target row. Exercise both all-replay and mixed batches with independent connections; newly inserted caller-transaction rows remain invisible until commit.
- Controlled deletion during replay verification demonstrates the documented observation-time success contract and the retention limit; it is not evidence of a partial commit.
- A deadlock forced through recording calls on both sides gives the victim a database error with its original source, and the other commits. Retrying the whole transaction succeeds. In the `REPEATABLE READ` case, establish the caller snapshot before the competing writer commits and assert the retained backend error is `SerializationFailure`, not an arbitrary timeout/error; retrying the whole transaction succeeds.

Failures

- Connection loss during a batch returns an error reporting uncertain, on every path.
- A rollback that fails after a healthy conflict or database failure reports uncertain, including inside a caller's transaction. Inject failures at savepoint establishment, release, and rollback, asserting that the failing step's backend error is retained; no failed eligibility diagnostic becomes a false no-open-transaction result. A failed pooled commit reports uncertain.
- Crate-authored diagnostics contain no payload content; test them separately from the documented backend-originated exception. Display stays bounded for large batches and long backend messages; full identifier lists and retained source errors remain inspectable.
- An uncertain attempt followed by a conflict or retry exhaustion remains uncertain, asserted against the handler example required below; missing rows never authorize replacement IDs.

**Also required**

- A dedicated PostgreSQL job runs all outbox database suites, including ignored existing tests, on every pull request. Use an explicit required-suite inventory and run both ordinary and ignored eligible database cases, without requiring unrelated ignored AMQP tests. Verify database preflight and discovered/executed counts per suite; missing suites, zero cases, or skipped eligible cases fail the job. Once the job exists, it is a required status check on `main`, so a failure blocks merging.
- All suites use isolated per-test schemas or equivalent isolation with `docs/outbox_entries.sql` as their baseline, configuring pooled, direct, and observer connections consistently. Test-specific schema changes are explicit; fixtures never reset another test's tables. Blocking tests observe the relevant backend blocker/waiter relationship; nonblocking and deletion-race tests prove their controlled overlap/state transitions instead. All use independent control connections and bounded watchdogs, not timing or simultaneous starts alone.
- Manual check, with procedure and result recorded: a controlled protocol/connection fault withholds a successful COMMIT acknowledgment after committed rows are independently observed. Cover both pooled recording (a recording error) and a caller-owned business/outbox transaction (an application commit error), observing committed rows independently and reconciling before any business retry. Same-ID recording retry adds no rows while they remain retained. Cutting the connection after the client received success does not satisfy this check.
- CI builds `mulac-outbox` with default, no, and all features, plus API/HTTP adapter feature combinations without Diesel. Package-isolated library dependency checks confirm no Diesel when disabled. Downstream compile tests import only through `prelude`, exercise trait objects/test doubles, and match recording errors in Diesel and non-Diesel builds.
- The reasoning for the deadlock guarantee is written down and reviewed.
- The suggested batch size rests on recorded release-build measurements of end-to-end recording time and process peak memory for all-new and all-replay batches. State PostgreSQL version/configuration, payload distribution, batch sizes, and measurement boundaries; use a fresh process per sample, prepare fixtures and replay seed data outside the measured process, and state whether input allocation is included. Report client memory separately from database memory. No CI performance threshold applies.
- `docs/outbox-spec.md` and the crate README cover every caller-visible rule above, plus:
  - one handler example, compiled and exercised in CI, which the documents include or link to, with a bounded retry loop and application-owned error type that distinguishes recording, business/database, and caller commit failures before explicit handler/subscriber error conversion; record callback success/failure so outer rollback errors cannot replace the original callback failure or masquerade as commit errors; test that double failure, cover every error case, state how stable IDs/content and unresolved outcome history survive reinvocation/restart, and log event IDs on an uncertain outcome
  - recording can wait without limit on a concurrent uncommitted write of the same event; timeouts are the caller's, as is whether recording shares a pool with consumers. State how to set a database-side timeout on both the pooled and caller-connection paths, how a timeout appears in the Failures table, and how to identify the blocking session. A caller that stops waiting without receiving a result must treat the attempt as uncertain, and the connection may stay occupied until the database call ends
  - pooled recording inside a caller's transaction commits independently: a business rollback does not remove those committed outbox rows or prevent their later publication, and cannot retract messages already published
  - recovering from permanent replay conflicts, stating that the crate ships no redrive or repair API, so any requeue of an exhausted row is an application-owned database change whose required state and risks the docs must state. Cover at least these options and what each gives up: resubmit with the original replay-compared content if input validation and stored-row replay checks pass (needs that content; it neither repairs invalid stored rows nor restarts exhausted outbox publication); drop the event from the batch and continue (the omitted candidate is not recorded by this attempt and its outbox guarantee is lost; any existing same-ID row and its publication are unaffected); use a new event ID with new content (a separate message, which consumers cannot deduplicate against the original by event ID alone, and which may duplicate the logical message if the original was or will be published); leave an exhausted TwoPhased source event Failed and escalate (the source event stays blocked and later subscribers do not run). Distinguish exhausted source-event redrive (earlier subscribers repeat) from exhausted outbox-publication recovery (matching replay leaves attempts/status unchanged); investigate uncertainty by inspecting event IDs and compared content: matching retained rows establish current outbox presence, not business success or broker delivery; missing rows do not prove nonpublication, and a mixed result does not prove a partial commit. Coordinate retention and in-flight writers or preserve uncertainty and escalate
- A focused application-integration test, listed in the dedicated PostgreSQL job's required-suite inventory, verifies the documented subscriber behavior: permanent recording conflict repeats earlier subscribers, skips later ones, and leaves an exhausted TwoPhased source event Failed; Direct dispatch returns the explicitly mapped error without automatic retry. No kernel behavior change is required.
- None of `README.md`, `libs/outbox/README.md`, `libs/outbox/flow.md`, `docs/outbox-spec.md`, `docs/eventing-spec.md`, `docs/architecture-spec.md`, `docs/components.md`, `docs/contracts.md`, `docs/implementation-guide.md`, `docs/repo-tldr.md`, or any outbox port comment claims that the dispatcher is the only caller, that conflicts are always accepted, that success always means durable, or that custom recording stores are supported. Historical plans, checklists, and reviews under `docs/` are exempt.
- `CHANGELOG.md` covers every item under Compatibility under `[Unreleased]`.
- The items above not run by CI are named manual checks, closed by the repository owner on the delivering pull request. That pull request records the tested commit and the manual procedure and result, links the deadlock reasoning and batch-size measurements, and records the owner's review of documentation coverage.
