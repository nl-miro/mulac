# Kernel Specification

> **Status:** decided 2026-10-22 (D-04 in [test-coverage-report.md](../docs/test-coverage-report.md)).
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

## Capability matrix

| Capability | Direct (`start`) | Persistent (`start_persistent`) |
| --- | --- | --- |
| `command_handler(type, handler)` | Supported: dispatched synchronously | Supported: recorded, then executed by the command consumer |
| `event_subscriber(type, name, subscriber)` | Supported: invoked synchronously | Supported: invoked by the event consumer |
| `event_subscriber_with_command_gateway(type, name, factory)` | Supported: the factory receives the direct `CommandGateway` | Supported: the factory receives the two-phased `CommandGateway` |
| `outbox_subscriber(type)` | **Unsupported**: `start()` fails with `KernelError::Unsupported` | Supported: writes an outbox entry through `outbox::io::OutboxRecorder`; requires outbox AMQP config |
| Inbox recording (`AppState` inbox recorder) | Not exposed | Supported: records into `InboxStoreStorage` |
| Inbox AMQP intake | Not available | Supported when inbox AMQP config is present |
| Command and event workers | Not applicable | Run by the kernel until the handle's token is cancelled |
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

The kernel starts all of its background loops and ties them to the handle's cancellation token.
`shutdown()` cancels the token; `wait()` resolves once every loop has stopped.

## Example

```rust
let kernel = kernel::boot(kernel::KernelConfig::from_env()?)
    .command_handler("PostTweet", Arc::new(PostTweetHandler::new(pool.clone())))
    .event_subscriber_with_command_gateway("TweetPosted", "timeline-fan-out", |gateway| {
        Arc::new(FanOutSubscriber::new(gateway)) as Arc<dyn kernel::EventSubscriberPort>
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
- `wait()` cancels the token instead of waiting for the background loops.
- The test apps write their own outbox subscribers against their own tables instead of using `outbox_subscriber`.
