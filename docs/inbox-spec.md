# Inbox Specification

## Overview

The inbox crate provides a durable inbound message buffer that decouples external transport delivery from internal command processing. It accepts raw messages from an external transport (AMQP or other), stores them atomically, and exposes them to consumers that transform each entry into a command for the write side.

The inbox guarantees at-least-once delivery to consumers. Deduplication, stale-reservation recovery, and retry scheduling are built in.

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
- Respects a `CancellationToken`; shuts down cleanly on cancellation or stream end.

### AmqpTransport / AmqpClient

Wraps a `lapin::Consumer`. Implements `InboxTransportPort` by converting AMQP `Delivery` objects into `InboundMessageEnvelope`, extracting UUID properties from AMQP message headers.

### InboxRecorder / InboxRecorderRepository

Use case: record an inbound message durably.

- Converts an `InboundMessageEnvelope` to a `NewInboxMessageEnvelope`.
- Generates a UUID v7 message ID when the transport does not supply one.
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
- Resets them to `Failed` with a retry backoff, clearing reservation fields.
- Does not increment the attempt counter (only explicit `failed()` calls count as attempts).

### InboxStoreStorage (infra, Diesel)

Implements `InboxStorePort`. Inserts a new `inbox_entries` row. Does not touch existing rows on conflict.

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
| `id`             | `i64`                   | Auto-increment primary key              |
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

| Field      | Type                           | Description               |
|------------|--------------------------------|---------------------------|
| `payload`  | `String`                       | Raw message body          |
| `metadata` | `Option<InboxMessageMetadata>` | Optional routing metadata |

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

Write-side envelope used for recording. `message_id` is always `Uuid` (never `Option`) — generated as UUID v7 when absent from the transport.

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
async fn completed(&self, id: i64, reservation_id: Uuid) -> Result<(), InboxError>;
async fn failed(&self, id: i64, reservation_id: Uuid, max_attempts: i32) -> Result<(), InboxError>;
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
    id              BIGSERIAL    PRIMARY KEY,
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

- Retry delay = `attempts × 30 seconds`
- Cap: 2 minutes per attempt
- After `max_attempts` (default 6) the entry transitions to `Dead`
- Stale sweep does **not** increment the attempt counter

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
