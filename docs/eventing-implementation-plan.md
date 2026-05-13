# Eventing Implementation Plan

## Context

The `write_side` crate has a scaffolded `eventing/` module with `EventStatus` in `model.rs` and empty stubs throughout. This plan implements it fully, following the commanding module as the reference pattern. The commanding module's `CommandDispatcher` already references `EventDispatchPort` (declared in `eventing/assembly/application.rs`); `EventGateway` must implement that port to close the loop. Scope is eventing only.

---

## Checklist

- [ ] Task 1 — Module scaffold (`mod.rs` restructure)
- [ ] Task 2 — Domain layer (`eventing/assembly/domain.rs`)
- [ ] Task 3 — Application layer (`eventing/assembly/application.rs`)
- [ ] Task 4 — Record events (`eventing/record_events.rs`)
- [ ] Task 5 — Event consumer (`eventing/event_consumer.rs`)
- [ ] Task 6 — Stale event sweep (`eventing/stale_event_sweep.rs`)
- [ ] Task 7 — Dispatcher (`eventing/dispatcher.rs`)
- [ ] Task 8 — Gateway (`eventing/gateway.rs`)
- [ ] Task 9 — Diesel infra (`eventing/assembly/infra_diesel.rs`)
- [ ] Task 10 — Public API facade (`lib.rs`)
- [ ] Verification

---

## Critical Files

- `write_side/src/eventing/mod.rs` — add `record_events`, `event_consumer`, `stale_event_sweep`; remove `subscriber` and `recorder` stubs
- `write_side/src/eventing/model.rs` — leave as re-export shim after moving `EventStatus` to domain
- `write_side/src/eventing/assembly/mod.rs` — add `domain`; add `infra_diesel` under `#[cfg(feature = "diesel")]`
- `write_side/src/eventing/assembly/application.rs` — expand from minimal stub to full application layer + `EventDispatchPort`
- `write_side/src/eventing/gateway.rs` — implement `EventGateway` (implements `EventDispatchPort`)
- `write_side/src/eventing/dispatcher.rs` — implement `EventDispatcher`
- `write_side/src/lib.rs` — extend public API facade to include eventing exports
- Reference: `write_side/src/commanding/` (entire module) — exact pattern to mirror

---

## Tasks

### Task 1 — Module scaffold

- [ ] Update `write_side/src/eventing/mod.rs`: add `record_events`, `event_consumer`, `stale_event_sweep`; remove `recorder` and `subscriber` stubs
- [ ] Update `write_side/src/eventing/assembly/mod.rs`: re-export `domain`, `application`; add `infra_diesel` under `#[cfg(feature = "diesel")]`
- [ ] Delete `eventing/recorder.rs` and `eventing/subscriber.rs` stubs

---

### Task 2 — Domain layer (`eventing/assembly/domain.rs`)

- [ ] Move `EventStatus` (with all `TryFrom`, `From`, `as_str` impls) from `model.rs` into `domain.rs`
- [ ] Define `Criterion` enum: `StatusIn(Vec<EventStatus>)`, `ScheduledBeforeNow`, `MaxAttempts(i32)`, `ReservedBefore(DateTime<Utc>)`, `OrderByScheduledAtAsc`
- [ ] Leave `model.rs` as a re-export shim for backwards compat

---

### Task 3 — Application layer (`eventing/assembly/application.rs`)

- [ ] Define `NewEventMetadata { event_id: Uuid, correlation_id: Option<Uuid>, causation_id: Option<Uuid>, source: Option<String> }` with `serde` derives
- [ ] Define `NewEventEnvelope { event_type: String, payload: String, metadata: Option<NewEventMetadata> }` — gateway input
- [ ] Define `EventMetadata { event_id: Uuid, correlation_id: Option<Uuid>, causation_id: Option<Uuid>, source: Option<String> }` (read-side decoded)
- [ ] Expand `EventEnvelope` (flat — no infra types): `id: Uuid`, `reservation_id: Uuid`, `event_type: String`, `payload: String`, `attempts: i32`, `metadata: Option<EventMetadata>` — constructed by infra, returned from reservation
- [ ] Define `EventError` enum variants:
  - `Storage(String)`
  - `Reservation(String)`
  - `SubscriberNotFound(String)`
  - `SubscriberExecution(String)`
  - `MissingReservation { id: Uuid }`
  - `Conversion(String)`
