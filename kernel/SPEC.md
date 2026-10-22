# Kernel Specification

> **Status:** decided 2026-10-22 (D-04, D-07 and D-11 in [test-coverage-report.md](../docs/test-coverage-report.md)).
> The current implementation does not yet match this contract. See [Implementation gaps](#implementation-gaps).

## Overview

The kernel assembles mulac's components (commanding, eventing, inbox, outbox) behind one builder,
`kernel::boot(config)`, and starts them in one of two modes:

- **Direct** (`start()`): commands and events run synchronously, in-flight, with no database. Intended for tests,
  local-only flows, and request-response paths where the caller owns retry.
- **Persistent** (`start_persistent(...)`): commands and events are recorded durably and processed by background
  consumers with retry, stale-reservation recovery, inbox intake, and outbox publication.

**Invariant:** every registration and every config field either takes real effect in the chosen mode, or startup
fails with an error. The kernel never silently swaps in a no-op for something the application registered.

## Public API

The crate root declares only `pub mod io`, following the repository's single-interface rule. Everything an application uses is imported from `kernel::io`:

- builder and startup: `boot`, `KernelConfig`, `KernelBuilder`, `KernelError`
- handles and state: `KernelHandle`, `AppState`, `PersistentKernelHandle`, `PersistentKernelState`
- application boundary: `ApplicationCommand`, `NewCommandEnvelope<C>`, and the re-exported library types (`ApplicationEvent`, `EventSubscriberPort`, `NewCommandMetadata`, `NewEventEnvelope`, errors, gateways, consumers)
- runtime helpers: `run_command_worker`, `run_event_worker`, `block_on_blocking`, `first_command_error`, `first_event_error`

Library `io` facades export every type a supported mode needs, whatever storage features are enabled. For example, `NewCommand` is available without the `diesel` feature, because direct mode needs it as well.

## Capability matrix

| Capability | Direct (`start`) | Persistent (`start_persistent`) |
| --- | --- | --- |
| `command_handler(type, handler)` | Supported: dispatched synchronously | Supported: recorded, then executed by the command consumer |
| `event_subscriber(type, name, subscriber)` | Supported: invoked synchronously | Supported: invoked by the event consumer |
| `event_subscriber_with_command_gateway(type, name, factory)` | Supported: the factory receives the direct `CommandGateway` | Supported: the factory receives the two-phased `CommandGateway` |
| `outbox_subscriber(type)` | **Unsupported**: `start()` fails with `KernelError::Unsupported` | Supported: writes an outbox entry through `outbox::io::OutboxRecorder`; requires outbox AMQP config |
| Inbox recording (`AppState` inbox recorder) | Not exposed | Supported: records into `InboxStoreStorage` |
| Inbox AMQP intake | Not available | Supported when inbox AMQP config is present |
| Command and event workers | Not applicable | Kernel-owned loops follow the cancellation and failure rules under [Lifecycle](#lifecycle) |
| Outbox consumer and AMQP publication | Not available | Run by the kernel when outbox subscribers are registered |
| Stale-reservation sweepers (command, event, inbox, outbox) | Not applicable | Run by the kernel for every component in use, following the [entry retry policy](../docs/contracts.md#entry-retry-policy) |

Inbox entry *processing* (how an application turns inbox entries into commands) needs its own registration API.
That API is still open and is not decided here.

### Direct mode: command-gateway subscribers

Building direct mode involves a dependency cycle: the command dispatcher needs the event gateway, the event gateway
needs the subscribers, and command-gateway subscribers need the command gateway. The kernel resolves the cycle with
a gateway handle that is set once, after the command gateway is built and before `start()` returns. Dispatching
through the handle before it is set is impossible, because no event can be dispatched before `start()` returns.

### Persistent mode: outbox subscribers

The kernel-registered subscriber (reserved name `_outbox`) converts each event into a `NewOutboxEnvelope` and
records it. The routing key comes from the event metadata. If the routing key is missing or empty, the subscriber
fails, so the event entry follows the normal retry policy instead of being marked `completed` without outbound work.

## Configuration

`KernelConfig` (`KernelConfig::from_env()` reads the environment variables shown):

| Field | Env var | Direct | Persistent |
| --- | --- | --- | --- |
| `database_url` | `DATABASE_URL` | Ignored | Used to build the pool unless the caller passes one explicitly (an explicit pool wins). Neither present → `Config` error |
| `amqp_url` | `AMQP_URL` | Ignored | Required by inbox intake and outbox publication |
| `inbox_queue` | `INBOX_QUEUE` | Ignored | With `amqp_url` and `inbox_exchange`, enables inbox AMQP intake |
| `inbox_exchange` | `INBOX_EXCHANGE` | Ignored | See `inbox_queue` |
| `outbox_exchange` | `OUTBOX_EXCHANGE` | Ignored | With `amqp_url`, configures the outbox `AmqpPublisher` |
| `outbox_content_type` | `OUTBOX_CONTENT_TYPE` | Ignored | Content type of published messages |

Direct mode ignores config values so that a single `from_env()` configuration works for both modes.

## Startup validation and errors

`start()` and `start_persistent(...)` validate before building anything:

| Condition | Error |
| --- | --- |
| Two handlers for the same command type | `DuplicateCommandHandler` |
| Same `(event type, subscriber name)` registered twice | `DuplicateEventSubscriber` |
| An application subscriber named `_outbox`, or `outbox_subscriber` called twice for one event type | `ReservedSubscriberName` |
| `outbox_subscriber` registered in direct mode | `Unsupported` |
| Persistent mode without a pool and without `database_url` | `Config` |
| Some but not all of the inbox AMQP fields (`amqp_url`, `inbox_queue`, `inbox_exchange`) set | `Config` |
| Outbox subscribers registered without `amqp_url` and `outbox_exchange` | `Config` |

## Lifecycle

**D-07: decided; implementation pending.** The kernel owns its background loops and ties them to the handle's cancellation token. The following are contractual outcomes, not claims that the current handles already implement them.

### Cancellation and waiting

- `shutdown()` is idempotent and requests cancellation. Workers stop taking new batches or deliveries, finish already-started work, and perform its completion/failure recording and acknowledgements. Finishing an attempt does not guarantee that the attempt succeeds.
- Shutdown does not drain queued work. Unprocessed durable work remains available for restart, subject to the existing retry and stale-reservation policies.
- There is no kernel-imposed shutdown deadline. Slow or blocked in-flight work can delay termination indefinitely; existing operation-specific timeouts, including publisher-confirm timeouts, are unaffected.
- `wait()` observes termination and does not initiate cancellation. It resolves only after every kernel-owned worker has stopped, returning an error if any exited with an unrecoverable error or panic. Calling it while workers continue running may wait indefinitely.
- An unrecoverable worker exit or panic neither cancels sibling workers nor triggers automatic restart. Siblings continue until separately stopped; `wait()` retains the failure and cannot report success merely because the remaining workers later stop cleanly. Per-entry failures continue to follow their existing retry behavior.
- `wait()` does not join arbitrary application-owned request tasks. Applications must stop and await their own request intake/tasks. In direct mode, with no kernel-owned workers, waiting does not imply that application-owned direct dispatch tasks have finished.
- Inbox stream closure without explicit cancellation is a transport error, not clean shutdown. Explicit cancellation is the clean shutdown path; recording or acknowledgement failures must not be reported as successful completion.

### Dispatch during shutdown

- Once shutdown begins, new external dispatch through kernel-provided state, gateways, and their clones is rejected before side effects or durable acceptance. Holding a previously obtained handle does not bypass this restriction.
- Already-accepted executions may finish event handoff and issue follow-up commands. The exemption follows the accepted execution, not mere possession of a handle; new external requests remain rejected even while other work is in flight.
- Persistent follow-up commands may be recorded without execution before shutdown. They remain queued for restart; accepting them does not authorize workers to claim additional batches during shutdown. Direct in-flight follow-ups may complete inline.
- Distinguishing accepted in-flight follow-ups from new external dispatch is a required, pending capability. This decision specifies no implementation mechanism or new public type. It does not add lifecycle guarantees to independently constructed component gateways outside kernel ownership.

## Supported runtime contexts

Kernel-managed asynchronous operations and runtime-bridging helpers promise support under a live **multi-thread Tokio runtime**:

| Operation | Supported invocation context |
| --- | --- |
| Async workers and async publication (`publish_async`) | Asynchronous execution under the live multi-thread runtime |
| Synchronous AMQP publication (`publish`) | Blocking threads associated with that runtime, not direct calls from async tasks |
| `block_on_blocking` | Multi-thread Tokio runtime tasks |

Standalone synchronous publication and current-thread runtimes are outside this promised support boundary, even where current code contains fallback behavior. Runtime restrictions are not automatically enforced today; unsupported calls must not be assumed safe from panic or deadlock. This boundary concerns asynchronous operations and runtime bridges, not ordinary synchronous domain/storage functions that do not bridge a runtime.

## Example

```rust
use kernel::io::{EventSubscriberPort, KernelConfig, boot};

let kernel = boot(KernelConfig::from_env()?)
    .command_handler("PostTweet", Arc::new(PostTweetHandler::new(pool.clone())))
    .event_subscriber_with_command_gateway("TweetPosted", "timeline-fan-out", |gateway| {
        Arc::new(FanOutSubscriber::new(gateway)) as Arc<dyn EventSubscriberPort>
    })
    .outbox_subscriber("TweetPosted")
    .start_persistent(pool, 2)?;
```

## Implementation gaps

Current `kernel/src/lib.rs` differs from this contract:

- `start()` replaces command-gateway subscribers with `NoopEventSubscriber`, registers a no-op `_outbox` subscriber
  instead of rejecting it, and backs `AppState.inbox_recorder` with `NoopInboxStore`.
- `start_persistent()` registers a no-op `_outbox` subscriber, so events are marked `completed` without an outbox
  entry being written. It wires no inbox, outbox consumer, publisher, or sweepers, and takes the pool as its only
  database input.
- `KernelConfig` is stored as `_config` and never read. `KernelError` has no `Config` or `Unsupported` variants.
- Neither handle owns worker joins. Direct `wait()` returns immediately; persistent `wait()` cancels its token and returns immediately. Neither implements the waiting/error-reporting contract above.
- Existing dispatch handles have no shutdown gate or accepted-execution exemption for in-flight follow-ups. These guarantees require future implementation and tests.
- Synchronous AMQP publication currently calls `Handle::block_on` when a runtime is present and creates a fallback runtime otherwise. `block_on_blocking` uses `block_in_place` and the current runtime handle. These paths do not establish support for arbitrary calling contexts or enforce the documented restrictions.
- The test apps write their own outbox subscribers against their own tables instead of using `outbox_subscriber`.
- `kernel/src/lib.rs` exports about 30 items at the crate root next to `pub mod io`, and both test apps import from the root (`kernel::ApplicationCommand`, `kernel::EventSubscriberPort`, `kernel::boot`, …). Commanding exports `NewCommand` only under `#[cfg(feature = "diesel")]`.
