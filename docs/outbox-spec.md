# Outbox Specification

See [item failure isolation](item-failure-isolation.md) for the implemented
transactional reservation, item-level panic containment, and recovery limits.

## Overview

The outbox crate provides a durable outbound message buffer that decouples internal event processing from external transport publication. It accepts event envelopes from the event dispatcher, stores outbound work atomically, and exposes stored entries to consumers that publish each entry to a broker or other external transport. Applications and the event dispatcher can record into the outbox, singly or in batches, and a batch can be recorded inside the caller's own database transaction so that business writes and outbox events commit or roll back together. See [Recording](#recording).

The outbox guarantees at-least-once delivery to the external transport boundary. Reservation, retry scheduling, failure recording, and stale-reservation recovery are built in. Duplicate publication is possible and must be tolerated by downstream systems.

## Architecture

Private layers separate domain status/timeout rules, application envelopes and
ports, the execution core (consumer and reservation spec), Diesel persistence, and HTTP adaptation. Only
`prelude` is public. `DieselPgOutboxRepository::new(pool)` implements all four storage ports.
`OutboxRecorder` and `ReservationSweeper` take their trait-object ports; `OutboxConsumer<R>` holds one `Arc<R>`
implementing reserve/process. `AsyncOutboxService` wraps the consumer and erases `R` at the
blocking-task boundary. There is no synchronous HTTP service, so callers without
Tokio use the consumer directly. Synchronous code supports borrowed repositories.
Transport adapters retain their independent acknowledgment/publication behavior.
See [ADR-012](adr/012-unified-queue-repositories.md) and
[ADR-014](adr/014-merge-sibling-execution-bridges.md).

## Components

### OutboxRecorder

