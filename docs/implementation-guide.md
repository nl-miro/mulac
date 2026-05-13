# Crate Implementation Guide

This guide describes how to implement new crates in this repository, using the `inbox` crate as the reference implementation. Follow these patterns when building `write_side` (Command/Event Dispatchers) and `outbox`.

For the architectural rationale behind each pattern, see the [ADR index](adr/).

---

## File structure

Every component crate follows this layout:

```
<crate>/
├── Cargo.toml
├── SPEC.md               ← technical specification (filled before implementation)
└── src/
    ├── lib.rs            ← public io facade only, no logic
    ├── assembly/
    │   ├── mod.rs
    │   ├── domain.rs     ← core models, status enums, value objects
    │   ├── application.rs← port traits, application envelopes, error type
    │   └── infra_<name>.rs ← concrete storage or transport adapter
    ├── <use_case_1>.rs   ← one file per use case
    ├── <use_case_2>.rs
    └── <use_case_n>.rs
```

Infrastructure adapters that are optional go behind feature flags and into `infra_<name>.rs` modules inside `assembly/`.

---

## Layer rules

### domain.rs

- Contains only models: structs, enums, and their `impl` blocks.
- No external crate dependencies — not even `serde` unless the domain type is directly serialised.
- Status enums use sparse integer codes. See [ADR-002](adr/002-status-code-gaps.md).
- Implement `From<i32>` and `Into<i32>` (or `TryFrom<i32>`) for status enums.

```rust
// Sparse codes: 0, 2, 4, 5, 7, 8. Gaps reserved for future insertion.
pub enum CommandStatus {
    Received   = 0,
    Reserved   = 2,
    Failed     = 4,
    Completed  = 5,
    Dead       = 7,
    Archive    = 8,
}
```

### application.rs

- Contains port traits, application-level envelopes, and the crate's `Error` enum.
- All port traits are async (`async fn` or return `impl Future`).
- Error enum uses `thiserror::Error`. Each variant corresponds to a failure category (storage, transport, conversion, etc.), not to individual functions.
- No concrete implementations — this file has no `use diesel::…` or `use lapin::…`.

Port naming convention:

| Responsibility                  | Trait name                 |
|---------------------------------|----------------------------|
| Write a new entry               | `<Component>StorePort`     |
| Reserve entries                 | `<Component>ReservePort`   |
| Mark completed/failed           | `<Component>ProcessPort`   |
| Release stale reservations      | `<Component>SweepPort`     |
| Receive from external transport | `<Component>TransportPort` |
| Acknowledge transport delivery  | `AcknowledgeHandle`        |

### use case modules

One file per use case. Each file contains:

1. **Spec struct** — configuration for that use case (limits, timeouts, attempt counts).
2. **Repository struct** — holds `Arc<dyn SomePort>` fields; named `<Component><UseCase>Repository`.
3. **Component struct** — the public-facing handle; named `<Component><Noun>` (e.g. `InboxRecorder`, `InboxConsumer`). Wraps the repository.
4. **Constructor function** — `pub fn repository(database_url: String) -> Result<…, String>` that builds the storage adapter and wires up the repository.

```rust
// record_messages.rs pattern
pub struct RecordableSpec { /* config */ }

pub struct CommandRecorderRepository {
    store: Arc<dyn CommandStorePort>,
}

pub struct CommandRecorder {
    repository: CommandRecorderRepository,
}

pub fn repository(database_url: String) -> Result<CommandRecorderRepository, String> {
    let pool = build_pool(&database_url).map_err(|e| e.to_string())?;
    let store = Arc::new(CommandStoreStorage::new(pool));
    Ok(CommandRecorderRepository::new(store))
}
```

### infra_<name>.rs

- Contains concrete implementations of the port traits defined in `application.rs`.
- Named by adapter, not by use case: `infra_diesel.rs`, not `recording_diesel.rs`.
- One storage struct per responsibility cluster:

  ```
  CommandStoreStorage    → implements CommandStorePort
  CommandConsumerStorage → implements CommandReservePort + CommandProcessPort + CommandSweepPort
  ```

- Reservation uses `SELECT … FOR UPDATE SKIP LOCKED` in a single transaction. See [ADR-004](adr/004-skip-locked-reservation.md).
- Failed transitions apply retry backoff: `attempts × 30 seconds`, capped at 2 minutes. See [contracts.md](contracts.md).
- All generated IDs use UUID v7. See [ADR-003](adr/003-uuid-v7-identifiers.md).

