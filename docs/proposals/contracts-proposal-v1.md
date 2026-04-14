# Component Contracts

This document defines the interface contracts for each mulac component. Each contract specifies what a component accepts, what it produces, and what it guarantees to callers and collaborators.

For component responsibilities and lifecycle rules, see [components.md](components.md).

## Inbox

**Accepts:**
An `InboundMessage` containing:
- An opaque byte payload
- Required transport metadata: stable message ID, source identifier, received-at timestamp

**Produces:**
An `InboxEntry` — a durable record containing the raw payload and transport metadata.

**Consumer-facing operations:**
- `reserve(count) → InboxEntry[]` — atomically transitions up to `count` available entries to `in_progress` and returns them; an `in_progress` entry cannot be claimed by another consumer
- `completed(entry_id)` — signals successful processing; the Inbox transitions the entry to `completed`
- `failed(entry_id)` — signals processing failure; the Inbox transitions the entry to `failed` and makes it eligible for retry
- a call to `completed` or `failed` for an entry no longer owned by the caller (released by the sweep) is rejected

**Stale reservation sweep:**
A background process runs on a configurable interval (default: every 10 minutes) and releases any `in_progress` entry whose reservation is older than a configurable staleness threshold (default: 15 minutes) back to the available pool.

**Guarantees:**
- Acceptance is signalled to the caller only after the `InboxEntry` is durably stored
- A message carrying a message ID already seen from the same source within the deduplication retention window will not produce a new `InboxEntry`; messages outside the window or from a different source are accepted as new
- An `in_progress` entry will not be concurrently claimed by another consumer
- Delivery to consumers is at-least-once; the same entry may be surfaced more than once (after a failed report or a stale reservation release)

**Requires from collaborators:**
- Transport adapter must supply a stable, unique message ID with each `InboundMessage`
- Transport adapter must not acknowledge delivery to the broker until the Inbox signals acceptance
- Inbox consumer must tolerate at-least-once delivery (duplicate `InboxEntry` processing must be safe)
- Inbox consumer must call `completed` or `failed` for every reserved entry; unreported entries will be released by the sweep and retried

---

## Command Dispatcher

**Accepts:**
A `CommandEnvelope` containing:
- The command payload
- Required metadata: correlation ID, causation ID, created-at timestamp
- Additional caller-supplied metadata (open, no prescribed shape)

**Produces:**
- Direct: `EventEnvelope` instances handed off inline to the Event Dispatcher
- TwoPhased: a durable `CommandEntry`; a queue consumer later invokes the handler and produces `EventEnvelope` instances handed off to the Event Dispatcher

**Consumer-facing operations (TwoPhased only):**
- `reserve(count) → CommandEntry[]` — atomically transitions up to `count` available entries to `in_progress` and returns them; an `in_progress` entry cannot be claimed by another consumer
- `completed(entry_id)` — signals successful processing; the dispatcher transitions the entry to `completed`
- `failed(entry_id)` — signals processing failure; the dispatcher transitions the entry to `failed` and makes it eligible for retry
- a call to `completed` or `failed` for an entry no longer owned by the caller (released by the sweep) is rejected

**Stale reservation sweep (TwoPhased only):**
A background process runs on a configurable interval (default: every 10 minutes) and releases any `in_progress` entry whose reservation is older than a configurable staleness threshold (default: 15 minutes) back to the available pool.

**Guarantees:**
- Direct: the command is executed at most once; if the handler fails the error is propagated to the caller and retrying is the caller's responsibility
- TwoPhased: acceptance is signalled only after the `CommandEntry` is durably stored
- TwoPhased: execution is at-least-once; the same `CommandEntry` may be processed more than once (after a failed report or a stale reservation release)
- TwoPhased: an `in_progress` `CommandEntry` will not be concurrently claimed by another consumer

**Requires from collaborators:**
- Caller must provide a well-formed `CommandEnvelope` with all required metadata fields
- Caller must ensure a handler is registered for the command type before dispatching; an unroutable command is a caller error
- Command handler must be idempotent regardless of variant
- Event Dispatcher must be available to accept handoff; if it is not, TwoPhased retries the `CommandEntry` which can re-invoke the handler — making handler idempotency essential

---

## Event Dispatcher

