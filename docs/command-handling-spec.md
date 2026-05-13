# Command Handling Specification

## Overview

`CommandGateway` is the single entry point for all command dispatch. Callers (application code or the inbox consumer) pass a `NewCommandEnvelope` to the gateway; the gateway decides how execution proceeds based on the configured variant:

- **Direct**: the gateway immediately delegates to `CommandDispatcher`, which resolves and invokes the handler synchronously. No durable state; errors propagate immediately to the caller.
- **TwoPhased**: the gateway persists a durable `NewCommandEntry` and returns. A separate consumer loop later picks up the entry and passes it to `CommandDispatcher` for execution, with full retry and stale-reservation recovery.

`CommandDispatcher` is the shared execution engine for both variants. It resolves the command handler from a registry, invokes it, and hands produced events to the event dispatcher.

The TwoPhased variant guarantees at-least-once execution. Command handlers must therefore tolerate duplicate invocations or implement their own deduplication.

## Architecture

The crate follows a hexagonal (ports and adapters) architecture with four layers:

```
domain  ←  application  ←  features  ←  infra adapters
```

- **domain** (`assembly/domain.rs`): `CommandStatus` — no I/O, no external types
- **application** (`assembly/application.rs`): port traits, envelopes, error type — no implementation
- **features** (`record_commands.rs`, `command_consumer.rs`, `stale_command_sweep.rs`): use-case orchestration — depends on ports only
- **infra adapters** (`assembly/infra_diesel.rs`): concrete I/O — depends on feature and application layers

External callers import exclusively from `lib.rs` via the `io` re-export facade.

```
lib.rs (io facade)
  ├─ gateway                CommandGateway
  ├─ dispatcher             CommandDispatcher
  ├─ record_commands        CommandRecorder, CommandRecorderRepository
  ├─ command_consumer       CommandConsumer, CommandConsumerRepository, ReservableCommandSpec
  ├─ stale_command_sweep    CommandSweeper, StaleCommandSpec
  ├─ assembly/application   NewCommandEnvelope, NewCommandMetadata,
  │                         CommandEnvelope, CommandMetadata,
  │                         CommandError, CommandStatus,
  │                         CommandStorePort, CommandReservePort, CommandProcessPort,
  │                         CommandHandlerPort, CommandSweepPort
  └─ assembly/infra_diesel  [feature = "diesel"]
       CommandEntry, NewCommandEntry,
       CommandStoreStorage, CommandConsumerStorage, DbPool, build_pool
```

## Variants

### Direct

Execution is synchronous and in-flight only. No database table, no reservation mechanism, no retry record.

1. The caller passes a `NewCommandEnvelope` to `CommandGateway`.
2. The gateway delegates immediately to `CommandDispatcher`.
3. The dispatcher resolves the handler via `CommandHandlerPort` using `command_type` and invokes it.
4. The handler returns zero or more `EventEnvelope` instances; the dispatcher hands each to the event dispatcher.
5. On any failure the error propagates to the caller. Any handler side-effects already applied remain in place; no automatic retry is available.

The Direct variant is suitable for synchronous request-response flows where the caller controls retry and the command handler is idempotent.

### TwoPhased

Execution is decoupled and durable. The gateway persists a `NewCommandEntry` and returns; a consumer loop does the actual work via `CommandDispatcher`.

1. The caller passes a `NewCommandEnvelope` to `CommandGateway`.
2. The gateway stores a `NewCommandEntry` (status `Received`) via `CommandStorePort` and returns.
3. The `NewCommandEntry` is now durably queued; the caller's work is done.
4. A worker drives a consumer loop that reserves eligible entries atomically via `CommandReservePort`.
5. For each reserved entry, the consumer passes the envelope to `CommandDispatcher`.
6. The dispatcher resolves the handler via `CommandHandlerPort` and invokes it.
7. The handler returns zero or more `EventEnvelope` instances; the dispatcher hands each to the event dispatcher.
8. On success: the consumer marks the entry `Completed` via `CommandProcessPort`.
9. On failure: the consumer marks the entry `Failed` and schedules a retry, or `Dead` if the retry limit is exhausted.
10. A separate sweep job releases entries stuck in `Reserved` longer than the configured timeout.

## Components

### CommandGateway

The single entry point for all command dispatch. Callers never interact with `CommandDispatcher` directly.