### lib.rs

Re-export everything that external callers need through a single `io` module. Nothing else.

```rust
pub mod io {
    pub use crate::record_messages::{CommandRecorder, CommandRecorderRepository};
    pub use crate::command_consumer::{CommandConsumer, CommandConsumerRepository, ReservableCommandSpec};
    pub use crate::stale_reservation_sweep::{ReservationSweeper, StaleReservationSpec};
    pub use crate::assembly::application::{
        CommandEnvelope,
        CommandError,
        CommandProcessPort, //
    };

    #[cfg(feature = "diesel")]
    pub use crate::assembly::infra_diesel::{CommandStoreStorage, CommandConsumerStorage, DbPool};
}
```

---

## Database schema

Each component owns one entry table. The schema follows the inbox pattern exactly:

```sql
CREATE TABLE <component>_entries (
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

-- required indexes
CREATE INDEX ON <component>_entries (status, scheduled_at);
CREATE INDEX ON <component>_entries (status, reserved_at);
```

Metadata is stored as JSONB. See [ADR-005](adr/005-jsonb-metadata.md).

---

## Retry policy

Apply this policy uniformly in `failed()` transitions across all components:

- Delay = `attempts × 30 seconds`, where `attempts` is the value after incrementing on reservation.
- Maximum single-attempt delay: 2 minutes.
- At `max_attempts` (default 6): transition to `Dead` instead of `Failed`.
- Stale sweep resets to `Received`/`Failed` without incrementing `attempts`.

```rust
let delay_secs = (entry.attempts as i64 * 30).min(120);
let scheduled_at = Utc::now() + Duration::seconds(delay_secs);
```

---

## Cargo.toml checklist

```toml
[package]
edition = "2024"

[features]
diesel = ["dep:diesel", "dep:r2d2", "dep:diesel-derive-newtype"]
amqp   = ["dep:lapin", "dep:tokio-amqp"]

[dependencies]
thiserror = "..."
uuid = { version = "...", features = ["v7"] }
chrono = { version = "...", features = ["serde"] }
serde = { version = "...", features = ["derive"] }
serde_json = "..."
tokio = { version = "...", features = ["full"] }
# infrastructure dependencies are optional:
diesel = { version = "...", features = ["postgres", "chrono", "uuid", "serde_json"], optional = true }
```

---

## Checklist for implementing a new crate

- [ ] Fill in `SPEC.md` before writing code
- [ ] Write `assembly/domain.rs`: status enum with sparse codes, entry model
- [ ] Write `assembly/application.rs`: port traits, envelope types, error enum
- [ ] Write use-case modules with Spec, Repository, Component, and constructor
- [ ] Write `assembly/infra_<name>.rs` behind feature flag
- [ ] Wire up `lib.rs` with `io` re-exports
- [ ] Add reservation tests: concurrent workers must not claim the same entry
- [ ] Add retry tests: failed entries schedule at correct future time
- [ ] Add stale sweep tests: sweep does not increment attempt counter
- [ ] Update `Makefile` if the crate is new

---

## What not to do

- Do not put SQL in use-case modules. SQL belongs in `infra_*` modules only.
- Do not import from `crate::io` inside the crate. Use direct module paths internally; `io` is for external callers.
- Do not share a single repository across multiple use cases. Each use case gets its own.
- Do not create a generic `<Component>Repository<S>` as the public API. Use `Arc<dyn Trait>` and keep the repository concrete. See [ADR-008](adr/008-arc-dyn-trait-dependencies.md).
- Do not add status codes sequentially. Leave gaps. See [ADR-002](adr/002-status-code-gaps.md).
- Do not use UUID v4 for generated IDs. Use `Uuid::now_v7()`. See [ADR-003](adr/003-uuid-v7-identifiers.md).
- Do not acknowledge transport delivery before durable storage is confirmed.

---

## References

- [developer-guidelines.md](developer-guidelines.md) — import style, Rust idioms
- [contracts.md](contracts.md) — what each component guarantees to its callers
- [components.md](components.md) — component responsibilities and lifecycle states
- [architecture-spec.md](architecture-spec.md) — end-to-end flows
- [inbox-spec.md](inbox-spec.md) — reference specification
- [ADR index](adr/) — rationale behind every architectural decision