- [ ] Define `EventDispatchPort` trait: `fn dispatch(&self, event: NewEventEnvelope) -> Result<(), CommandError>` — this is the port commanding's `CommandDispatcher` calls into; must match the signature already referenced in `commanding/dispatcher.rs`
- [ ] Define application-layer ports:
  - `EventStorePort`: `fn record(&self, envelope: &NewEventEnvelope) -> Result<(), EventError>`
  - `EventProcessPort`: `fn completed(&self, id: Uuid, reservation_id: Uuid) -> Result<(), EventError>` + `fn failed(&self, id: Uuid, reservation_id: Uuid, max_attempts: i32) -> Result<(), EventError>`
  - Note: `EventReservePort`, `EventSweepPort`, `EventSubscriberPort` reference feature-layer spec types and are defined in their respective feature modules (Tasks 5, 6, 7)

---

### Task 4 — Record events (`eventing/record_events.rs`)

- [ ] Define `EventRecorderRepository { store: Arc<dyn EventStorePort> }`
- [ ] Define `EventRecorder { repo: Arc<EventRecorderRepository> }` with constructor `new(repo: Arc<EventRecorderRepository>)`, matching the command recorder pattern
- [ ] Implement `record(&self, envelope: &NewEventEnvelope) -> Result<(), EventError>`: delegates through the repository to `EventStorePort::record`

---

### Task 5 — Event consumer (`eventing/event_consumer.rs`)

- [ ] Define `ReservableEventSpec { limit: usize, max_attempts: i32 }` with `DEFAULT_MAX_ATTEMPTS: i32 = 6` and `fn criteria(&self) -> Vec<Criterion>`
- [ ] Define `EventReservePort` trait: `fn reserve(&self, spec: &ReservableEventSpec) -> Result<Vec<EventEnvelope>, EventError>`
- [ ] Define `EventConsumerRepository { reserve: Arc<dyn EventReservePort>, process: Arc<dyn EventProcessPort> }`
- [ ] Define `EventConsumer { repository: EventConsumerRepository, dispatcher: Arc<EventDispatcher> }` with constructor
- [ ] Implement `consume(&self, spec: &ReservableEventSpec) -> Result<(), Vec<EventError>>`:
  - Reserve entries via `repository.reserve(spec)`
  - For each `EventEnvelope`, reconstruct `NewEventEnvelope` and pass to `dispatcher.dispatch()`
  - On success: call `repository.process.completed(id, reservation_id)`
  - On failure: call `repository.process.failed(id, reservation_id, spec.max_attempts)`
  - Collect all errors; return `Err(errors)` if non-empty

---

### Task 6 — Stale event sweep (`eventing/stale_event_sweep.rs`)

- [ ] Define `StaleEventSpec { timeout: Duration, max_attempts: i32 }` with `fn criteria(&self) -> Vec<Criterion>` returning `[ReservedBefore(Utc::now() - timeout)]`
- [ ] Define `EventSweepPort` trait: `fn sweep(&self, spec: &StaleEventSpec) -> Result<u64, EventError>`
- [ ] Define `EventSweeper { port: Arc<dyn EventSweepPort> }` with constructor
- [ ] Implement `sweep(&self, spec: &StaleEventSpec) -> Result<u64, EventError>`: delegates to `self.port.sweep(spec)`

---

### Task 7 — Dispatcher (`eventing/dispatcher.rs`)

