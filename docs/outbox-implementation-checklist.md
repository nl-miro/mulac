# Outbox Implementation Checklist

Use this checklist to track implementation progress for [outbox-implementation-plan.md](outbox-implementation-plan.md). Each item should be completed in a small, reviewable commit.

## Preparation

- [x] Update `outbox/Cargo.toml` dependencies and feature flags.
- [x] Add `assembly/` module skeleton.
- [x] Remove generated sample `add()` function/test from `outbox/src/lib.rs`.
- [x] Drop `outbox/SPEC.md`; use `docs/outbox-spec.md` as canonical specification.

## Domain Layer

- [x] Implement `OutboxStatus` sparse status enum.
- [x] Implement `UnknownOutboxStatus`.
- [x] Implement `TryFrom<i32>` for `OutboxStatus`.
- [x] Implement `From<OutboxStatus> for i32`.
- [x] Implement `OutboxStatus::as_str()`.
- [x] Add status conversion tests.
- [x] Implement `OutboxEntry`.
- [x] Implement `NewOutboxEntry`.
- [x] Resolve metadata ownership between domain/application layers.

## Application Layer

- [x] Implement `NewOutboxEnvelope`.
- [x] Implement `NewOutboxMetadata`.
- [x] Implement `OutboxEntryMetadata`.
- [x] Implement metadata normalization: `message_id.unwrap_or(event_id)`.
- [x] Implement `OutboxEntryEnvelope`.
- [x] Implement `OutboundMessageEnvelope`.
- [x] Implement `OutboxError`.
- [x] Define `OutboxStorePort`.
- [x] Define `OutboxReservePort`.
- [x] Define `OutboxProcessPort`.
- [x] Define `OutboxSweepPort`.
- [x] Define `OutboxPublisherPort`.
- [x] Add metadata normalization tests.

## Record Events Use Case

- [x] Add `record_events.rs`.
- [x] Implement `OutboxRecorderRepository`.
- [x] Implement `OutboxRecorder`.
- [x] Validate non-blank `routing_key` before storage.
- [x] Convert `NewOutboxEnvelope` to `NewOutboxEntry`.
- [x] Ensure `event_id` is not generated or replaced.
- [x] Delegate persistence to `OutboxStorePort::record`.
- [x] Add fake-store tests for successful recording.
- [x] Add fake-store tests for routing validation failure.
- [x] Add fake-store tests for id preservation.

## Consumer Use Case

- [x] Add/replace `outbox_consumer.rs`.
- [x] Implement `ReservableOutboxSpec`.
- [x] Implement default `max_attempts = 6` helper or constructor.
- [x] Implement `OutboxConsumerRepository`.
- [x] Implement `OutboxConsumer`.
- [x] Reserve eligible entries via `OutboxReservePort`.
- [x] Reject entries missing `reservation_id`.
- [x] Convert reserved entries to `OutboundMessageEnvelope`.
- [x] Publish via `OutboxPublisherPort`.
- [x] Mark entries `completed` after publish success.
- [x] Mark entries `failed` after retriable publish failure.
- [x] Mark entries `dead` after non-retriable conversion/serialization failure.
- [x] Continue batch processing after per-entry failures.
- [x] Collect and return batch errors.
- [x] Add tests for success path.
- [x] Add tests for publish failure.
- [x] Add tests for conversion/serialization failure.
- [x] Add tests for missing reservation.
- [x] Add tests confirming one failure does not stop later entries.

## Stale Reservation Sweep Use Case

- [x] Add `stale_reservation_sweep.rs`.
- [x] Implement `StaleReservationSpec`.
- [x] Implement `ReservationSweeper`.
- [x] Delegate sweep to `OutboxSweepPort`.
- [x] Add fake-port tests for success count.
- [x] Add fake-port tests for error propagation.
- [x] Add fake-port tests confirming spec pass-through.

## Public API Facade

- [x] Re-export domain models from `outbox::io`.
- [x] Re-export application envelopes from `outbox::io`.
- [x] Re-export ports from `outbox::io`.
- [x] Re-export use-case components from `outbox::io`.
- [x] Feature-gate Diesel adapter re-exports.
- [x] Feature-gate AMQP adapter re-exports.
- [x] Add public API smoke tests using only `outbox::io::*`.

## Diesel Storage Adapter