**Accepts:**
An `EventEnvelope` containing:
- The event payload
- Required metadata: event type, correlation ID, causation ID, created-at timestamp
- Additional caller-supplied metadata (open, no prescribed shape)

**Produces:**
- Direct: delivers the `EventEnvelope` inline to all resolved subscribers; if one subscriber fails the error propagates and already-invoked subscribers are not rolled back
- TwoPhased: a durable `EventEntry`; a queue consumer later delivers to all subscribers as a unit; retry can re-invoke already-successful subscribers

**Consumer-facing operations (TwoPhased only):**
- `reserve(count) → EventEntry[]` — atomically transitions up to `count` available entries to `in_progress` and returns them; an `in_progress` entry cannot be claimed by another consumer
- `completed(entry_id)` — signals successful delivery to all subscribers; the dispatcher transitions the entry to `completed`
- `failed(entry_id)` — signals delivery failure; the dispatcher transitions the entry to `failed` and makes it eligible for retry
- a call to `completed` or `failed` for an entry no longer owned by the caller (released by the sweep) is rejected

**Stale reservation sweep (TwoPhased only):**
A background process runs on a configurable interval (default: every 10 minutes) and releases any `in_progress` entry whose reservation is older than a configurable staleness threshold (default: 15 minutes) back to the available pool.

**Guarantees:**
- Direct: each subscriber is invoked at most once per dispatch; if one subscriber fails the error propagates to the caller and retrying is the caller's responsibility; already-invoked subscribers are not rolled back
- TwoPhased: acceptance is signalled only after the `EventEntry` is durably stored
- TwoPhased: delivery is at-least-once; the same `EventEntry` may be delivered more than once (after a failed report or a stale reservation release)
- TwoPhased: if delivery to one subscriber fails after others have succeeded, a retry will re-invoke all subscribers — this is a known partial delivery hazard
- TwoPhased: an `in_progress` `EventEntry` will not be concurrently claimed by another consumer
- Zero subscribers for an event type is a valid, expected outcome; the `EventEntry` is marked `completed` silently

**Requires from collaborators:**
- Caller must provide a well-formed `EventEnvelope` with all required metadata fields; zero subscribers for the given event type is not a caller error
- Subscribers must tolerate at-least-once delivery
- Subscribers must not assume anything about whether other subscribers have been invoked; each subscriber operates independently

---

## Outbox

**Accepts:**
An `EventEnvelope` — received as a subscriber of the Event Dispatcher; the Outbox owns the transformation to an outbound message for the external transport.

**Produces:**
- Stage 1: a durable `OutboxEntry` — the event is stored durably upon acceptance
- Stage 2: an outbound message accepted by the external transport — the outbox consumer publishes the message and waits for broker acceptance

**Consumer-facing operations:**
- `reserve(count) → OutboxEntry[]` — atomically transitions up to `count` available entries to `in_progress` and returns them; an `in_progress` entry cannot be claimed by another consumer
- `completed(entry_id)` — signals broker acceptance; the Outbox transitions the entry to `completed`
- `failed(entry_id)` — signals delivery failure; the Outbox transitions the entry to `failed` and makes it eligible for retry
- a call to `completed` or `failed` for an entry no longer owned by the caller (released by the sweep) is rejected

**Stale reservation sweep:**
A background process runs on a configurable interval (default: every 10 minutes) and releases any `in_progress` entry whose reservation is older than a configurable staleness threshold (default: 15 minutes) back to the available pool.

**Guarantees:**
- Acceptance is signalled only after the `OutboxEntry` is durably stored
- Delivery to the external transport is at-least-once; if broker acceptance succeeds but the `completed` transition fails, the entry is retried and the outbound message may be published again
- An `in_progress` `OutboxEntry` will not be concurrently claimed by another consumer
- No ordering guarantee across entries or after retries; delivery order is not guaranteed

**Requires from collaborators:**
- Event Dispatcher must not consider the event delivered until the Outbox confirms durable storage of the `OutboxEntry`
- Outbox consumer must call `completed` only after the broker has accepted the message; calling `completed` before acceptance risks message loss
- Behaviour beyond broker acceptance is outside the Outbox contract boundary

---

## Document Metadata

| Version | Author | Reviewers | Summary | Date |
|---------|--------|-----------|---------|------|
| 0.1.0   | Miro, Claude & Codex | Miro, Claude & Codex | All component contracts defined | 2026-04-14 |