- [ ] Define `EventSubscriberPort` trait: `fn handle(&self, envelope: &NewEventEnvelope) -> Result<(), EventError>`
- [ ] Define `EventDispatcher { subscriber: Arc<dyn EventSubscriberPort> }` with constructor
- [ ] Implement `dispatch(&self, envelope: &NewEventEnvelope) -> Result<(), EventError>`: calls `self.subscriber.handle(envelope)` and returns the result

---

### Task 8 — Gateway (`eventing/gateway.rs`)

- [ ] Define `EventGateway` enum with variants: `Direct { dispatcher: Arc<EventDispatcher> }`, `TwoPhased { recorder: Arc<EventRecorder> }`
- [ ] Add constructors `direct(dispatcher: Arc<EventDispatcher>)` and `two_phased(recorder: Arc<EventRecorder>)`
- [ ] Add an inherent `dispatch(&self, envelope: NewEventEnvelope) -> Result<(), EventError>` method that validates `metadata.event_id` is present before either direct delivery or durable recording
- [ ] Implement `EventDispatchPort` for `EventGateway`:
  - `Direct`: call `dispatcher.dispatch(&envelope)`
  - `TwoPhased`: call `recorder.record(&envelope)`
  - Map `EventError` into `CommandError::EventDispatch` for the command side
- [ ] Remove old empty stub

---

### Task 9 — Diesel infra (`eventing/assembly/infra_diesel.rs`) `[cfg(feature = "diesel")]`

Follow `write_side/src/commanding/assembly/infra_diesel.rs` exactly:

- [ ] Define `EventEntry` struct (Queryable, all DB columns) and `NewEventEntry` struct (Insertable, application-provided fields)
- [ ] Define Diesel schema for `event_entries` table (mirrors `command_entries` schema)
- [ ] Implement conversion `NewEventEnvelope` → `NewEventEntry`
- [ ] Implement conversion `EventEntry` → `EventEnvelope`
- [ ] Define `EventStoreStorage(DbPool)`; implement `EventStorePort`: INSERT with `ON CONFLICT DO NOTHING`
- [ ] Define `EventConsumerStorage(DbPool)`; implement `EventReservePort`: CTE with `SELECT … FOR UPDATE SKIP LOCKED`, update to `Reserved`, assign UUID v7 `reservation_id`, increment `attempts`, return `Vec<EventEnvelope>`
- [ ] Implement `EventProcessPort` on `EventConsumerStorage`:
  - `completed()`: set status=Completed, clear reservation fields, set `processed_at`
  - `failed()`: if `attempts >= max_attempts` → Dead; else → Failed with `scheduled_at = now + attempts × 30s` (cap 120s)
- [ ] Implement `EventSweepPort` on `EventConsumerStorage`: UPDATE where `status=Reserved AND reserved_at < cutoff`, transition to `Failed`, schedule retry from the current `attempts`, and clear reservation fields (do not increment `attempts`)

---

### Task 10 — Public API facade (`lib.rs`)

- [ ] Extend `write_side/src/lib.rs` `pub mod io` to also re-export eventing:
  - Gateway: `EventGateway`
  - Dispatcher: `EventDispatcher`
  - Record: `EventRecorder`, `EventRecorderRepository`
  - Consumer: `EventConsumer`, `EventConsumerRepository`, `ReservableEventSpec`
  - Sweep: `EventSweeper`, `StaleEventSpec`
  - Application: `NewEventEnvelope`, `NewEventMetadata`, `EventEnvelope`, `EventMetadata`, `EventError`, `EventStatus`, all port traits (`EventDispatchPort`, `EventStorePort`, `EventProcessPort`, `EventReservePort`, `EventSweepPort`, `EventSubscriberPort`)
  - Infra (feature-gated): `EventEntry`, `NewEventEntry`, `EventStoreStorage`, `EventConsumerStorage`

---

## Verification

- [ ] `make check` — all crates compile cleanly
- [ ] `make test` — all tests pass
