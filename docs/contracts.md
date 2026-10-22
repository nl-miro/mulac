# Component Contracts

This document defines the interface contracts for each mulac component. Each contract specifies what a component accepts, what it produces, and what it guarantees to callers and collaborators.

For component responsibilities and lifecycle rules, see [components.md](components.md).

## Lifecycle and runtime boundary

**D-07: decided 2026-10-22; implementation pending.** See [Kernel lifecycle](../kernel/SPEC.md#lifecycle) and [supported runtime contexts](../kernel/SPEC.md#supported-runtime-contexts) for the shared kernel-managed contract.

- `shutdown()` is an idempotent cancellation request. Workers stop taking new batches/deliveries and finish already-started attempts, including completion/failure recording and acknowledgements. Queued work is not drained; there is no kernel shutdown deadline. Existing operation-specific timeouts are unchanged.
- `wait()` does not request cancellation. It waits for all kernel-owned workers to stop and reports any unrecoverable worker error or panic afterward. Such exits do not cancel siblings or trigger automatic restart; per-entry retry behavior is unchanged. Application-owned request tasks are not joined by `wait()`.
- New external dispatch through kernel-provided state, gateways, and clones is rejected after shutdown begins, before side effects or durable acceptance. Already-accepted executions may finish event handoff and issue follow-up commands; persistent follow-ups may remain queued for restart, and direct follow-ups may finish inline. This exemption belongs to the accepted execution, not to any holder of an existing handle.
- Unexpected inbox stream closure is a transport error. Explicit cancellation is the clean shutdown path, not a reason to report failed recording/acknowledgement as successful.
- Async workers/publication and runtime bridges promise support under a live multi-thread Tokio runtime. Synchronous AMQP publication belongs on associated blocking threads, not directly in async tasks; `block_on_blocking` is restricted to multi-thread runtime tasks. Standalone synchronous publication and current-thread runtimes are outside the promised support boundary.

**Current gaps:** kernel handles do not join workers or implement the dispatch gate/in-flight exemption; runtime restrictions are not automatically enforced. These decisions do not change public APIs or prove runtime compliance. Applications retain responsibility for stopping and awaiting their own request tasks. Ordinary synchronous functions that do not bridge a runtime are not restricted by this runtime boundary.

## Entry retry policy

This policy applies to every durable queue-backed entry type (`InboxEntry`, `CommandEntry`, `EventEntry`, `OutboxEntry`).

- **Attempt counting:** an attempt is one reservation. `reserve` increments the entry's attempt counter; `failed` and the stale-reservation sweep do not. Because every reservation counts, an entry whose consumer crashes or times out repeatedly still exhausts its attempts instead of cycling indefinitely.
- **Attempt limit:** `max_attempts` (default: 6 — one initial delivery plus five retries) is configurable per component. Entries whose attempt counter has reached `max_attempts` are never reserved again.
- **Backoff:** a retriable transition schedules the next attempt at `now + attempts × 30 seconds` (linear, no cap). The entry becomes eligible for reservation again once that time has passed.
- **`failed(entry_id)`:** transitions the entry to `failed` with the backoff above, or to `dead` when `attempts >= max_attempts`.
- **Stale-reservation release (sweep):** transitions the entry to `failed` with the same backoff, or to `dead` when `attempts >= max_attempts`. The sweep never returns an entry to `received`.
- **`dead`** entries are not retried automatically; an operator may manually reset a `dead` entry back to `received`, which clears the attempt counter and allows the full retry sequence to run again.

## Metadata propagation

These rules keep one trace across commands, events and follow-up commands.

**Events produced by a command handler** (assigned by the commanding library when it wraps the handler):

| Field | Value |
| --- | --- |
| `event_id` | Fresh UUID v7 for each produced event, including on re-execution of the same command |
| `causation_id` | The command's `command_id` |
| `correlation_id` | The command's `correlation_id`, or its `command_id` when it has none |
| `source` | The command's `source`; a command without a source produces events without one |

**Follow-up commands dispatched by an event subscriber** (an obligation on the subscriber; the kernel does not stamp these fields):

| Field | Value |
| --- | --- |
| `command_id` | Chosen by the subscriber |
| `causation_id` | The triggering event's `event_id` |
| `correlation_id` | The triggering event's `correlation_id`, or its `event_id` when it has none |
| `source` | Chosen by the subscriber, identifying it (e.g. `event:TweetPosted`) |

**Commands from inbox messages:** `causation_id` is the inbox `message_id`; `correlation_id` is the message's correlation when the transport supplies one, otherwise the `message_id`.

---

## Inbox

**Accepts:**
An `InboundMessage` containing:
- A UTF-8 text payload; non-UTF-8 deliveries are rejected at the transport boundary and dead-lettered without retry
- Required transport metadata: stable UUID message ID, source identifier, received-at timestamp. Message IDs have no additional UUID-version restriction; source is metadata, not part of the deduplication key.

**Produces:**
An `InboxEntry` — a durable record containing the raw payload and transport metadata.

**Consumer-facing operations:**
- `reserve(count) → InboxEntry[]` — atomically transitions up to `count` eligible entries (`received`, or `failed` whose scheduled retry time has passed, with attempts below the limit) to `reserved` and returns them; a `reserved` entry cannot be claimed by another consumer
- `completed(entry_id)` — signals successful processing; the Inbox transitions the entry to `completed`
- `failed(entry_id)` — signals processing failure; the Inbox transitions the entry to `failed` and schedules it for retry
- a call to `completed` or `failed` for an entry no longer owned by the caller (released by the sweep) is rejected

**Deduplication:**
The message UUID alone is the deduplication key within an `inbox_entries` table, across all sources. A repeated UUID is accepted as a duplicate without creating or changing a row, regardless of source, payload differences, age, or lifecycle state. The original payload, metadata, timestamps, and processing state are preserved. Different UUIDs remain distinct messages even when their payloads are identical.

Deduplication protection lasts while the corresponding row exists. There is no configurable retention window or automatic expiry after 24 hours. Deleting the row removes its deduplication protection, allowing that UUID to be recorded again. This contract introduces no cleanup policy and no deduplication guarantee across separate inbox tables.

**Message-ID validation — decided; enforcement pending:**
- Missing, empty, or non-UUID transport message IDs must be rejected before recording; no replacement UUID may be generated and no success acknowledgement may be sent.
- For AMQP, rejection must not requeue the delivery. Processing continues with subsequent deliveries after successful rejection; rejection failures surface as errors rather than being silently treated as successful disposal.
- Dead-lettering depends on broker configuration; rejection alone does not guarantee a dead-letter destination.
- **Current implementation gap:** UUID-only storage deduplication already exists, but AMQP parsing converts missing/malformed IDs to an absent ID and recording conversion generates a fresh UUID. The rejection policy above is not yet enforced. Existing public types, conversion APIs, and storage schemas are unchanged by this documentation decision.

**Stale reservation sweep:**
A background process runs on a configurable interval (default: every 10 minutes) and releases any `reserved` entry whose reservation is older than a configurable staleness threshold (default: 15 minutes) to `failed` (scheduled per the [entry retry policy](#entry-retry-policy)), or to `dead` if its attempts are exhausted.

**Guarantees:**
- Acceptance is signalled to the caller only after the `InboxEntry` is durably stored
- Unexpected intake stream closure is a transport error; explicit cancellation follows the [lifecycle boundary](#lifecycle-and-runtime-boundary), allowing an already-started delivery to finish recording and acknowledgement without taking another delivery
- A message UUID whose row still exists will not produce a new `InboxEntry`, even from a different source or after more than 24 hours; its original row remains unchanged
- A `reserved` entry will not be concurrently claimed by another consumer
- Delivery to consumers is at-least-once; the same entry may be surfaced more than once (after a failed report or a stale reservation release)

**Requires from collaborators:**
- Transport adapter must supply a stable UUID message ID with each `InboundMessage`; producers sharing an inbox table must use distinct IDs for distinct messages, regardless of source
- Transport adapter must not acknowledge delivery to the broker until the Inbox signals acceptance
- Inbox consumer must tolerate at-least-once delivery (duplicate `InboxEntry` processing must be safe)
- Inbox consumer must call `completed` or `failed` for every reserved entry; unreported entries will be released by the sweep and retried

---

## Command Dispatcher

**Accepts:**
A `CommandEnvelope` containing:
- The command payload
- Required metadata: command type, correlation ID, causation ID, created-at timestamp
- Additional caller-supplied metadata (open, no prescribed shape)

The `command type` field is the routing key used to resolve the handler; it is preserved in the durable `CommandEntry` record.

**Produces:**
- Direct: `EventEnvelope` instances handed off inline to the Event Dispatcher
- TwoPhased: a durable `CommandEntry`; a queue consumer later invokes the handler and produces `EventEnvelope` instances handed off to the Event Dispatcher

**Consumer-facing operations (TwoPhased only):**
- `reserve(count) → CommandEntry[]` — atomically transitions up to `count` eligible entries (`received`, or `failed` whose scheduled retry time has passed, with attempts below the limit) to `reserved` and returns them; a `reserved` entry cannot be claimed by another consumer
- `completed(entry_id)` — signals successful processing; the dispatcher transitions the entry to `completed`
- `failed(entry_id)` — signals processing failure; the dispatcher transitions the entry to `failed` and schedules it for retry
- a call to `completed` or `failed` for an entry no longer owned by the caller (released by the sweep) is rejected

**Stale reservation sweep (TwoPhased only):**
A background process runs on a configurable interval (default: every 10 minutes) and releases any `reserved` entry whose reservation is older than a configurable staleness threshold (default: 15 minutes) to `failed` (scheduled per the [entry retry policy](#entry-retry-policy)), or to `dead` if its attempts are exhausted.

**Guarantees:**
- Direct: the command is executed at most once; if the handler fails, or if the handler succeeds but inline handoff to the Event Dispatcher fails afterward, the error is propagated to the caller and retrying is the caller's responsibility
- Direct: if inline handoff to the Event Dispatcher fails after the handler has already executed, handler side-effects are not rolled back and the produced `EventEnvelope` instances are not retained for automatic redelivery
- TwoPhased: acceptance is signalled only after the `CommandEntry` is durably stored
- TwoPhased: execution is at-least-once; the same `CommandEntry` may be processed more than once (after a failed report or a stale reservation release)
- TwoPhased: a `reserved` `CommandEntry` will not be concurrently claimed by another consumer

**Business effects and event recovery — decided 2026-10-22:**
- Application handlers own business-effect idempotency and recovery of required events across retries, including when a business mutation commits but event handoff fails. Making a repeated mutation a no-op is not, by itself, sufficient to recover its required events.
- The library owns durable command acceptance in TwoPhased mode, reservation/retry processing, and dispatch of the events returned by each execution. It does not guarantee atomicity between application business changes and event recording, or exactly-once business effects.
- The dispatcher succeeds when all events returned by the current execution are accepted by the configured event boundary. An empty event result also succeeds and permits command completion; the library does not reconstruct outputs from earlier attempts. Completion is not proof that the application recovered every event required by earlier business changes.
- `wrap_handler` assigns fresh event UUIDs whenever an execution produces events, including command re-executions. Stable event IDs across command executions are not guaranteed; event-ID deduplication does not collapse separately generated events from those executions.
- Redelivery of an already-recorded event is different from command re-execution: its stored event identity remains unchanged. Existing event/outbox identity guarantees are unaffected.
- Direct retries remain caller-controlled; TwoPhased retries follow the queue retry policy. Neither mode assumes responsibility for repairing application business effects or recovering unrecorded outputs from a previous execution.

**Requires from collaborators:**
- Caller must provide a well-formed `CommandEnvelope` with all required metadata fields
- Caller must ensure a handler is registered for the command type before dispatching; an unroutable command is a caller error
- Command handler must be idempotent regardless of variant and must own recovery of its required events across retries; application compliance with these obligations requires separate evidence
- Event Dispatcher must be available to accept handoff; if it is not, TwoPhased retries the `CommandEntry` which can re-invoke the handler. Successful earlier side effects or handoffs are not rolled back, and the application must account for partial success

---

## Event Dispatcher

**Accepts:**
An `EventEnvelope` containing:
- The event payload
- Required metadata: event type, routing key, correlation ID, causation ID, created-at timestamp
- Additional caller-supplied metadata (open, no prescribed shape)

The `routing key` is supplied by the event producer and used by downstream subscribers (such as the Outbox) to resolve the outbound transport destination.

**Produces:**
- Direct: delivers the `EventEnvelope` inline to all resolved subscribers; if one subscriber fails the error propagates and already-invoked subscribers are not rolled back
- TwoPhased: a durable `EventEntry`; a queue consumer later delivers to all subscribers as a unit; retry can re-invoke already-successful subscribers

**Consumer-facing operations (TwoPhased only):**
- `reserve(count) → EventEntry[]` — atomically transitions up to `count` eligible entries (`received`, or `failed` whose scheduled retry time has passed, with attempts below the limit) to `reserved` and returns them; a `reserved` entry cannot be claimed by another consumer
- `completed(entry_id)` — signals successful delivery to all subscribers; the dispatcher transitions the entry to `completed`
- `failed(entry_id)` — signals delivery failure; the dispatcher transitions the entry to `failed` and schedules it for retry
- a call to `completed` or `failed` for an entry no longer owned by the caller (released by the sweep) is rejected

**Stale reservation sweep (TwoPhased only):**
A background process runs on a configurable interval (default: every 10 minutes) and releases any `reserved` entry whose reservation is older than a configurable staleness threshold (default: 15 minutes) to `failed` (scheduled per the [entry retry policy](#entry-retry-policy)), or to `dead` if its attempts are exhausted.

**Guarantees:**
- Direct: each subscriber is invoked at most once per dispatch; if one subscriber fails the error propagates to the caller and retrying is the caller's responsibility; already-invoked subscribers are not rolled back
- TwoPhased: acceptance is signalled only after the `EventEntry` is durably stored
- TwoPhased: delivery is at-least-once; the same `EventEntry` may be delivered more than once (after a failed report or a stale reservation release)
- TwoPhased: if delivery to one subscriber fails after others have succeeded, a retry will re-invoke all subscribers — this is a known partial delivery hazard
- TwoPhased: a `reserved` `EventEntry` will not be concurrently claimed by another consumer
- Zero subscribers for an event type is a valid, expected outcome; the `EventEntry` is marked `completed` silently

**Requires from collaborators:**
- Caller must provide a well-formed `EventEnvelope` with all required metadata fields; zero subscribers for the given event type is not a caller error
- Subscribers must tolerate at-least-once delivery
- Subscribers must not assume anything about whether other subscribers have been invoked; each subscriber operates independently

---

## Outbox

**Accepts:**
An `EventEnvelope` — received as a subscriber of the Event Dispatcher; the Outbox owns the transformation to an outbound message for the external transport. The Outbox uses the `routing key` field from the `EventEnvelope` metadata to resolve the outbound transport destination; a missing or unresolvable routing key is a caller error and the `EventEnvelope` is rejected at acceptance time.

**Transformation contract:**
- Each accepted `EventEnvelope` maps to exactly one outbound message; cardinality is always 1:1.
- Routing key errors are caught at acceptance before an `OutboxEntry` is created; the `EventEnvelope` is rejected and no durable record is written.
- Serialisation errors occur after the `OutboxEntry` exists; the outbox consumer calls `dead(entry_id)` to transition the entry to `dead` immediately, bypassing the retry schedule.
- Transport failures follow the standard [entry retry policy](#entry-retry-policy). They include: broker unavailable, broker negative acknowledgement, an unroutable mandatory publish returned by the broker, confirmations not active on the channel, and a confirmation that never arrives (channel closed or confirm timeout).

**Produces:**
- Stage 1: a durable `OutboxEntry` — the event is stored durably upon acceptance
- Stage 2: an outbound message accepted by the external transport — the outbox consumer publishes the message and waits for broker acceptance

**Broker acceptance:**
Broker acceptance means the broker positively confirmed the publish *and*, when the publish is mandatory (the default), routed it to at least one queue. Every other outcome is a retriable transport failure (`failed`), never acceptance:

| Broker outcome | Outbox outcome |
| --- | --- |
| Publisher confirm `ack`, no returned message | Accepted → `completed` |
| `ack` with a returned message (mandatory publish matched no queue) | Transport failure → `failed` |
| `nack` | Transport failure → `failed` |
| Confirmations not active on the channel | Transport failure → `failed` |
| No confirmation (channel or connection closed, or confirm timeout) | Transport failure → `failed`; the message may still have been published, which at-least-once delivery covers |

Publishing without the mandatory flag is an explicit opt-out: the broker then silently drops unroutable messages and still acknowledges them, and that loss is outside the Outbox contract.

**Consumer-facing operations:**
- `reserve(count) → OutboxEntry[]` — atomically transitions up to `count` eligible entries (`received`, or `failed` whose scheduled retry time has passed, with attempts below the limit) to `reserved` and returns them; a `reserved` entry cannot be claimed by another consumer
- `completed(entry_id)` — signals broker acceptance; the Outbox transitions the entry to `completed`
- `failed(entry_id)` — signals delivery failure; the Outbox transitions the entry to `failed` and schedules it for retry
- `dead(entry_id)` — signals a non-retriable post-acceptance transformation failure such as serialisation; the Outbox transitions the entry to `dead` immediately without scheduling retry
- a call to `completed`, `failed`, or `dead` for an entry no longer owned by the caller (released by the sweep) is rejected

**Stale reservation sweep:**
A background process runs on a configurable interval (default: every 10 minutes) and releases any `reserved` entry whose reservation is older than a configurable staleness threshold (default: 15 minutes) to `failed` (scheduled per the [entry retry policy](#entry-retry-policy)), or to `dead` if its attempts are exhausted.

**Guarantees:**
- Acceptance is signalled only after the `OutboxEntry` is durably stored
- Delivery to the external transport is at-least-once; if broker acceptance succeeds but the `completed` transition fails, the entry is retried and the outbound message may be published again
- A `reserved` `OutboxEntry` will not be concurrently claimed by another consumer
- No ordering guarantee across entries or after retries; delivery order is not guaranteed
- Broker acceptance does not imply durable broker storage. Messages are published persistent by default, but surviving a broker restart also requires durable queues, which the Outbox does not verify; that is a deployment concern

**Requires from collaborators:**
- Event Dispatcher must not consider the event delivered until the Outbox confirms durable storage of the `OutboxEntry`
- Outbox consumer must call `completed` only after the broker has accepted the message; calling `completed` before acceptance risks message loss
- Outbox consumer must call `dead(entry_id)` for non-retriable post-acceptance transformation failures such as serialisation; retriable transport failures must use `failed(entry_id)`
- Behaviour beyond broker acceptance is outside the Outbox contract boundary

---

## Document Metadata

| Version | Author               | Reviewers            | Summary                                                                                 | Date       |
|---------|----------------------|----------------------|-----------------------------------------------------------------------------------------|------------|
| 0.1.0   | Miro, Claude & Codex | Miro, Claude & Codex | Initial component contracts for Inbox, Command Dispatcher, Event Dispatcher, and Outbox | 2026-04-14 |