- [x] Add Diesel schema for `outbox_entries`.
- [x] Add `DbPool` type alias.
- [x] Add `build_pool` helper.
- [x] Add JSONB metadata mapping/newtype if needed.
- [x] Add insertable record struct.
- [x] Add queryable/selectable record struct.
- [x] Implement `NewOutboxEntry -> insertable record` conversion.
- [x] Implement `record -> OutboxEntryEnvelope` conversion.
- [x] Implement `OutboxStoreStorage`.
- [x] Implement idempotent insert with `ON CONFLICT (id) DO NOTHING`.
- [x] Treat duplicate `event_id` as success.
- [x] Implement `OutboxConsumerStorage`.
- [x] Implement reservation using `FOR UPDATE SKIP LOCKED`.
- [x] Increment `attempts` exactly once per reservation.
- [x] Assign UUID v7 `reservation_id`.
- [x] Implement `completed()` with reservation ownership check.
- [x] Implement `failed()` with reservation ownership check.
- [x] Implement retry scheduling: `min(attempts * 30s, 120s)`.
- [x] Implement exhausted retry transition to `Dead`.
- [x] Implement `dead()` without retry scheduling.
- [x] Implement stale reservation sweep.
- [x] Ensure stale sweep does not increment `attempts`.
- [x] Add database tests for idempotent recording.
- [x] Add database tests for concurrent reservation safety.
- [x] Add database tests for lifecycle transitions.
- [x] Add database tests for stale sweep behavior.

## Database Schema / Migrations

- [x] Add `outbox_entries` table migration or documented SQL.
- [x] Use `UUID PRIMARY KEY` for `id`.
- [x] Use non-null `JSONB` for `meta`.
- [x] Add `(status, scheduled_at)` index.
- [x] Add `(status, reserved_at)` index.
- [x] Consider `(meta ->> 'routing_key')` operational index.
- [x] Consider `(meta ->> 'message_id')` idempotency/debug index.
- [x] Verify schema matches `docs/outbox-spec.md`.

## AMQP Publisher Adapter

- [x] Add/replace `amqp_publisher.rs`.
- [x] Implement `AmqpPublishConfig`.
- [x] Implement `AmqpPublisher`.
- [x] Implement `OutboxPublisherPort` for `AmqpPublisher`.
- [x] Use metadata `routing_key` for publish routing.
- [x] Set AMQP `message_id` from metadata.
- [x] Set AMQP `correlation_id` when present.
- [x] Set AMQP `content_type` when present/defaulted.
- [x] Add event metadata headers: `event_id`, `causation_id`, `event_type`, `source`.
- [x] Wait for broker publisher confirmation.
- [x] Map negative confirmation/channel/connection failures to `OutboxError::Transport`.
- [x] Add metadata-to-AMQP-property tests where possible.

## Constructor Helpers

- [x] Add storage-backed recorder repository constructor.
- [x] Add storage-backed consumer repository constructor.
- [x] Add storage-backed sweeper constructor.
- [x] Ensure constructors can share an existing pool where useful.

## Integration / Lifecycle Tests

- [x] Test `record -> reserve -> publish -> completed`.
- [x] Test duplicate `event_id` creates only one row and succeeds.
- [x] Test transport failure transitions to `Failed`.
- [x] Test failed entries become reservable after `scheduled_at`.
- [x] Test conversion/serialization failure transitions to `Dead`.
- [x] Test stale sweep releases old reservations.
- [x] Test completed-after-publish failure can lead to duplicate publish with stable `message_id`.

## Repository Integration

- [x] Confirm `make check` includes `outbox`.
- [x] Confirm `make test` includes `outbox`.
- [x] Add README/docs links if required.
- [ ] Run `make check`.
- [ ] Run `make test` if environment supports all integration dependencies.

## Final Consistency Pass

- [x] Remove stale TODO comments that point to old `outbox/SPEC.md`.
- [x] Ensure `outbox/SPEC.md` does not diverge from `docs/outbox-spec.md`.
- [x] Ensure all public APIs are re-exported through `outbox::io`.
- [x] Ensure no internal modules import through `crate::io`.
- [x] Ensure docs, schema, and implementation agree on UUID primary key semantics.
- [x] Ensure docs, schema, and implementation agree on retry/dead-letter behavior.
- [x] Ensure docs, schema, and implementation agree on idempotent duplicate recording.