- Accepts a `NewCommandEnvelope` from any caller. The caller is responsible for supplying a `command_id`; no generation occurs inside the gateway.
- **Direct**: delegates immediately to `CommandDispatcher`.
- **TwoPhased**: stores the command via `CommandRecorder` and returns. Does not invoke `CommandDispatcher`.

### CommandDispatcher

Shared execution engine used by both variants. Not called directly by application code.

- Receives a `NewCommandEnvelope` (from the gateway in Direct, from the consumer in TwoPhased).
- Resolves the command handler via `CommandHandlerPort` using `command_type`.
- Invokes the handler and collects produced `EventEnvelope` instances.
- Hands each event to the event dispatcher.
- Returns the combined result to its caller (gateway or consumer).

### CommandRecorder / CommandRecorderRepository

Use case: persist a `NewCommandEnvelope` durably before execution (TwoPhased only).

- Receives a `NewCommandEnvelope` directly; no conversion or ID generation is performed.
- Delegates persistence to `CommandStorePort`.

### CommandConsumer / CommandConsumerRepository

Use case: reserve persisted commands and execute them via `CommandDispatcher` (TwoPhased only).

- Reserves up to `limit` eligible entries atomically.
- Passes each reserved `CommandEnvelope` to `CommandDispatcher`.
- Marks each entry `Completed` or `Failed` based on the outcome returned by the dispatcher.
- Returns all errors collected across the batch; does not abort the batch on first error.

### CommandSweeper

Use case: release stale reservations back to the eligible queue (TwoPhased only).

- Finds all `Reserved` entries whose `reserved_at` is older than the configured timeout.
- Resets them to `Failed` with a retry backoff, clearing reservation fields.
- Does **not** increment the attempt counter (only explicit `failed()` calls count as attempts).

### CommandStoreStorage (infra, Diesel)

Implements `CommandStorePort`. Inserts a new `command_entries` row. Does not touch existing rows on conflict (idempotent recording).

### CommandConsumerStorage (infra, Diesel)

Implements `CommandReservePort` and `CommandProcessPort`.

- **reserve**: `SELECT … FOR UPDATE SKIP LOCKED`, then updates status to `Reserved`, assigns `reservation_id` (UUID v7), increments `attempts`. Returns full envelopes.
- **completed**: Updates status to `Completed`, clears `reservation_id` and `reserved_at`, sets `processed_at`.
- **failed**: Evaluates `attempts` against `max_attempts`; transitions to `Failed` (schedules retry) or `Dead` (exhausted). Clears reservation fields. Retry delay: `attempts × 30 seconds`, capped at 2 minutes.

## Models

### CommandStatus (domain)

| Variant     | Code | Description                                 |
|-------------|------|---------------------------------------------|
| `Received`  | `0`  | Stored, eligible for reservation            |
| `Reserved`  | `2`  | Claimed by a consumer                       |
| `Failed`    | `4`  | Execution failed; scheduled for retry       |
| `Completed` | `5`  | Successfully executed and events handed off |
| `Dead`      | `7`  | Retry limit exhausted                       |
| `Archive`   | `8`  | Archived (no further processing)            |

Numeric codes use intentional gaps (1, 3, 6 are reserved for future insertion without renumbering). See [ADR-002](adr/002-status-code-gaps.md).

### NewCommandEnvelope (application)

The gateway input envelope. `command_id` is always `Uuid` — the caller must supply it (UUID v7 recommended). No generation occurs inside the system boundary.

| Field          | Type                         | Description                               |
|----------------|------------------------------|-------------------------------------------|
| `command_type` | `String`                     | Handler discriminator                     |
| `payload`      | `String`                     | Raw command body                          |
| `metadata`     | `Option<NewCommandMetadata>` | Optional routing and correlation metadata |

### NewCommandMetadata (application)

| Field            | Type             | Description                                |
|------------------|------------------|--------------------------------------------|
| `command_id`     | `Uuid`           | Stable deduplication ID (required)         |
| `correlation_id` | `Option<Uuid>`   | Cross-service correlation chain            |
| `causation_id`   | `Option<Uuid>`   | ID of the message that caused this command |
| `source`         | `Option<String>` | Originating service identifier             |

`causation_id` should be propagated from the triggering inbox message's `message_id` when a command originates from inbox processing. Produced `EventEnvelope` instances should carry the command's `command_id` as their own `causation_id`.

### CommandEnvelope (application)

