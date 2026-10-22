# Kernel Specification

> **Status:** decided 2026-10-22 (D-04, D-07 and D-11 in [test-coverage-report.md](../docs/test-coverage-report.md)).
> QA-01 registration and transport assembly are implemented and covered. Lifecycle gating, retry-policy enforcement and broker-confirmation gaps remain; see [Implementation gaps](#implementation-gaps).

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

The kernel facade exports the types needed by its supported modes. `NewCommandEnvelope<C>` is the typed application envelope; `GatewayNewCommandEnvelope` is the raw gateway envelope. Independent library feature-gating cleanup remains tracked under D-11.

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
a non-owning `CommandGateway::Reference` built using `Arc::new_cyclic`. The target becomes available before
`start()` returns, without a strong-reference cycle. Factories construct subscribers during startup; they must not
dispatch commands themselves. Premature dispatch returns an error rather than silently succeeding.

### Persistent mode: outbox subscribers

The kernel-registered subscriber (reserved name `_outbox`) converts each event into a `NewOutboxEnvelope` and
records it. The routing key comes from `NewEventMetadata::routing_key` (and is preserved through durable event storage). If the routing key is missing or empty, the subscriber
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
| Inbox queue/exchange configured without all three inbox AMQP fields | `Config` |
| AMQP URL without inbox intake/outbox subscribers, or outbox configuration without outbox subscribers | `Config` |
| Outbox subscribers registered without `amqp_url` and `outbox_exchange` | `Config` |

## Persistent assembly details

`start_persistent(pool, drain_rounds)` accepts an explicit pool as before; passing `None` builds one from
`database_url`. Startup requires a live multi-thread Tokio runtime and reports `Unsupported` otherwise.
It owns command/event workers (one-second polling), optional inbox intake and outbox publication, and a
reservation-sweep loop (one-minute polling, five-minute stale threshold). Applications need not spawn additional
command/event workers. `drain_rounds` still controls synchronous command/event draining after dispatch; zero leaves
processing to the workers. Keep the handle alive while workers are needed; dropping it requests cancellation.

`PersistentKernelState::inbox_recorder` records durably; direct `AppState` has no inbox recorder.
`PersistentKernelState::event_gateway()` exposes durable event acceptance. Exchanges are provisioned by the
application. Inbox startup declares the configured queue using the inbox adapter and binds it to the configured
exchange with `#` (use a topic or fanout exchange for catch-all intake). Outbox publication uses the configured
exchange and content type. An empty outbox exchange selects the AMQP default exchange. Transport setup errors
fail startup before background loops are spawned. Partial/unused AMQP configuration is a `Config` error.

Adding `routing_key: Option<String>` to both event metadata types requires updating Rust struct literals;
previously stored JSON metadata without the field remains readable. Handler-generated events obtain the routing key from
`ApplicationEvent::routing_key()` (default `None`); events intended for outbox recording must override that method
or supply routing metadata on a directly recorded event. Missing routing causes event failure/retry, never silent completion.

## Lifecycle

**D-07: partially implemented.** Persistent handles own worker joins, `shutdown()` requests cancellation, and
`wait()` observes all joins without cancelling and retains worker errors/panics. Dispatch gating and in-flight
follow-up exemptions remain pending. The following are the complete contractual outcomes, not a claim that all
lifecycle scenarios have been verified.

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

Registration assembly now lives in private `assembly`, `registrations`, and `runtime` modules; the crate exposes
only `pub mod io`. Both test applications import through that facade. The former `NoopInboxStore` and
`NoopEventSubscriber` paths have been removed.

Remaining gaps outside QA-01:

- Dispatch handles have no shutdown gate or accepted-execution exemption for in-flight follow-ups (D-07).
- Sweepers are wired, but component retry/exhaustion policy fixes and recovery coverage remain D-01/D-02 work.
- The AMQP publisher still needs D-06 confirmation/return/timeout and persistent-delivery enforcement. A passing
  routed publication test does not establish those failure/durability guarantees.
- Inbox invalid-transport-ID rejection remains D-03 work; inbox-entry-to-command registration is still undecided.
- Synchronous AMQP publication and `block_on_blocking` do not enforce runtime restrictions for independent callers.
- The test apps retain their app-owned outbox tables/subscribers rather than using `outbox_subscriber`.
- Independent commanding facade feature gating and metadata propagation/error-classification fixes remain D-11 work.

## QA-01 verification

`cargo test --manifest-path kernel/Cargo.toml --all-features --offline` runs registration/error tests,
outbox-adapter tests, and a compile-fail check that direct state exposes no inbox.
Three opt-in integration tests use isolated PostgreSQL schemas and uniquely named RabbitMQ resources:

```sh
KERNEL_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
KERNEL_TEST_AMQP_URL=amqp://guest:guest@127.0.0.1:5672/%2f \
cargo test --manifest-path kernel/Cargo.toml --offline --test persistent -- --ignored --test-threads=1
```

These verify persistent subscribers/follow-up commands, database URL versus explicit-pool selection, background
processing with no synchronous draining, durable inbox acceptance (and storage-failure rejection), real outbox
recording, missing-routing failure/retry, configured exchange/content type publication, and AMQP inbox intake.
They do not establish the remaining lifecycle, retry, acknowledgement-ordering or broker-failure contracts.
