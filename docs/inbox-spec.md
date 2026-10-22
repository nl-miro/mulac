# Inbox Specification

## Overview

The inbox crate provides a durable inbound message buffer that decouples external transport delivery from internal command processing. It accepts raw messages from an external transport (AMQP or other), stores them atomically, and exposes them to consumers that transform each entry into a command for the write side.

The inbox guarantees at-least-once delivery to consumers. Deduplication, stale-reservation recovery, and retry scheduling are built in.

## Message identity and deduplication

**Decision recorded 2026-10-22; message-ID rejection enforcement is pending.** See [the Inbox contract](contracts.md#inbox).

- The supplied message UUID is the `inbox_entries.id` primary key. Deduplication uses that UUID alone within the same inbox table; source remains metadata and does not partition identity.
- Sequential or concurrent recording of the same UUID leaves one original row unchanged, regardless of source, payload differences, age, or lifecycle state. Distinct UUIDs with identical payloads remain distinct entries.
- Protection lasts while the row exists, including beyond 24 hours. There is no configurable time window. Deleting the row allows the UUID to be recorded again; this decision adds no cleanup policy.
- The intended acceptance contract requires a valid UUID message ID, with no additional UUID-version restriction. Missing, empty, or non-UUID IDs must be rejected before storage, without generating replacement IDs or sending success acknowledgements.
- For AMQP, invalid-ID rejection must not requeue the delivery. After successful rejection, processing continues with subsequent deliveries. Rejection failures surface as errors. Dead-lettering occurs only if broker configuration provides it; rejection alone does not guarantee it.

**Current implementation:** storage already deduplicates on UUID identity. Inbound types still allow an absent message ID; the AMQP adapter treats missing/malformed UUID properties as absent, and conversion to the write-side envelope generates a UUID v7 fallback. Consequently, repeated deliveries without a valid ID can still create separate entries. Removing that fallback and enforcing rejection remain future code work, not completed behavior. This documentation update changes no public type, conversion API, or database schema. Correlation-ID handling is unaffected.

## Architecture

The crate follows a hexagonal (ports and adapters) architecture with four layers:

```
domain  ←  application  ←  features  ←  infra adapters
```

- **domain** (`assembly/domain.rs`): core models and status machine — no I/O, no external types
- **application** (`assembly/application.rs`): port traits and application-layer envelopes — no implementation
- **features** (`record_messages.rs`, `inbox_consumer.rs`, `stale_reservation_sweep.rs`): use-case orchestration — depends on ports only
- **infra adapters** (`assembly/infra_diesel.rs`, `amqp_consumption.rs`): concrete I/O — depends on feature and application layers

External callers import exclusively from `lib.rs` via the `io` re-export facade.

```
lib.rs (io facade)
  ├─ record_messages       InboxRecorder, InboxRecorderRepository
  ├─ inbox_consumer        InboxConsumer, InboxConsumerRepository, ReservableInboxSpec
  ├─ stale_reservation_sweep  ReservationSweeper, StaleReservationSpec
  ├─ assembly/application  InboundMessageEnvelope, InboxMessageEnvelope, InboxMessageMetadata,
  │                        InboxError, InboxTransportPort, InboxProcessPort, AcknowledgeHandle
  ├─ assembly/infra_diesel [feature = "diesel"]
  │    InboxStoreStorage, InboxConsumerStorage, DbPool
  └─ amqp_consumption      [feature = "amqp"]
       AmqpWorker, AmqpTransport, WorkerLoop
```

## Components

### AmqpWorker / WorkerLoop

Bridges an external AMQP consumer to the inbox recorder. One worker per queue binding.

- Polls the transport for the next `InboundMessageEnvelope`.
- Spawns a blocking task to record the message via `InboxRecorder`.
- On success: acknowledges the delivery via `AcknowledgeHandle`.
- On recording failure: negative-acknowledges (nacks) the delivery and backs off.
- Respects a `CancellationToken`; explicit cancellation is the clean shutdown path. Stream closure without explicit cancellation is a transport error, not clean shutdown.

### Cancellation and worker termination

**D-07: decided; kernel lifecycle enforcement remains pending.** Under the [kernel lifecycle contract](../kernel/SPEC.md#lifecycle), cancellation stops new intake. An already-started delivery finishes recording and its acknowledgement or negative acknowledgement before the worker stops; recording/acknowledgement failures remain errors, not successful disposal. Cancellation while idle or backing off does not require a new delivery to arrive before stopping.

No queue drain or kernel shutdown deadline is promised. In-flight work can delay termination indefinitely; existing operation-specific timeout/error behavior is unchanged. `wait()` must wait for all kernel-owned workers without initiating cancellation. An unrecoverable inbox-worker error, including unexpected stream closure, does not cancel sibling workers or trigger automatic restart; the failure is reported by `wait()` after all owned workers have stopped. Per-delivery failures remain subject to their existing handling policies.

Inbox asynchronous operation is supported under a live multi-thread Tokio runtime; current-thread runtime support is not promised. Applications remain responsible for stopping and awaiting their own request tasks. Current handles do not own worker joins, so documentation of these guarantees is not evidence that waiting or failure reporting is implemented. See [supported runtime contexts](../kernel/SPEC.md#supported-runtime-contexts).

### AmqpTransport / AmqpClient

Wraps a `lapin::Consumer`. Implements `InboxTransportPort` by converting AMQP `Delivery` objects into `InboundMessageEnvelope`, extracting UUID properties from AMQP message headers.

Missing/malformed message-ID properties currently become absent IDs. The required no-requeue rejection and continuation behavior described under [message identity](#message-identity-and-deduplication) is pending implementation.

### InboxRecorder / InboxRecorderRepository

Use case: record an inbound message durably.

- Converts an `InboundMessageEnvelope` to a `NewInboxMessageEnvelope`.
- Currently generates a UUID v7 message ID when the transport does not supply a valid one. This fallback conflicts with the decided rejection policy above and must not be treated as the intended acceptance contract.
- Delegates persistence to `InboxStorePort`.

### InboxConsumer / InboxConsumerRepository

Use case: reserve messages and process them into commands.

- Reserves up to `limit` eligible entries atomically.
- Converts each `InboxMessageEnvelope` to a `Command::Publish(payload)`.
- Publishes each command via the write-side gateway.
- Marks each entry completed or failed based on the publish outcome.
- Returns all errors collected across the batch; does not abort early.

### ReservationSweeper

Use case: release stale reservations back to the eligible queue.

- Finds all `Reserved` entries whose `reserved_at` is older than the configured timeout.
- Resets them to `Failed` with retry scheduling, or to `Dead` when `attempts >= max_attempts`, clearing reservation fields.
- Does not increment the attempt counter; the attempt was already counted at reservation.

### InboxStoreStorage (infra, Diesel)

Implements `InboxStorePort`. Inserts a new `inbox_entries` row using the supplied message UUID as its primary key. On an ID conflict, recording succeeds without changing the existing row; source and elapsed time do not participate in conflict detection.

### InboxConsumerStorage (infra, Diesel)

Implements `InboxReservePort` and `InboxProcessPort`.

- **reserve**: `SELECT … FOR UPDATE SKIP LOCKED`, then updates status to `Reserved`, assigns `reservation_id` (UUID v7), increments `attempts`. Returns full envelopes.
- **completed**: Updates status to `Completed`, clears `reservation_id` and `reserved_at`, sets `processed_at`.
- **failed**: Evaluates `attempts` against max; transitions to `Failed` (schedules retry) or `Dead` (exhausted). Clears reservation fields. Retry delay: `attempts × 30 seconds`.

## Models

### InboxMessage (domain)

The core domain model for a stored message.

| Field            | Type                    | Description                             |
|------------------|-------------------------|-----------------------------------------|
| `id`             | `Uuid`                  | Supplied message UUID; deduplication key |
| `status`         | `InboxStatus`           | Lifecycle state                         |
| `payload`        | `String`                | Raw UTF-8 message body                  |
| `meta`           | `Option<MetadataJsonb>` | JSONB-serialised routing metadata       |
| `scheduled_at`   | `DateTime<Utc>`         | Earliest eligible-for-reservation time  |
| `attempts`       | `i32`                   | Number of times reserved for processing |
| `reservation_id` | `Option<Uuid>`          | Ownership token assigned on reservation |
| `reserved_at`    | `Option<DateTime<Utc>>` | Timestamp of last reservation           |
| `received_at`    | `DateTime<Utc>`         | Timestamp of initial storage            |
| `updated_at`     | `DateTime<Utc>`         | Timestamp of last state change          |
| `processed_at`   | `Option<DateTime<Utc>>` | Timestamp of completion                 |

### InboxStatus (domain)

| Variant     | Code | Description                            |
|-------------|------|----------------------------------------|
| `Received`  | `0`  | Stored, eligible for reservation       |
| `Reserved`  | `2`  | Claimed by a consumer                  |
| `Failed`    | `4`  | Processing failed; scheduled for retry |
| `Completed` | `5`  | Successfully handed off                |
| `Dead`      | `7`  | Retry limit exhausted                  |
| `Archive`   | `8`  | Archived (no further processing)       |

Numeric codes use intentional gaps (1, 3, 6 are reserved for future insertion without renumbering). See [ADR-002](adr/002-status-code-gaps.md).

### InboundMessageEnvelope (application)

Transport-facing inbound message before storage.

The current Rust type exposes `message_id: Option<Uuid>` directly alongside payload, correlation ID, source, and routing key. The optional representation remains unchanged; the decided acceptance contract requires a present valid UUID. The table below shows those current fields.

| Field            | Type             | Description |
|------------------|------------------|-------------|
| `payload`        | `String`         | Raw message body |
| `message_id`     | `Option<Uuid>`   | Currently optional; required for acceptance under the decided contract |
| `correlation_id` | `Option<Uuid>`   | Cross-service correlation chain |
| `source`         | `Option<String>` | Source metadata, not a deduplication-key component |
| `routing_key`    | `Option<String>` | Routing hint from transport headers |

### InboxMessageMetadata (application)

| Field            | Type             | Description                            |
|------------------|------------------|----------------------------------------|
| `message_id`     | `Option<Uuid>`   | Stable deduplication ID from transport |
| `correlation_id` | `Option<Uuid>`   | Cross-service correlation chain        |
| `source`         | `Option<String>` | Originating service identifier         |
| `routing_key`    | `Option<String>` | Routing hint from transport headers    |

### InboxMessageEnvelope (application)

Read-side envelope returned to consumers after reservation.

| Field      | Type                           | Description             |
|------------|--------------------------------|-------------------------|
| `message`  | `InboxMessage`                 | The stored domain model |
| `metadata` | `Option<InboxMessageMetadata>` | Decoded metadata        |

### NewInboxMessageEnvelope / NewInboxMessageMetadata (record_messages)

Write-side envelope used for recording. `message_id` is always `Uuid` (never `Option`). The current conversion generates a UUID v7 when the inbound ID is absent; under the decided contract, the ID must instead come from a valid supplied message ID and missing/malformed IDs must be rejected. That conversion change is pending; no new signature is specified or claimed here.

### ReservableInboxSpec (inbox_consumer)

| Field          | Default | Description                                        |
|----------------|---------|----------------------------------------------------|
| `limit`        | —       | Maximum number of messages to reserve per call     |
| `max_attempts` | `6`     | Entries at or above this attempt count are skipped |

Builds query criteria: `StatusIn([Received, Failed])`, `ScheduledBeforeNow`, `MaxAttempts(max_attempts)`, `OrderByScheduledAtAsc`.

### StaleReservationSpec (stale_reservation_sweep)

| Field          | Default | Description                                                      |
|----------------|---------|------------------------------------------------------------------|
| `timeout`      | —       | Staleness threshold; entries reserved longer than this are swept |
| `max_attempts` | `6`     | Max attempts forwarded to the retry policy                       |

### InboxError (application)

| Variant              | Description                             |
|----------------------|-----------------------------------------|
| `Transport`          | Transport adapter error                 |
| `Storage`            | Storage adapter error                   |
| `Acknowledgement`    | AMQP ack/nack failure                   |
| `Recording`          | Message recording failure               |
| `Reservation`        | Reservation query or update failure     |
| `Publish`            | Command gateway publish failure         |
| `MissingReservation` | Entry returned without a reservation_id |
| `Conversion`         | Envelope-to-command conversion failure  |

## Ports

### InboxStorePort

```rust
async fn record(&self, envelope: NewInboxMessageEnvelope) -> Result<(), InboxError>;
```

### InboxReservePort

```rust
async fn reserve(&self, spec: &ReservableInboxSpec) -> Result<Vec<InboxMessageEnvelope>, InboxError>;
```

### InboxProcessPort

```rust
async fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), InboxError>;
async fn failed(&self, id: Uuid, reservation_id: Uuid, max_attempts: i32) -> Result<(), InboxError>;
```

### InboxSweepPort

```rust
async fn sweep(&self, spec: &StaleReservationSpec) -> Result<u64, InboxError>;
```

### InboxTransportPort

```rust
async fn next(&mut self) -> Option<(InboundMessageEnvelope, Box<dyn AcknowledgeHandle>)>;
```

### AcknowledgeHandle

```rust
async fn ack(&self) -> Result<(), InboxError>;
async fn nack(&self) -> Result<(), InboxError>;
```

## Database Schema

Table: `inbox_entries`

```sql
CREATE TABLE inbox_entries (
    id              UUID         PRIMARY KEY,
    status          INTEGER      NOT NULL,
    payload         TEXT         NOT NULL,
    meta            JSONB,
    scheduled_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    attempts        INTEGER      NOT NULL DEFAULT 0,
    reservation_id  UUID,
    reserved_at     TIMESTAMPTZ,
    received_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    processed_at    TIMESTAMPTZ
);
```

Recommended indexes:
- `(status, scheduled_at)` — reservation queries
- `(status, reserved_at)` — stale sweep queries

## Retry Policy

Applies to `failed()` transitions:

- An attempt is one reservation; `reserve` increments `attempts`, `failed()` and the stale sweep do not
- Retry delay = `attempts × 30 seconds` (linear, no cap)
- When `attempts >= max_attempts` (default 6) the entry transitions to `Dead`
- The stale sweep applies the same rules: `Failed` with the retry delay, or `Dead` when attempts are exhausted
- See [contracts.md — Entry retry policy](contracts.md#entry-retry-policy)

## Feature Flags

| Feature  | Adds                                                                              |
|----------|-----------------------------------------------------------------------------------|
| `diesel` | `InboxStoreStorage`, `InboxConsumerStorage`, `DbPool`, Diesel + r2d2 dependencies |
| `amqp`   | `AmqpWorker`, `AmqpTransport`, `WorkerLoop`, lapin + tokio-amqp dependencies      |

## Examples

### Recording via AMQP worker

```rust
let worker = AmqpWorker::new(transport, recorder, cancellation_token);
worker.run().await;
```

### Manual reservation and processing

```rust
let spec = ReservableInboxSpec { limit: 10, max_attempts: 6 };
let entries = repository.reserve(&spec).await?;

for entry in entries {
    match process(&entry).await {
        Ok(_) => repository.completed(entry.message.id, reservation_id).await?,
        Err(_) => repository.failed(entry.message.id, reservation_id, spec.max_attempts).await?,
    }
}
```

### Stale reservation sweep

```rust
let spec = StaleReservationSpec {
    timeout: Duration::from_secs(15 * 60),
    max_attempts: 6,
};
let released = sweeper.sweep(&spec).await?;
```

## Document Metadata

| Version | Author                          | Summary                               | Date       |
|---------|---------------------------------|---------------------------------------|------------|
| 0.1.0   | Claude::claude-sonnet-4-6::high | Initial inbox technical specification | 2026-05-12 |