Read-side envelope returned to the consumer after reservation. Constructed by the infra layer from the stored `CommandEntry`; contains all fields the consumer needs without exposing infra types.

| Field            | Type                      | Description                                    |
|------------------|---------------------------|------------------------------------------------|
| `id`             | `Uuid`                    | Entry identifier, used to mark complete/failed |
| `reservation_id` | `Uuid`                    | Ownership token, required for process calls    |
| `command_type`   | `String`                  | Handler discriminator                          |
| `payload`        | `String`                  | Raw command body                               |
| `attempts`       | `i32`                     | Number of times reserved so far                |
| `metadata`       | `Option<CommandMetadata>` | Decoded routing and correlation metadata       |

### CommandMetadata (application)

Decoded metadata attached to a reserved command.

| Field            | Type             | Description                                |
|------------------|------------------|--------------------------------------------|
| `command_id`     | `Uuid`           | Stable deduplication ID                    |
| `correlation_id` | `Option<Uuid>`   | Cross-service correlation chain            |
| `causation_id`   | `Option<Uuid>`   | ID of the message that caused this command |
| `source`         | `Option<String>` | Originating service identifier             |

### ReservableCommandSpec (command_consumer)

| Field          | Default | Description                                        |
|----------------|---------|----------------------------------------------------|
| `limit`        | —       | Maximum number of entries to reserve per call      |
| `max_attempts` | `6`     | Entries at or above this attempt count are skipped |

Builds query criteria: `StatusIn([Received, Failed])`, `ScheduledBeforeNow`, `MaxAttempts(max_attempts)`, `OrderByScheduledAtAsc`.

### StaleCommandSpec (stale_command_sweep)

| Field          | Default | Description                                                      |
|----------------|---------|------------------------------------------------------------------|
| `timeout`      | —       | Staleness threshold; entries reserved longer than this are swept |
| `max_attempts` | `6`     | Max attempts forwarded to the retry policy                       |

### CommandError (application)

| Variant              | Description                                      |
|----------------------|--------------------------------------------------|
| `Storage`            | Storage adapter error                            |
| `Reservation`        | Reservation query or update failure              |
| `HandlerNotFound`    | No handler registered for the given command_type |
| `HandlerExecution`   | Command handler returned an error                |
| `EventDispatch`      | Event dispatcher rejected a produced event       |
| `MissingReservation` | Entry returned without a reservation_id          |
| `Conversion`         | Envelope conversion failure                      |

## Ports

### CommandStorePort

```rust
async fn record(&self, envelope: &NewCommandEnvelope) -> Result<(), CommandError>;
```

### CommandReservePort

```rust
async fn reserve(&self, spec: &ReservableCommandSpec) -> Result<Vec<CommandEnvelope>, CommandError>;
```

### CommandProcessPort

```rust
async fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), CommandError>;
async fn failed(&self, id: Uuid, reservation_id: Uuid, max_attempts: i32) -> Result<(), CommandError>;
```

### CommandSweepPort

```rust
async fn sweep(&self, spec: &StaleCommandSpec) -> Result<u64, CommandError>;
```

### CommandHandlerPort

```rust
async fn execute(&self, envelope: &NewCommandEnvelope) -> Result<Vec<EventEnvelope>, CommandError>;
```

## Infra Models

These types are defined in `assembly/infra_diesel.rs` and are only available with the `diesel` feature. They are not referenced by the application or feature layers.

### CommandEntry (infra, Diesel)

Queryable struct mapping the full `command_entries` row. Constructed by `CommandConsumerStorage` and converted into `CommandEnvelope` before being returned across the port boundary.

| Field            | Type                        | Description                               |
|------------------|-----------------------------|-------------------------------------------|
| `id`             | `Uuid`                      | UUID v7 primary key                       |
| `command_type`   | `String`                    | Discriminator used to resolve the handler |
| `status`         | `CommandStatus`             | Lifecycle state                           |
| `payload`        | `String`                    | Raw UTF-8 command body                    |
| `meta`           | `Option<serde_json::Value>` | JSONB-serialised routing metadata         |
| `scheduled_at`   | `DateTime<Utc>`             | Earliest eligible-for-reservation time    |
| `attempts`       | `i32`                       | Number of times reserved for execution    |
| `reservation_id` | `Option<Uuid>`              | Ownership token assigned on reservation   |
| `reserved_at`    | `Option<DateTime<Utc>>`     | Timestamp of last reservation             |
| `received_at`    | `DateTime<Utc>`             | Timestamp of initial storage              |
| `updated_at`     | `DateTime<Utc>`             | Timestamp of last state change            |
| `processed_at`   | `Option<DateTime<Utc>>`     | Timestamp of completion                   |