Use case: accept events and record outbound work. See [Recording](#recording) for the complete contract.

- Receives `NewOutboxEnvelope`s from the event dispatcher or from the application; no event ID generation is performed inside the outbox.
- Requires `event_id` and `routing_key` in metadata. `event_id` becomes the `OutboxEntry.id` and the default outbound `message_id`.
- Validates the whole batch (IDs, metadata, JSON object payloads) before any storage access, converts it into `NewOutboxEntry`s, and hands it to the store as one atomic batch. `record` is a batch of one.
- Stores exactly one `outbox_entries` row for each event ID. Recording an event ID that is already stored succeeds only if its immutable content matches; a conflicting replay is rejected and never overwrites the stored row.
- Returns success only after the batch is stored (or, for replays, found stored with matching content). Failures are `RecordingError`s that say whether rows may have been added.

### OutboxConsumer<R>

Use case: reserve stored outbound work and publish it to an external transport.

- Reserves up to `limit` eligible entries atomically.
- Converts each `OutboxEntryEnvelope` into an `OutboundMessageEnvelope`.
- Publishes each outbound message via `OutboxPublisherPort` and waits for broker acceptance.
- Marks each entry `Completed` after broker acceptance.
- Marks each entry `Failed` on processing or completion error, including recoverable unwinding panics.
- Conversion failures also become `Failed`; the automatic consumer never calls `dead()`.
- Returns all errors collected across the batch; does not abort early.

### ReservationSweeper

Use case: release stale reservations back to the eligible queue.

- Finds all `Reserved` entries whose `reserved_at` is older than the configured timeout.
- Sets all timed-out rows to `Failed`, including exhausted rows, and clears reservation fields. Only entries below the attempt cap can be reserved again.
- Does not increment the attempt counter; reservation increments attempts, not failure or sweeping.

### DieselPgOutboxRepository: recording

Implements `OutboxStorePort`; it is the only supported production recording store. A batch runs in its own transaction on one pooled connection (begun and committed through Diesel's transaction manager, so a connection with an unconfirmed commit or rollback is never reused). Event IDs are sorted once and inserted in that global order, in multi-row `INSERT … ON CONFLICT (id) DO NOTHING` statements of at most 5,000 rows. Skipped IDs are then read back, without row locks, and compared with the batch. `record_batch_on_connection` uses the same engine inside the caller's transaction, through a savepoint.

### DieselPgOutboxRepository: processing and recovery

Implements `OutboxReservePort`, `OutboxProcessPort`, and `OutboxSweepPort`.

- **reserve**: `SELECT … FOR UPDATE SKIP LOCKED`, then updates status to `Reserved`, assigns `reservation_id` (UUID v7), increments `attempts`, and returns full envelopes.
  Reservation, raw JSON loading, per-row decoding, and malformed-row failure writes share one transaction. Only healthy envelopes are returned after commit; the candidate limit is not refilled.
- **completed**: Updates status to `Completed`, clears `reservation_id` and `reserved_at`, sets `processed_at`.
- **failed**: records `Failed`, even at exhaustion, and clears reservation fields. The reservation attempt cap controls retry eligibility. Failure scheduling uses database time; see [item failure isolation](item-failure-isolation.md) for delay formulas.
- **dead**: Transitions directly to `Dead`, records the failure reason when available, and clears reservation fields without scheduling retry.

### AmqpPublisher

Implements `OutboxPublisherPort` for AMQP.

- Resolves the AMQP exchange and routing key from `OutboundMessageEnvelope` metadata.
- Publishes the payload and required message properties.
- Waits for publisher confirmation / broker acceptance before returning success.
- Maps broker unavailability, negative confirmations, and channel failures to retriable `OutboxError::Transport` errors.

### ConfirmedAmqpPublisher

A strict `OutboxPublisherPort` for AMQP, also behind the `amqp` feature.
`AmqpPublisher` keeps its behavior; choose this publisher when a message must
not count as published unless the broker routed and confirmed it.

- `ConfirmedAmqpPublisher::new(url, exchange)` does not connect and does not
  validate the URL; lapin parses it on the first connection. Environment
  parsing and URL policy stay with the application.
- The connection and channel are opened lazily, with `confirm_select` once per
  connection, reused after successful publishes and dropped after any failure,
  so the next publish reconnects.
- Every message is published with `mandatory: true` and persistent delivery
  (`delivery_mode` 2). Properties: `message_id`, optional `correlation_id`,
  the content type (default `application/json`) and the headers `event_id`,
  `event_type` and optional `causation_id` and `source`.
- Connecting and publishing are bounded by a timeout, 10 seconds by default
  (`ConfirmedAmqpPublisher::DEFAULT_TIMEOUT`, `with_timeout`).
- `publish` never creates a runtime. It needs a current Tokio runtime handle
  and must run on that runtime's blocking pool, as
  `AsyncOutboxService` does.
- Errors are fixed strings without broker diagnostics:

| Outcome                                                       | Error                                                                    |
| ------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Connection, channel or publish failure, or an unparseable URL | `transport error: RabbitMQ connection or publication failed`             |
| Timeout                                                       | `transport error: RabbitMQ publication timed out`                        |
| No current runtime                                            | `transport error: RabbitMQ publication requires the application runtime` |
| `Ack` with a returned (unroutable) message                    | `routing error: RabbitMQ returned an unroutable message`                 |
| `Nack`                                                        | `publish error: RabbitMQ negatively acknowledged publication`            |
| Confirms not active                                           | `publish error: RabbitMQ did not confirm publication`                    |

The broker test is ignored by default; run it with
`AMQP_URL_TEST=amqp://… cargo test -p mulac-outbox --features amqp -- --ignored`
from `libs/`.

## Timeout recovery contract

Sweeps run in one transaction and take one PostgreSQL `transaction_timestamp()`.
Only `Reserved` rows with non-null `reserved_at < database_time - timeout` are
selected, with blocking row locks held through the update. A pure internal policy
calculates each outcome; one set-based update clears reservation fields, preserves
existing errors and appends the timeout reason. Errors roll back the entire sweep.
Attempts do not change. The default retry limit is 6; configure the same limit on
consumer and sweeper. No recovery worker is started automatically.

Retry delay is `min(max(attempts, 1) × 30 seconds, 120 seconds)`. Exhausted
rows keep their previous `scheduled_at`; retryable rows use database time + delay.
Explicit failure paths now use database time; manual outbox `dead()` remains separate.

## Models

### OutboxEntry (domain)

The core domain model for stored outbound work.

| Field            | Type                    | Description                                      |
| ---------------- | ----------------------- | ------------------------------------------------ |
| `id`             | `Uuid`                  | UUID v7 primary key, sourced from event metadata |
| `status`         | `OutboxStatus`          | Lifecycle state                                  |
| `payload`        | `String`                | Serialised event payload                         |
| `meta`           | `OutboxEntryMetadata`   | JSONB-serialised routing metadata                |
| `scheduled_at`   | `DateTime<Utc>`         | Earliest eligible-for-reservation time           |
| `attempts`       | `i32`                   | Number of times reserved for publishing          |
| `reservation_id` | `Option<Uuid>`          | Ownership token assigned on reservation          |
| `reserved_at`    | `Option<DateTime<Utc>>` | Timestamp of last reservation                    |
| `received_at`    | `DateTime<Utc>`         | Timestamp of initial storage                     |
| `updated_at`     | `DateTime<Utc>`         | Timestamp of last state change                   |
| `processed_at`   | `Option<DateTime<Utc>>` | Timestamp of broker acceptance                   |
| `last_error`     | `Option<String>`        | Last failure reason for operations               |

### OutboxStatus (domain)

| Variant     | Code | Description                                                                |
| ----------- | ---- | -------------------------------------------------------------------------- |
| `Received`  | `0`  | Stored, eligible for reservation                                           |
| `Reserved`  | `2`  | Claimed by an outbox consumer                                              |
| `Failed`    | `4`  | Failure retained for inspection; retry eligible only below the attempt cap |
| `Completed` | `5`  | Broker accepted the outbound message                                       |
| `Dead`      | `7`  | Legacy/manual terminal status; never an automatic failure outcome          |
| `Archive`   | `8`  | Archived (no further processing)                                           |

Numeric codes use intentional gaps (1, 3, 6 are reserved for future insertion without renumbering). See [ADR-002](adr/002-status-code-gaps.md).

### NewOutboxEnvelope (application)

Event-dispatcher-facing input before storage. The event dispatcher must provide the stable event identity; the outbox does not generate it.

| Field      | Type                | Description                               |
| ---------- | ------------------- | ----------------------------------------- |
| `payload`  | `String`            | Event payload before outbound publication |
| `metadata` | `NewOutboxMetadata` | Required event and routing metadata       |

### NewOutboxMetadata (application)

Metadata accepted from the event dispatcher.

| Field            | Type             | Description                                          |
| ---------------- | ---------------- | ---------------------------------------------------- |
| `event_id`       | `Uuid`           | Stable event ID; becomes `OutboxEntry.id`            |
| `message_id`     | `Option<Uuid>`   | Optional outbound message ID; defaults to `event_id` |
| `correlation_id` | `Option<Uuid>`   | Cross-service correlation chain                      |
| `causation_id`   | `Option<Uuid>`   | Command/message that caused this event               |
| `event_type`     | `String`         | Event type name                                      |
| `routing_key`    | `String`         | Destination routing key; required at acceptance      |
| `source`         | `Option<String>` | Originating service identifier                       |
| `content_type`   | `Option<String>` | Payload content type, for example `application/json` |

### OutboxEntryMetadata (application)

Decoded metadata attached to a stored or reserved `OutboxEntry`. `message_id` is always present after recording.

| Field            | Type             | Description                                          |
| ---------------- | ---------------- | ---------------------------------------------------- |
| `event_id`       | `Uuid`           | Original event identifier                            |
| `message_id`     | `Uuid`           | Stable outbound message ID / idempotency key         |
| `correlation_id` | `Option<Uuid>`   | Cross-service correlation chain                      |
| `causation_id`   | `Option<Uuid>`   | Command/message that caused this event               |
| `event_type`     | `String`         | Event type name                                      |
| `routing_key`    | `String`         | Destination routing key                              |
| `source`         | `Option<String>` | Originating service identifier                       |
| `content_type`   | `Option<String>` | Payload content type, for example `application/json` |

### OutboxEntryEnvelope (application)

Read-side envelope returned to consumers after reservation.

| Field      | Type                  | Description             |
| ---------- | --------------------- | ----------------------- |
| `message`  | `OutboxEntry`         | The stored domain model |
| `metadata` | `OutboxEntryMetadata` | Decoded metadata        |

### NewOutboxEntry (domain)

Write-side struct passed to `OutboxStorePort` for insertion. Contains only the fields supplied or derived during acceptance; database-managed fields (`attempts`, `reservation_id`, `reserved_at`, `updated_at`, `processed_at`, `last_error`) are absent.

| Field          | Type                  | Description                                        |
| -------------- | --------------------- | -------------------------------------------------- |
| `id`           | `Uuid`                | UUID v7, sourced from `NewOutboxMetadata.event_id` |
| `payload`      | `String`              | Serialised event payload                           |
| `meta`         | `OutboxEntryMetadata` | JSONB-serialised routing metadata                  |
| `scheduled_at` | `DateTime<Utc>`       | Earliest eligible-for-reservation time             |
| `received_at`  | `DateTime<Utc>`       | Timestamp of initial storage                       |

### OutboundMessageEnvelope (application)

Transport-facing outbound message after reservation and transformation.

| Field      | Type                  | Description                          |
| ---------- | --------------------- | ------------------------------------ |
| `payload`  | `Vec<u8>`             | Bytes sent to the external transport |
| `metadata` | `OutboxEntryMetadata` | Routing and message properties       |

### ReservableOutboxSpec (outbox_consumer)

| Field          | Default | Description                                        |
| -------------- | ------- | -------------------------------------------------- |
| `limit`        | —       | Maximum number of entries to reserve per call      |
| `max_attempts` | `6`     | Entries at or above this attempt count are skipped |

Builds query criteria: `StatusIn([Received, Failed])`, `ScheduledBeforeNow`, `MaxAttempts(max_attempts)`, `OrderByScheduledAtAsc`.

### StaleReservationSpec (stale_reservation_sweep)

| Field          | Default | Description                                                      |
| -------------- | ------- | ---------------------------------------------------------------- |
| `timeout`      | —       | Staleness threshold; entries reserved longer than this are swept |
| `max_attempts` | `6`     | Max attempts forwarded to the retry policy                       |

### OutboxError (application)

| Variant              | Description                                     |
| -------------------- | ----------------------------------------------- |
| `Storage`            | Storage adapter error                           |
| `Routing`            | Missing or unresolvable routing key             |
| `Serialization`      | Payload cannot be converted for the transport   |
| `Transport`          | Transport adapter or broker acceptance error    |
| `Reservation`        | Reservation query or update failure             |
| `Publish`            | Outbound publish orchestration failure          |
| `MissingReservation` | Entry returned without a `reservation_id`       |
| `Conversion`         | Envelope-to-outbound-message conversion failure |

Recording does not use `OutboxError`: it returns `RecordingError` (see [Failures](#failures)).

## Ports

### OutboxStorePort

```rust
fn record_batch(&self, entries: &[NewOutboxEntry]) -> Result<(), RecordingError>;
fn record(&self, entry: &NewOutboxEntry) -> Result<(), RecordingError>; // a batch of one
```

`record_batch` is required and all-or-nothing. The port is unsealed so applications can write test doubles; only the Diesel repository is a supported production store.

### OutboxReservePort

```rust
fn reserve(&self, spec: &ReservableOutboxSpec) -> Result<Vec<OutboxEntryEnvelope>, OutboxError>;
```

### OutboxProcessPort

```rust
fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), OutboxError>;
fn failed(&self, id: Uuid, reservation_id: Uuid, max_attempts: i32, reason: Option<String>) -> Result<(), OutboxError>;
fn dead(&self, id: Uuid, reservation_id: Uuid, reason: Option<String>) -> Result<(), OutboxError>;
```

### OutboxSweepPort

```rust
fn sweep(&self, spec: &StaleReservationSpec) -> Result<u64, OutboxError>;
```

### OutboxPublisherPort

```rust
fn publish(&self, envelope: OutboundMessageEnvelope) -> Result<(), OutboxError>;
```

## Database Schema

Table: `outbox_entries`

```sql
CREATE TABLE outbox_entries (
    id              UUID         PRIMARY KEY,
    status          INTEGER      NOT NULL,
    payload         TEXT         NOT NULL,
    meta            JSONB        NOT NULL,
    scheduled_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    attempts        INTEGER      NOT NULL DEFAULT 0,
    reservation_id  UUID,
    reserved_at     TIMESTAMPTZ,
    received_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    processed_at    TIMESTAMPTZ,
    last_error      TEXT
);
```

Recommended indexes:

- `(status, scheduled_at)` — reservation queries
- `(status, reserved_at)` — stale sweep queries
- `(meta ->> 'routing_key')` — optional operational lookup by destination
- `(meta ->> 'message_id')` — optional downstream idempotency lookup

The primary key is UUID (not BIGSERIAL) because the event ID is generated before outbox storage and is reused as the idempotency key for subscriber retries. See [ADR-003](adr/003-uuid-v7-identifiers.md).

## Recording

Applications record outbound events through three entry points. All of them apply
the same rules, and recording one event is a batch of one.

| Entry point                                     | Takes                | Runs on                                                         |
| ----------------------------------------------- | -------------------- | --------------------------------------------------------------- |
| `OutboxRecorder::record_batch` (and `record`)   | `NewOutboxEnvelope`  | the store's own pooled connection, in its own transaction       |
| `OutboxStorePort::record_batch` (and `record`)  | `NewOutboxEntry`     | the same                                                        |
| `record_batch_on_connection` (`diesel` feature) | `NewOutboxEnvelope`  | the caller's Diesel connection, inside the caller's transaction |

### What a batch guarantees

- **All or nothing.** A batch's new rows commit together or not at all.
- **What success means.** Each event was newly inserted by this call, or was
  already stored with matching immutable content when the call checked it,
  including when a concurrent writer stored it first. There is no per-event
  result, so a new insert and a matching replay look the same. Success does not
  promise that a replayed row still exists at return or at the caller's commit:
  a concurrent deletion can remove it afterwards.
- **Empty batches** succeed without acquiring a pooled connection or writing
  rows. On the caller-connection path an empty batch still needs an open
  transaction.
- **No maximum batch size** is enforced, and database statement limits do not
  limit it (large batches are written in several statements). See
  [batch size](#batch-size).
- **No ordering.** Rows get a scheduling time as they are converted, and
  consumers take the oldest scheduled first, so batch order is not preserved.
  The outbox does not guarantee publication order, and callers must not infer one
  from a batch.
- **No deadlocks between batches** that hold no other locks, at `READ COMMITTED`,
  whatever order their events are supplied in. This assumes the documented
  schema without application-added locking triggers, constraints or concurrent
  schema changes, and it does not extend to transactions that already hold
  locks, including from earlier recording calls. The reasoning is in
  [outbox-recording-deadlock-reasoning.md](outbox-recording-deadlock-reasoning.md).
- **Supported stores.** Only the Diesel PostgreSQL repository is a supported
  production recording store. `OutboxStorePort` stays unsealed so applications
  can write test doubles, but third-party production recording stores are
  unsupported. Custom implementations of the reservation, processing, sweep and
  publisher ports are unaffected. Without the `diesel` feature the recorder, the
  port and `RecordingError` remain, but no supported store exists.

### Validation

These rules apply on every path. The whole batch is validated before a pooled
connection is acquired or the database is touched, and invalid input takes
precedence over connection state and stored-row conflicts. If several input
defects exist, which of the applicable validation errors is returned is
unspecified. A valid caller-connection request, including an empty batch, then
checks that a transaction is open. One invalid event rejects the whole batch, and
nothing is written.

| Input                                                                                    | Result   |
| ---------------------------------------------------------------------------------------- | -------- |
| The same event ID twice in one batch, even if identical                                  | Rejected |
| Entry ID differs from its metadata event ID (only expressible through the storage port)  | Rejected |
| Payload that isn't exactly one RFC 8259 JSON object, with only JSON whitespace around it | Rejected |
| Duplicate key within one object, after decoding escapes                                  | Rejected |
| Lone surrogate escape, or more than 127 nested containers counting the outermost         | Rejected |
| Numbers of any magnitude, and `\u0000` escapes in a payload                              | Accepted |
| NUL character in a metadata string (routing key, event type, source, content type)       | Rejected |
| Blank routing key (`str::trim().is_empty()`)                                             | Rejected |
| Sequence outside 1 to `i64::MAX`                                                         | Rejected |
| No sequence                                                                              | Accepted |

Accepted payloads are stored exactly as supplied. `content_type` grants no
exemption: every payload must be a JSON object, whatever its content type says.
Accepted routing keys are not trimmed.

### Replay

The event ID is the identity of a row. A new event ID creates a new row even if
its message ID is reused. Envelope conversion defaults an absent message ID to the
event ID before comparison, so omitting it and supplying the event ID replay
equivalently. The other optional compared fields distinguish absence from
presence, including an empty string, and metadata strings compare exactly,
without trimming or Unicode normalization.

When a batch contains an event ID that is already stored:

| Field                                                       | Rule                                                     |
| ----------------------------------------------------------- | -------------------------------------------------------- |
| Stored metadata                                             | Must be readable, and its event ID must equal the row ID |
| Payload                                                     | Must be equivalent (below)                               |
| Message ID, routing key, event type, content type, sequence | Must match                                               |
| Correlation ID, causation ID, source                        | May differ; the stored values are kept                   |
| Timestamps and delivery state                               | Ignored                                                  |

**Payload equivalence.** Object keys compare decoded, in any order. Array order
matters. Strings compare decoded, without Unicode normalization. **Numbers compare
by exact written form**, so `1` and `1.0`, `1e2` and `1E2`, `1e2` and `1e+2`, and
`0` and `-0` all conflict, and two integers that differ only in the last digit of
a huge number conflict too. Whitespace between tokens does not matter. Validation
and comparison give the same results whatever `serde_json` features an application
enables, and they do not change how other code parses JSON. They apply to
recording only, never to reservation or delivery.

- A matching replay succeeds and leaves the stored row untouched, whatever its
  delivery state. It neither creates another row nor resets or reschedules
  delivery; existing publication and retry work proceeds unchanged.
- A conflicting replay fails the whole batch and never overwrites stored data.
- Replay verification adds no row locks that block reservation, finalization or
  stale sweeping of already committed replay rows, including in mixed batches and
  while a caller's transaction stays open. It cannot remove locks or visibility
  restrictions the caller established earlier. This is a database-lock guarantee,
  not protection from shared-pool exhaustion. New rows written inside a caller's
  transaction stay invisible to consumers until it commits.
- Existing rows that break the payload rules, have unreadable metadata, or have a
  metadata event ID that differs from the row ID keep their current reservation
  and delivery behavior. Replaying one with a valid batch reports a conflict
  (`stored_metadata`, `stored_event_id` or `payload`); an invalid batch is rejected
  by validation first.

### Retention

Replay protection lasts only while the row is stored. After a row is deleted the
same event ID can insert again and repeat publication. Retention must cover the
supported retry and reconciliation horizon of every producer, and the outbox does
not delete rows itself: whatever removes completed rows belongs to the
application.

### Failures

Recording has its own error type, `RecordingError`, separate from reservation and
publishing, with the same cases in every build. Backend errors do not convert into
it implicitly, so a failure of the caller's own commit or of its other statements
is never reported as a recording error. A database failure observed by a
recording call, including an aborted transaction or a lost connection, is.
`RecordingError::outcome()` returns `RecordingOutcome::NoRowsAdded` or
`RecordingOutcome::Uncertain`.

| Case                                                                    | `RecordingError`        | Identifies                                                          | Outcome of this attempt | Next step                                                         |
| ----------------------------------------------------------------------- | ----------------------- | ------------------------------------------------------------------- | ----------------------- | ----------------------------------------------------------------- |
| Duplicate event ID                                                      | `DuplicateEventId`      | Event ID                                                            | No rows added           | Fix input                                                         |
| Mismatched event ID                                                     | `MismatchedEventId`     | Both IDs                                                            | No rows added           | Fix input                                                         |
| Invalid payload                                                         | `InvalidPayload`        | Event ID                                                            | No rows added           | Fix input                                                         |
| Invalid metadata                                                        | `InvalidMetadata`       | Event ID, field                                                     | No rows added           | Fix input                                                         |
| Replay conflict                                                         | `ReplayConflict`        | Event ID, differing field or stored-row defect                      | No rows added           | Fix input or investigate                                          |
| No open transaction                                                     | `NoOpenTransaction`     |                                                                     | No rows added           | Fix the call                                                      |
| Events neither inserted nor found afterwards (for example, deleted concurrently) | `MissingEvents` | All event IDs                                                       | No rows added           | May retry with the same event IDs; a retry can publish them again |
| No connection                                                           | `NoConnection`          | Source error                                                        | No rows added           | May retry                                                         |
| Database failure, rolled back                                           | `DatabaseRolledBack`    | Source error                                                        | No rows added           | May retry                                                         |
| Database failure, uncertain                                             | `DatabaseUncertain`     | Source error                                                        | Uncertain               | Retry with the same event IDs                                     |

A database-side timeout (`lock_timeout` or `statement_timeout`) is not a separate
case. It appears as **Database failure, rolled back**, because the rollback to
the savepoint (or of the transaction) succeeds, and the retained source is the
database's own error. If that rollback cannot be confirmed, it is **Database
failure, uncertain**.

**Outcomes**

- "No rows added" requires that nothing was written or that rollback succeeded.
  Anything else, including any failed commit or failed rollback, is uncertain.
  Conservative uncertainty calls for reconciliation or a same-ID replay; a false
  "no rows added" invites replacement IDs and duplicate publication. Same-ID
  replay prevents reinsertion only while the original row is retained.
- Outcomes describe one attempt. After an uncertain attempt, a later negative
  result cannot establish that the earlier attempt added no rows. A successful
  retry while the original rows remain retained satisfies the recording
  obligation, but does not prove the earlier attempt's commit or publication
  history. After deletion a retry can insert and publish again; if retention is
  not assured, reconcile or escalate rather than claim duplicate-safe recovery.
  Inside a caller's transaction, how that transaction ends decides.
- If a retry after an uncertain attempt reports a conflict, or retries run out,
  "fix input" no longer applies: stop, keep the IDs and content, and escalate. The
  outcome remains uncertain.

**Retrying**

- Choose event IDs and replay-compared content **once per logical operation,
  before the first attempt**, and preserve or reconstruct them, together with any
  unresolved outcome history, across handler reinvocations, delayed manual
  redrives and process restarts. Never mint replacement IDs while the outcome is
  uncertain.
- Recording never retries internally. Callers may retry a limited number of times
  where the table allows it. In a caller's transaction, retrying means ending it
  and retrying all of it. Deadlocks cannot be told apart from other database
  errors, so this rule covers all of them. Stable event IDs prevent reinsertion
  only while the original rows are retained, and business writes need their own
  retry safety.

**Error content**

- `RecordingError` implements `Error + Send + Sync + 'static`. Its variants and
  auxiliary types are the same with and without `diesel`; retained backend errors
  are reachable through `Error::source()` and downcasting without making Diesel an
  unconditional dependency.
- If a rollback fails, the uncertain error retains the rollback error, and
  `source()` exposes it; the original failure is not required to be retained. Any
  other failed recovery step or diagnostic probe retains the error that step
  observed.
- Crate-authored messages and fields never quote payload content, including
  offending object keys. Backend-originated diagnostics are an explicit
  exception: retained source errors are unchanged and never reduced to text, and
  their bounded rendering in `Display` may contain sensitive database-reported
  values. Truncation is not redaction, so treat source inspection and rendered
  backend text as sensitive.
- `Display` shows the event ID, the metadata field or fixed defect category, and
  the outcome where they apply. Crate-authored details and rendered backend
  messages are bounded (long messages and ID lists are truncated in `Display`
  only). Complete sources and ID lists stay available as data, and `Debug`, full
  source inspection and complete ID lists are not bounded.

### Recording in a caller's transaction

`record_batch_on_connection(&mut conn, &envelopes)` works on the caller's Diesel
connection and needs no outbox pool.

- After validation it requires an open transaction, **however it was opened**: a
  Diesel `transaction`, or raw `BEGIN` as the very first statement. With none
  open it writes nothing and returns `NoOpenTransaction`. An aborted transaction
  or a lost connection is a database failure, never this error. Detection does not
  read Diesel's tracked depth or any error text.
- It never begins or commits the outer transaction; it uses one savepoint, named
  `mulac_outbox_record`, and releases it. A caller savepoint with the same name is
  shadowed while the batch runs and is intact afterwards.
- Nothing is durable until the caller commits. A failed batch's new rows are
  rolled back to the savepoint without undoing the caller's earlier work,
  provided that rollback succeeds.
- Validation errors and replay conflicts should propagate by default. Continuing
  is an explicit exception: the business writes then commit without those events,
  giving up the outbox guarantee for them.
- Any other failure, apart from `NoOpenTransaction`, means abandoning the whole
  transaction, even if the database would allow it to continue. Ignoring this can
  make a commit report success after the database rolled back. Retry only after
  the failed transaction has ended, and retry all of it.
- Deadlocks can occur at `READ COMMITTED` when transactions already hold locks. A
  concurrent write of the same event can raise a serialization failure above
  `READ COMMITTED`. Both are reported as a rolled-back database failure whose
  source is the original database error (Diesel's `DatabaseErrorKind::SerializationFailure`
  for the latter).
- The caller owns the outcome of its own commit. A backend error after its
  transaction closure returned success is an uncertain commit; if the closure
  failed, the transaction did not commit. Replaying outbox events does not make
  business writes idempotent, so after an uncertain commit the caller must
  reconcile or safely retry the complete business operation.

### Pooled recording inside a caller's transaction

`OutboxRecorder` and `OutboxStorePort` recording use the store's own pooled
connection and their own transaction. Calling them while the application has a
business transaction open on a different connection **commits the outbox rows
independently**: if the business transaction later rolls back, those outbox rows
stay committed and are published later, and messages already published cannot be
retracted. To make business writes and outbox events commit or roll back
together, record with `record_batch_on_connection` on the business transaction's
connection.

### Handler example

`libs/outbox/examples/record_with_retry.rs` is the one handler example. CI
compiles it and runs it against PostgreSQL (`libs/outbox/tests/handler_example.rs`
includes the file itself). It shows:

- a bounded retry loop over whole transactions, with one application-owned error
  type that keeps recording failures, business/database failures and the caller's
  own commit failure apart, before any explicit conversion into a handler or
  subscriber error;
- recording what the callback itself returned, so an outer rollback error cannot
  replace the callback's failure or pass for a commit error;
- `Plan` (event IDs and replay-compared content, chosen once, and whether row
  retention is assured) and `History` (attempt outcomes, whether an uncertain
  outcome is unresolved, and whether the obligation is recorded), which the
  application persists with the business operation so both survive reinvocation,
  redrive and restart. A later success sets `recorded` but never clears
  `unresolved`; only the application does, after reconciling;
- retention as an explicit precondition: when the plan does not assure that the
  rows outlive the retry horizon, the loop never retries an uncertain outcome and
  returns an escalation for reconciliation instead;
- every recording case handled explicitly, with no wildcard;
- event IDs logged whenever an outcome is uncertain, and escalation (not new IDs)
  when a retry after an uncertain attempt conflicts or retries run out;
- a business write that is safe to repeat. A `COMMIT` can reach the database
  while its acknowledgment is lost, and the handler cannot tell that from a failed
  commit, so it runs the whole transaction again, business callback included. The
  outbox rows are found again by their IDs and add nothing; the callback must be
  keyed by the plan's operation ID (or reconcile first) to do the same. The
  `handler_example` suite loses a real commit acknowledgment and shows both
  outcomes: a keyed write stays single, an unkeyed one is duplicated;
- a retry budget that cannot be zero (`NonZeroU32`), so a last failure always
  exists to report.

### Waits, timeouts and blocking sessions

Recording can **wait without limit** on a concurrent uncommitted write of the same
event ID (another recording batch, or an application update of that row), and it
holds its connection while it waits. The crate sets no timeout. Timeouts are the
caller's, and so is the choice of whether recording shares a pool with consumers:
a pool shared with consumers can be exhausted by waiting recorders, which stalls
reservation. Prefer a separate pool for recording, or bound the wait.

**Set a database-side timeout.**

- *Pooled recording:* put the setting on the pool's connections, for example in
  the URL given to `build_pool`: `postgres://…?options=-clock_timeout%3D5000`
  (milliseconds; separate several settings with `%20`, as in
  `-clock_timeout%3D5000%20-cstatement_timeout%3D30000`). Use a pool dedicated to
  recording so consumers keep their own settings.
- *Caller connection:* `SET LOCAL lock_timeout = '5s'` (or `statement_timeout`) in
  the caller's transaction before recording. It ends with the transaction.

**How a timeout appears.** As **Database failure, rolled back** in the Failures
table: a `RecordingError::DatabaseRolledBack` whose `source()` is the database
error ("canceling statement due to lock timeout", or "… statement timeout").
Inside a caller's transaction the usual rule applies: abandon the transaction and
retry all of it.

**A caller that stops waiting without receiving a result** (for example a
client-side timeout that drops the call, or a cancelled task) must treat the
attempt as **uncertain**. The connection may stay occupied until the database
call ends.

**Identify the blocking session** while a recording call is waiting:

```sql
SELECT blocked.pid   AS blocked_pid,
       blocking.pid  AS blocking_pid,
       blocking.state,
       blocking.xact_start,
       blocking.query AS blocking_last_query
FROM pg_stat_activity AS blocked
CROSS JOIN LATERAL unnest(pg_blocking_pids(blocked.pid)) AS b(pid)
JOIN pg_stat_activity AS blocking ON blocking.pid = b.pid
WHERE blocked.query LIKE '%outbox_entries%';
```

A blocking session that is `idle in transaction` is usually a caller that opened
a transaction, recorded, and has not committed.

### Recording from event subscribers

An event subscriber that records must expect permanent conflicts, and must map
`RecordingError` into the subscriber's error explicitly.

- **TwoPhased eventing.** Propagating a conflict fails the event and retries it
  with the same event ID and payload: earlier subscribers re-run, later ones never
  run, and the exhausted event stays `Failed`.
- **Direct eventing.** The error returns to the caller with no retry, and each
  command execution assigns new event IDs. A subscriber that uses the delivered
  event ID as its outbox event ID therefore gets replacement IDs if the caller
  re-dispatches the command. After an uncertain outcome that is not a same-ID
  retry and can publish a duplicate. Such a subscriber must take its outbox event
  IDs and replay-compared content from something stable across command retries,
  or the caller must reconcile or escalate instead of re-dispatching.

`test_apps/outbox_subscriber_behavior` runs these cases against PostgreSQL.

### Recovering from a permanent replay conflict

The crate ships **no redrive or repair API**. Any requeue of an exhausted row is
an application-owned database change, and deleting or rewriting stored outbox
rows is not a safe conflict repair. The options, and what each gives up:

| Option | Gives up |
| ------ | -------- |
| **Resubmit with the original replay-compared content**, if input validation and the stored-row replay checks pass | Needs that content to still exist. It neither repairs invalid stored rows nor restarts exhausted outbox publication. |
| **Drop the event from the batch and continue** | The omitted candidate is not recorded by this attempt, so its outbox guarantee is lost. Any existing row with that ID, and its publication, are unaffected. |
| **Use a new event ID with new content** | A separate message that consumers cannot deduplicate against the original by event ID alone, and which may duplicate the logical message if the original was or will be published. |
| **Leave an exhausted TwoPhased source event `Failed` and escalate** | The source event stays blocked and later subscribers do not run. |

Two different things are called "exhausted", and they recover differently:

- **An exhausted source event (eventing).** A requeue re-runs the subscribers
  from the start, so earlier subscribers repeat.
- **Exhausted outbox publication.** A matching replay leaves `attempts` and
  `status` unchanged, so recording again does not restart publication. Requeueing
  is a database change by the application: the row must end up `Received` or
  `Failed`, with `attempts` below the consumer's `max_attempts`, a `scheduled_at`
  in the past, and `reservation_id` and `reserved_at` cleared. Doing so can
  publish the message again.

To investigate an uncertain outcome, inspect the event IDs and the compared
content in the outbox:

- matching retained rows establish current outbox presence, not business success
  or broker delivery;
- missing rows do not prove the events were never published;
- a mixed result (some present, some missing) does not prove a partial commit.

Coordinate retention and in-flight writers first, or preserve the uncertainty and
escalate.

### Batch size

There is no enforced maximum, and statement limits do not limit a batch. A very
large batch has costs:

- **Memory.** The caller holds the whole input, and recording holds converted
  copies of it. In the measurements, client peak memory was about three times the
  payload size.
- **One long transaction and a held connection.** A batch is a single transaction
  on a single pooled connection from its first insert to its commit, and its rows
  stay invisible until the commit. Recording time grows in proportion to the
  batch. A long transaction also holds back vacuum cleanup.
- **A delivery backlog.** Consumers take the oldest scheduled entries first, so a
  very large batch is a burst of eligible rows at once.

**Suggested size: about 1,000 to 5,000 events per batch.** In the measurements,
throughput levelled off around 5,000 events (about 25,000 events per second;
1,000 new events were about 20% slower) and did not improve above that, while
larger batches added only memory and transaction time (5,000 events: about 0.2 s
and 64 MiB of client peak memory; 50,000: about 2.3 s and 268 MiB; 200,000: about
7.6 s and 944 MiB, with the database also growing by hundreds of MiB). Treat this
as a starting point and measure with your own payloads and hardware.

The measurements, with the PostgreSQL version and configuration, the payload
distribution, the batch sizes and exactly what was and was not measured, are in
[outbox-bulk-recording-measurements.md](outbox-bulk-recording-measurements.md).
They are a record, not a performance threshold, and nothing in CI asserts them.

## Retry Policy

Applies to all automatic `failed()` transitions, including conversion and publication failures:

- Failure retry delay = `max(attempts, 1) × 30 seconds`, measured from database time
- Cap: 2 minutes per attempt
- At `max_attempts` (default 6), failed entries remain `Failed` but are ineligible for automatic reservation
- Stale sweep does **not** increment the attempt counter
- Explicit administrative `dead()` remains available; automatic consumers never invoke it

## Delivery and Idempotency

- The outbox acceptance boundary is the commit of the recording transaction, or, for an already-recorded `event_id`, finding the stored row with matching content. For recording inside a caller's transaction it is the caller's commit.
- The publication boundary is broker acceptance / publisher confirmation.
- If broker acceptance succeeds but `completed()` fails, the entry may be retried and the same outbound message may be published again.
- Consumers downstream of the external transport must treat `message_id` as an idempotency key when possible. By default, `message_id` equals `event_id`, unless the event dispatcher supplied a distinct outbound message ID.
- The outbox does not guarantee ordering across entries, destinations, retries, or concurrent consumers.

## HTTP Batch Endpoint

`POST /run` is provided by this crate's `axum` and `poem` features and mounted under
an application-owned prefix ([ADR-011](adr/011-core-http-features.md)). It publishes
one batch through
`OutboxConsumer::publish_batch` with `ReservableOutboxSpec::new(limit)`:

- `limit` must be from `1` to `1000`; other values return `400` with that message.
  A body that can't be parsed returns `400` with only a logged error ID.
- A processed batch returns `200` with `{"processed":N,"errors":[]}`. Any
  failure sets `processed` to `null` and lists one logged error ID per failure in
  `errors`; published entries stay completed.
- Without a configured publisher, valid requests return `503`
  `outbox publisher is not configured` and storage is not touched.
- A failed blocking task returns `500` with only a logged error ID.

Error IDs are explained in [ADR-015](adr/015-http-errors-by-id.md).

`AsyncOutboxService::from_postgres(pool, publisher)` assembles the consumer from
`Arc::new(DieselPgOutboxRepository::new(pool))`. The publisher is any `OutboxPublisherPort`; the
HTTP features never enable `amqp`.

## Feature Flags

| Feature  | Adds                                                                   |
| -------- | ---------------------------------------------------------------------- |
| `diesel` | PostgreSQL storage adapters, schema mapping, JSONB metadata support    |
| `amqp`   | `AmqpPublisher` and the strict `ConfirmedAmqpPublisher` (lapin, Tokio) |

## Examples

### Recording events

```rust
let recorder = OutboxRecorder::new(repository);
recorder.record_batch(&[NewOutboxEnvelope {
    payload: r#"{"user_id":"123"}"#.to_string(),
    metadata: NewOutboxMetadata {
        sequence: None,
        event_id,
        message_id: None,
        correlation_id: Some(correlation_id),
        causation_id: Some(command_id),
        event_type: "UserRegistered".to_string(),
        routing_key: "users.registered".to_string(),
        source: Some("identity-service".to_string()),
        content_type: Some("application/json".to_string()),
    },
}])?;
```

### Recording in the business transaction

```rust
conn.transaction::<_, diesel::result::Error, _>(|conn| {
    diesel::insert_into(users::table).values(&user).execute(conn)?;
    match record_batch_on_connection(conn, &events) {
        Ok(()) => Ok(()),
        // Abandon the whole transaction; do not convert the error implicitly.
        Err(_) => Err(diesel::result::Error::RollbackTransaction),
    }
})?;
```

See `libs/outbox/examples/record_with_retry.rs` for the complete handler with a
bounded retry loop.

### Publishing reserved entries

```rust
let spec = ReservableOutboxSpec { limit: 100, max_attempts: 6 };
let mut consumer = OutboxConsumer::new(repository, publisher);
let result = consumer.publish_batch(&spec).await;
```

## Reusable HTTP endpoints

Framework preludes expose `outbox_routes(service)` and `OutboxApi::new(service)`.
Both accept `impl Into<AsyncOutboxService>`, so an `Arc<OutboxConsumer<R>>` can be
passed directly.
Relative `POST /run` processes a batch.
The host owns mount prefixes, authentication, authorization and middleware.
Existing custom adapters remain available; HTTP status/body contracts and Cargo
feature names are unchanged. The shared Poem run schemas remain in `mulac_http`.