### NewCommandEntry (infra, Diesel)

Insertable struct built from a `NewCommandEnvelope` inside `CommandStoreStorage::record()`. Contains only the fields the application provides; database-managed fields are absent.

| Field          | Type                        | Description                                           |
|----------------|-----------------------------|-------------------------------------------------------|
| `id`           | `Uuid`                      | UUID v7, sourced from `NewCommandMetadata.command_id` |
| `command_type` | `String`                    | Discriminator used to resolve the handler             |
| `payload`      | `String`                    | Raw UTF-8 command body                                |
| `meta`         | `Option<serde_json::Value>` | JSONB-serialised routing metadata                     |
| `scheduled_at` | `DateTime<Utc>`             | Earliest eligible-for-reservation time                |
| `received_at`  | `DateTime<Utc>`             | Timestamp of initial storage                          |

## Database Schema

Applies to the TwoPhased variant only.

Table: `command_entries`

```sql
CREATE TABLE command_entries (
    id              UUID         PRIMARY KEY,
    command_type    TEXT         NOT NULL,
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
- `(command_type, status)` — per-type operational queries

The primary key is UUID (not BIGSERIAL) because command IDs are generated by the application layer before storage and may originate from external callers. See [ADR-003](adr/003-uuid-v7-identifiers.md).

## Retry Policy

Applies to `failed()` transitions (TwoPhased variant only):

- Retry delay = `attempts × 30 seconds`
- Cap: 2 minutes per attempt
- After `max_attempts` (default 6) the entry transitions to `Dead`
- Stale sweep does **not** increment the attempt counter

## Feature Flags

| Feature  | Adds                                                                                                             |
|----------|------------------------------------------------------------------------------------------------------------------|
| `diesel` | `CommandEntry`, `NewCommandEntry`, `CommandStoreStorage`, `CommandConsumerStorage`, `DbPool`, Diesel + r2d2 deps |

## Examples

### Direct dispatch via CommandGateway

```rust
let envelope = NewCommandEnvelope {
    command_type: "CreateOrder".into(),
    payload: serde_json::to_string(&cmd)?,
    metadata: Some(NewCommandMetadata {
        command_id: Uuid::now_v7(),
        correlation_id: Some(correlation_id),
        causation_id: Some(inbox_message_id),
        source: Some("order-service".into()),
    }),
};

// Gateway delegates to CommandDispatcher internally
gateway.dispatch(envelope).await?;
```

### TwoPhased: dispatching and consuming

```rust
// Caller side: gateway stores the command and returns
let envelope = NewCommandEnvelope {
    command_type: "PlaceOrder".into(),
    payload: serde_json::to_string(&cmd)?,
    metadata: Some(NewCommandMetadata {
        command_id: Uuid::now_v7(),
        correlation_id: Some(correlation_id),
        causation_id: Some(inbox_message_id),
        source: Some("order-service".into()),
    }),
};
gateway.dispatch(envelope).await?;

// Background consumer loop: reserves entries and passes each to CommandDispatcher
let spec = ReservableCommandSpec { limit: 10, max_attempts: 6 };
let entries = repository.reserve(&spec).await?;

for entry in entries {
    // consumer calls CommandDispatcher internally
    match consumer.execute(&entry).await {
        Ok(_) => repository.completed(entry.id, reservation_id).await?,
        Err(_) => repository.failed(entry.id, reservation_id, spec.max_attempts).await?,
    }
}
```

### Stale reservation sweep

```rust
let spec = StaleCommandSpec {
    timeout: Duration::from_secs(15 * 60),
    max_attempts: 6,
};
let released = sweeper.sweep(&spec).await?;
```

## Document Metadata

| Version | Author                          | Summary                                                                                                                   | Date       |
|---------|---------------------------------|---------------------------------------------------------------------------------------------------------------------------|------------|
| 0.1.0   | Claude::claude-sonnet-4-6::high | Initial command handling technical specification                                                                          | 2026-05-13 |
| 0.1.1   | Claude::claude-sonnet-4-6::high | Move CommandEntry and NewCommandEntry to infra_diesel; flatten CommandEnvelope; CommandStorePort takes NewCommandEnvelope | 2026-05-13 |
