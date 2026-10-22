# QA/QC test coverage report

## Scope and evidence

This report consolidates separate core/kernel, delivery/storage, and application QA/QC reviews of the repository on **2026-10-22**. It identifies **what needs coverage and the observable results required**, not test implementation methods.

The review covers `kernel`, all five crates under `libs`, both test applications, their migrations, and repository validation workflows. `kernel_derive` currently contains build artifacts but no source or Cargo manifest; there is no active derive implementation to assess.

### Executed verification

| Scope | Result |
| --- | --- |
| Library workspace, all features, offline | **82 passed; 8 ignored; 0 failed** |
| Kernel, all features, offline | **0 tests; 0 doctests** |
| Commanding without default features | Crate check passed; this does not establish external API usability |
| Database-backed outbox tests | **Not executed**; all eight are ignored by default |
| Application integration tests | Inspected, **not executed** |
| Live broker behavior, production binaries, restart/load behavior | Inspected where source exists, **not exercised** |

Commands used for the principal executed results were `cargo test --manifest-path libs/Cargo.toml --workspace --all-features --offline` and `cargo test --manifest-path kernel/Cargo.toml --all-features --offline`.

### Existing coverage and its limits

- **Commanding and eventing:** 14 tests each, primarily status conversions, error-list helpers, and reservation/sweep specification construction. These do not establish gateway, dispatcher, consumer, or database lifecycle correctness.
- **Inbox:** 21 tests, primarily domain/specification helpers, envelope conversions, and one AMQP metadata example. Durable acceptance, acknowledgement ordering, consumer orchestration, and worker lifecycle lack direct behavioral coverage.
- **Outbox:** 29 unit tests plus four nonignored integration-file tests. Coverage includes selected recording/consumer outcomes, metadata, public exports, and stable message identity after completion failure. Eight additional database tests exist but were skipped.
- **Kernel:** no direct tests. Application tests exercise selected persistent paths indirectly.
- **Applications:** principal happy paths and selected validation, missing-resource, duplicate, inbox, outbox, and fan-out cases exist. Alternate ingestion, interruptions, races, production startup, and actual replay remain substantially undercovered.
- **Shared database pool:** no direct tests found.

“Partial” below means relevant assertions exist, not that every behavior has been exercised. Source-level risks are not presented as reproduced runtime failures. Passing these requirements would increase confidence; no finite test suite guarantees defect-free software.

## Priorities and handoff rules

- **P0 — release-critical reliability:** silent loss, incorrect acceptance, ownership corruption, unrecoverable work, or invalid business writes.
- **P1 — required behavioral completeness:** supported contracts, error paths, data integrity, operational lifecycle, and trustworthy regression evidence.
- **P2 — compatibility and operational assurance:** supported distribution/build configurations, resource limits, and declared exposure constraints.

Each `QA-xx` item is an independently assignable coverage requirement. Its scope identifies the owning source; its scenarios and acceptance criteria define the required outcomes. Shared lifecycle requirements apply **separately to inbox, commanding, eventing, and outbox**; success in one component does not establish correctness in the others.

Where an item depends on a contract decision below, its expected result remains blocked until that decision is recorded. Tests must not silently turn an implementation accident into a product requirement. No mocking strategy, framework, fixture design, algorithm, or implementation recipe is prescribed here.

## Contract decisions required before unambiguous assertions

| ID | Decision required | Conflicting or missing evidence |
| --- | --- | --- |
| D-01 | **Resolved 2026-10-22** — see [contracts.md — Entry retry policy](contracts.md#entry-retry-policy). An attempt is one reservation; default `max_attempts` 6; delay `attempts × 30s`, linear, no cap; `failed` and the sweep both go to `failed`, or `dead` at the limit. | Code follow-ups: remove the 120s cap (`MAX_RETRY_BACKOFF_SECONDS` in commanding/eventing `infra_diesel.rs` and `command_consumer.rs`; `RETRY_BACKOFF_CAP_SECONDS`, `retry_delay_seconds` and the sweep `LEAST(...)` in outbox `infra_diesel.rs`). Inbox already has no cap. |
| D-02 | **Resolved 2026-10-22** with D-01 — an exhausted stale reservation transitions to `dead`. | Code follow-up: commanding (`stale_command_sweep.rs`), eventing (`assembly/infra_diesel.rs` sweep) and inbox (`stale_reservation_sweep.rs`, whose doc comment already promises `Dead`) always write `failed`, so exhausted entries get stuck because reservation excludes `attempts >= max`. Port the outbox sweep `CASE`. |
| D-03 | **Decided 2026-10-22; rejection enforcement pending** — see [the Inbox contract](contracts.md#inbox). Deduplicate by UUID alone across sources within the same inbox table while its row exists; no time-based window. Require a valid supplied UUID, without an additional version restriction. | Storage already uses UUID identity. Missing, empty, or non-UUID transport IDs must instead be rejected before storage, without a replacement ID or success acknowledgement. AMQP rejection must not requeue; continue after successful rejection and surface rejection failures. Dead-lettering depends on broker configuration. Current parsing/conversion still allows generated fallback IDs; enforcement and its tests remain pending. |
| D-04 | **Decided 2026-10-22; implementation pending** — see [kernel/SPEC.md](../kernel/SPEC.md#capability-matrix). Direct mode supports command handlers, event subscribers and command-gateway subscribers. It rejects outbox subscribers and exposes no inbox. Persistent mode supports everything, with real outbox recording, inbox intake, outbox publishing and sweepers. `KernelConfig` is applied in persistent mode and ignored in direct mode. No registration may silently become a no-op. | Code follow-ups: direct `start()` needs a lazily set command-gateway handle for factories, must reject `outbox_subscriber`, and must drop `NoopInboxStore`. `start_persistent()` needs an `OutboxRecorder`-backed `_outbox` subscriber, a pool built from `database_url` when none is passed, and inbox AMQP intake, the outbox consumer and publisher, and the sweepers. Add `KernelError::{Config, Unsupported}` and make `wait()` wait for the background loops. Still open: the inbox-processing registration API. |
| D-05 | **Decided 2026-10-22: application-handler responsibility** — see [the Command Dispatcher contract](contracts.md#command-dispatcher). Application handlers own business-effect idempotency and recovery of required events across retries. The library owns durable command acceptance, reservation/retry processing, and dispatch of each execution's returned events, not atomic business/event persistence or exactly-once business effects. | Fresh event IDs on command re-execution remain unchanged; already-recorded event redelivery preserves stored identity. Empty event results permit command completion without reconstruction of prior outputs. The follow-handler no-op after failed handoff remains an application reliability/coverage gap; application compliance has not been verified by this decision. |
| D-06 | **Decided 2026-10-22; implementation pending** — see [contracts.md — Outbox](contracts.md#outbox) and [outbox-spec.md — AMQP Publisher](outbox-spec.md#amqp-publisher). Only `Ack` with no returned message is acceptance (`completed`). A mandatory return, `nack`, `NotRequested`, a closed channel or a confirm timeout are retriable transport failures (`failed`). `mandatory` defaults to true, and publishes are persistent by default. Acceptance does not imply durable broker storage, which also needs durable queues the outbox does not verify. | Code follow-ups in `libs/outbox/src/amqp_publisher.rs`: map `Ack(Some(_))` and `NotRequested` to `OutboxError::Transport`, and bound the confirm wait with a timeout. Default `mandatory` to true. Add `persistent` (→ `with_delivery_mode(2)` in `properties_for`) and `confirm_timeout` to `AmqpPublishConfig`. Enable confirms once per channel instead of on every publish. Add tests for each outcome row and for the defaults. |
| D-07 | **Decided 2026-10-22; implementation pending** — see [Kernel lifecycle](../kernel/SPEC.md#lifecycle) and [runtime contexts](../kernel/SPEC.md#supported-runtime-contexts). Unexpected inbox closure is a transport error. Shutdown finishes in-flight work without draining queues or imposing a deadline; `wait()` observes all owned workers without cancelling. Worker failures leave siblings running and are reported after all stop. Reject new external dispatch, but allow follow-ups from accepted in-flight executions. | Handles do not own worker joins or implement the dispatch gate/in-flight exemption. Support is promised under a live multi-thread Tokio runtime: async publication asynchronously, synchronous publication on associated blocking threads, and `block_on_blocking` in multi-thread runtime tasks. Standalone synchronous publication/current-thread runtimes are outside that boundary; restrictions are not automatically enforced. No new runtime coverage or application-task joining is claimed. |
| D-08 | **Decided 2026-10-22; implementation pending** — see [test_app_todo.md — Statuses and transitions](test_app_todo.md#statuses-and-transitions) and the sections after it. A repeated or invalid transition is 409 with no effects. New archive/unarchive commands; archived todos are read-only except for delete. PUTs replace the whole value, so omitted and null both clear. Past due dates are accepted. Lists order by `created_at`, then `id`. Inbox `DeleteTodo` returns 200 with `todo: null`. | Code follow-ups: in `task_complete.rs` and `task_reopen.rs`, make the update conditional on the expected status and distinguish 409 from 404 (`interpret_dispatch_error` recognises only not-found and validation). Add `task_archive.rs` and `task_unarchive.rs` plus their outbox subscribers. Reject archived todos in `task_update.rs` and `task_schedule_due_dates.rs`. Add the `id` tie-break in `task_list.rs`. In `lib.rs` `process_command`, make `InboundResponse.todo` optional and skip `fetch_todo` after `DeleteTodo`. |
| D-09 | **Decided 2026-10-22; implementation pending** — see [test_app_todo.md — Inbox recovery and outbox replay](test_app_todo.md#inbox-recovery-and-outbox-replay) (same policy in [test_app_twitter.md](test_app_twitter.md#inbox-and-outbox)). `failed` means rejected (400/404/409) and is terminal. Infrastructure errors leave the row `received` and return 5xx. Redelivery is always 409. A background job re-dispatches stale `received` rows (default threshold 5 min, interval 1 min) using `SKIP LOCKED`, with no attempt cap. App outbox rows are keyed by `event_id` with `ON CONFLICT DO NOTHING`. | Code follow-ups: in Todo, `record_event_payload` must take `event_id` and ignore conflicts, and `OutboxSubscriber` must require `event_id`. In both apps' `process_command`, call `mark_failed` only for Validation/NotFound/Conflict; otherwise keep `received` and record `error`. Add a recovery job to each app, started alongside the command and event workers. |
| D-10 | **Decided 2026-10-22; unlike enforcement and full verification pending** — see [Twitter temporal semantics](test_app_twitter.md#temporal-semantics). Each fan-out attempt reads current followers; retries may add followers while existing timeline rows remain. Fan-out continues after deletion. Soft-delete only the selected tweet, retaining likes/timelines/retweets. Retweet chains preserve immediate-target identity; deleted ancestors do not invalidate active descendants. | Like, retweet, and unlike targeting missing/deleted tweets return 404 without mutation/event/outbox effects. Current unlike checks existence only and still permits removal from a deleted tweet; this change remains pending. Active-tweet unlike success/no-op and duplicate-retweet-ID behavior stay unchanged. Read-API visibility remains deferred; no new runtime verification or stronger concurrency guarantee is claimed. |
| D-11 | **Decided 2026-10-22; implementation pending** — see [kernel/SPEC.md — Public API](../kernel/SPEC.md#public-api), [eventing-spec.md — Gateway Error Classification](eventing-spec.md#gateway-error-classification) and [contracts.md — Metadata propagation](contracts.md#metadata-propagation). The kernel exposes only `kernel::io`, and library facades don't gate mode-required types behind storage features. Both event-gateway paths report `Conversion` for missing metadata and keep the original error variant otherwise. Events inherit the command's `source` and correlation, with causation set to the command ID. Subscriber follow-up commands chain causation and correlation from the triggering event. | Code follow-ups: move the kernel root items into `io` and update both apps' imports. Un-gate `NewCommand` in commanding `lib.rs`. In `libs/eventing/src/gateway.rs`, share one implementation and drop the `EventDispatch` re-wrap; `EventError::EventDispatch` then has no producer. Set `source` from command metadata in `wrap_handler` (`libs/commanding/src/dispatcher.rs`). Chain metadata in `test_apps/twitter/src/timeline_fan_out.rs`. |
| D-12 | Deployment and operational acceptance limits | Authentication/trust boundaries, sensitive outbox visibility, payload/history limits, supported runtimes/platforms, latency, throughput, recovery deadlines, and resource ceilings lack complete criteria. |

Relevant contract sources include [components.md](components.md), [contracts.md](contracts.md), [inbox-spec.md](inbox-spec.md), [command-handling-spec.md](command-handling-spec.md), [eventing-spec.md](eventing-spec.md), and [outbox-spec.md](outbox-spec.md). Older documentation is not uniformly aligned with current code; Todo guidance still contains SQLx-era descriptions.

## P0 — release-critical coverage

### QA-01 — Kernel registration must perform the promised work

**Scope:** [`kernel/src/lib.rs`](../kernel/src/lib.rs): `KernelBuilder::start`, `start_persistent`, `event_subscriber_with_command_gateway`, `outbox_subscriber`, `NoopInboxStore`, `NoopEventSubscriber`. **Coverage:** absent. **Decision:** D-04.

- Direct subscribers receive matching events; command-gateway subscriber registrations execute their registered behavior and can produce follow-up commands in every supported mode.
- Supported inbox acceptance actually transfers responsibility to durable storage or the explicitly agreed processing boundary.
- Supported outbox acceptance actually creates the promised outbound work.
- Unsupported capabilities have an explicit outcome; successful startup/dispatch must not falsely imply persistence or delivery.
- Configuration claimed to control a capability has an observable effect.

**Source risk:** current no-op branches can report success without the work their names imply.

### QA-02 — Exclusive reservation and ownership protection

**Scope:** `libs/{inbox,commanding}/*_consumer.rs` under `src`; [`eventing storage`](../libs/eventing/src/assembly/infra_diesel.rs); [`outbox storage`](../libs/outbox/src/assembly/infra_diesel.rs). **Coverage:** partial outbox only.

- Concurrent consumers never receive overlapping active claims; locked work does not prevent claiming other eligible work.
- Reservation status, token, timestamps, and attempt increment change atomically.
- Correct owners can complete/fail work; wrong tokens, nonexistent IDs, repeated transitions, and stale owners are rejected without mutation.
- After sweep and re-reservation, the old owner cannot complete, fail, or dead-letter the new claim.
- Completion/failure/dead-letter versus sweep races leave one coherent state and preserve the winning owner's data.
- Transition failure leaves no partial status, ownership, schedule, or error-history update.

### QA-03 — Retry exhaustion and stale recovery

**Scope:** all consumer failure paths and `stale_*_sweep.rs`; eventing/outbox storage sweep implementations. **Coverage:** partial outbox; otherwise mostly specification-construction tests. **Decisions:** D-01, D-02.

- Below-limit, exactly-at-limit, and above-limit attempts have the agreed retry/terminal outcome.
- Attempt accounting matches the agreed policy on reservation, explicit failure, and timeout.
- Retry deadlines follow the agreed formula and cap; work is ineligible before the deadline and eligible at its contractual boundary.
- Fresh reservations, null reservation timestamps, nonreserved states, and exact timeout boundaries have defined outcomes.
- Repeated sweep does not add duplicate timeout history or change already released entries.
- Exhausted stale work never remains permanently ineligible in a nonterminal state.

**Source risk:** inbox, command, and event sweeps ignore the maximum-attempt decision and always produce `failed`; their retry-delay handling also differs from capped paths.

### QA-04 — Durable inbox acceptance and broker acknowledgement

**Scope:** [`amqp_consumption.rs`](../libs/inbox/src/amqp_consumption.rs): `WorkerLoop::run`; [`record_messages.rs`](../libs/inbox/src/record_messages.rs). **Coverage:** absent for orchestration. **Decisions:** D-03, D-07.

- Acknowledgement occurs only after durable recording succeeds, exactly once per handled delivery; no negative acknowledgement accompanies success.
- Recording failure never acknowledges acceptance; its negative acknowledgement requests the contractual redelivery behavior.
- Storage success followed by acknowledgement failure or process interruption retains the accepted row; stable-ID redelivery does not create another row.
- Duplicate recording does not overwrite the original payload or lifecycle.
- Missing, empty, or non-UUID message IDs produce no inbox insertion or success acknowledgement and no replacement UUID. AMQP rejection does not requeue; later valid deliveries remain processable after successful rejection. Rejection failures surface as errors; dead-lettering is conditional on broker configuration.
- Acknowledgement errors, negative-acknowledgement errors, and recording-task failures remain distinguishable and cannot falsely signal acceptance.
- Cancellation while idle, receiving, or backing off stops new intake; an already-started delivery finishes recording and acknowledgement/negative acknowledgement without taking another delivery. Failed recording or acknowledgement remains observable. No kernel shutdown deadline or queue drain is promised.

**Implementation status:** D-03 is decided, but invalid-ID rejection is not implemented or verified. The current fallback remains; these criteria are required future outcomes, not additional passing coverage.

### QA-05 — Outbox publication requires the agreed broker evidence

**Scope:** [`amqp_publisher.rs`](../libs/outbox/src/amqp_publisher.rs); [`outbox_consumer.rs`](../libs/outbox/src/outbox_consumer.rs). **Coverage:** metadata/property tests only for the adapter. **Decisions:** D-06, D-07.

- No successful publication/completion precedes the required broker acknowledgement.
- Negative confirmation, failed confirmation setup, channel/connection closure, and interrupted publication yield transport failure rather than completion.
- Missing confirmation and mandatory returned/unroutable messages have the explicit agreed outcome.
- Default and configured exchanges/routing keys preserve destination semantics and payload identity.
- Under a live multi-thread Tokio runtime, async publication runs asynchronously and synchronous publication runs on associated blocking threads without panic or deadlock. Direct synchronous calls from async tasks, standalone synchronous publication, and current-thread runtimes are not promised supported contexts.
- Shutdown does not claim another outbox batch; an already-started batch finishes publication attempts and state updates under the existing acceptance/failure rules. The confirm timeout remains an operation-specific limit, not a kernel shutdown deadline.
- Broker restart behavior matches the explicitly claimed durability boundary; broker acceptance is not misreported as downstream consumer processing.

### QA-06 — Recovery across every responsibility-transfer boundary

**Scope:** inbox → command → event → subscriber/outbox → broker; application handlers and subscribers. **Coverage:** one outbox completion-failure case; otherwise incomplete. **Decisions:** D-05, D-09.

Cover interruption after durable recording, reservation, business mutation, event acceptance, subscriber side effects, broker acceptance, and immediately before completion.

**Library-owned acceptance:** durable queue work remains recoverable under the retry policy or reaches its legitimate terminal state. Command dispatch succeeds only after acceptance of all events returned by the current execution; an empty event result is valid and permits command completion. A failed handoff propagates to the caller in Direct mode or triggers queue failure/retry handling in TwoPhased mode, without rolling back earlier accepted events or business side effects. The library does not reconstruct prior outputs or guarantee atomic business/event persistence. Repeated publication of the same outbox entry preserves payload and message ID, including caller-supplied IDs. Completed/dead entries do not spontaneously re-enter processing.

**Application-owned acceptance:** handlers account for repeated business effects and recover events required by committed changes, including when a retried mutation becomes a conflict/no-op. A committed change followed by failed event handoff must not be treated as application recovery merely because the next command execution completes with no events. These are application obligations, not kernel-managed exactly-once guarantees.

**Remaining gap:** the Twitter follow handler can commit the relationship, fail to hand off `UserFollowed`, then return no events on retry. D-05 assigns this risk to application-handler coverage; it does not resolve the defect or demonstrate compliance.

### QA-07 — Recording deduplication preserves accepted data

**Scope:** all `record_*` modules and storage insert paths. **Coverage:** selected outbox row-count coverage; otherwise incomplete. **Decision:** D-03.

- Sequential and concurrent recording of the same identity have the agreed single-entry outcome.
- Same identity with different payload, type, routing, metadata, or timestamps does not silently replace accepted data.
- Re-recording reserved, failed, completed, or dead work does not reset its lifecycle or attempts.
- Distinct IDs with identical payload remain distinct work.
- Within one inbox table, the same UUID from the same or a different source leaves one original row unchanged, including with different payloads and across lifecycle states.
- The same UUID after more than 24 hours remains deduplicated while its row exists. There is no configurable retention window; deleting the row removes protection and permits recording that UUID again. No cleanup policy is implied.
- Missing, empty, or non-UUID inbound IDs are rejected before recording, without generating a replacement UUID. This decided rejection policy remains pending implementation; current fallback behavior is not evidence of compliance.
- Failed storage never returns durable acceptance.

### QA-08 — Twitter validation is consistent across entry points

**Scope:** [`tweet_post.rs`](../test_apps/twitter/src/tweet_post.rs), [`direct_message_send.rs`](../test_apps/twitter/src/direct_message_send.rs), [`inbox.rs`](../test_apps/twitter/src/inbox.rs), `validate_content`. **Coverage:** direct HTTP validation only.

For `PostTweet` and `SendDirectMessage`, cover direct HTTP and inbox ingestion with empty content, whitespace-only content including Unicode whitespace, 280/281 Unicode scalars, multibyte characters, and surrounding whitespace.

**Acceptance:** invalid content receives the contractual client error with no domain row, domain event, or outbox entry. Valid boundary content is preserved consistently. Inbox failure status reflects the rejected command.

**Source risk:** validation resides in direct HTTP handlers; inbox conversion and command handlers bypass it, and table definitions lack equivalent content constraints.

### QA-09 — Todo inbox deletion reports the actual committed outcome

**Scope:** [`test_apps/todo/src/lib.rs`](../test_apps/todo/src/lib.rs): `inbox::http::process_command`; [`task_delete.rs`](../test_apps/todo/src/task_delete.rs). **Coverage:** absent. **Decision:** D-08.

- Deleting an existing todo through inbox returns the agreed success response, removes the row, records the deletion effect, and marks the inbox record processed.
- Missing-row deletion has the agreed not-found response and creates no deletion event.
- A repeated accepted message ID has the existing duplicate-rejection outcome and no additional effects.

**Source risk:** inbox processing marks the message processed and then always fetches the todo. Successful deletion therefore reaches a not-found response after committing the operation. The current response requires a todo object, so its success shape needs a decision.

### QA-10 — Application inbox interruption and duplicate races

**Scope:** Todo inline inbox module; [`Twitter inbox`](../test_apps/twitter/src/inbox.rs). **Coverage:** basic sequential duplicates only. **Decision:** D-09.

- Concurrent requests with the same message ID, with identical and conflicting payloads, do not produce multiple accepted executions.
- Duplicate identities in received, processed, and failed states retain the original recorded data and contractual response.
- Failures after receipt recording, dispatch, status update, and response materialization leave states that accurately represent committed work.
- Recorded-but-undispatched and failed messages have the agreed recovery outcome.
- Client-visible errors do not imply rollback when business effects already committed; any such partial-success boundary is explicit.

## P1 — component and orchestration completeness

### QA-11 — Reservation eligibility and parameter boundaries

**Scope:** all `Reservable*Spec` types and storage reservation queries. **Coverage:** mostly construction tests. **Decision:** D-01.

Cover empty queues; zero/one/oversized batches; mixed due/future work; equal schedules; all persisted statuses; limits around exhaustion; zero/negative maximum attempts; extreme counts and durations.

**Acceptance:** only eligible states, schedules, and attempts are claimed; count never exceeds the requested valid limit; unselected rows remain unchanged. Zero limit claims nothing. Invalid parameters have explicit outcomes without overflow, panic, or accidental unlimited selection. Selection behavior must not be confused with an outbox delivery-order guarantee, which is explicitly absent.

### QA-12 — Complete consumer outcome matrices

**Scope:** all four consumer modules. **Coverage:** partial outbox; other orchestration largely uncovered.

- Empty batch succeeds with the documented count; reservation failure causes no dispatch/publication or state transition.
- Each successful entry uses its exact ID/token for completion.
- Missing ownership prevents downstream side effects.
- Conversion and dispatch/publication failures select the correct failed/dead transition.
- Completion failure, failure-recording failure, and dead-letter-recording failure remain observable.
- Original processing errors are retained alongside errors recording their outcome.
- Mixed batches continue according to contract and retain every relevant error; returned counts have a precise meaning and do not imply all entries failed when only some did.

### QA-13 — Direct and two-phased dispatch contracts

**Scope:** commanding/eventing `gateway.rs`, `dispatcher.rs`, recorder modules, and kernel subscriber registry. **Coverage:** absent directly. **Decisions:** D-05, D-11.

- Direct dispatch invokes work inline without creating a durable queue entry; two-phased dispatch durably records acceptance without invoking processing inline at that boundary.
- Missing required metadata, invalid command JSON, type mismatch, handler failure, storage failure, and subscriber failure produce the agreed errors and no prohibited side effects.
- Zero, one, and multiple produced events follow the declared handoff semantics. A successful empty-event result permits command completion even after an earlier attempt produced events; the kernel has no obligation to reconstruct those outputs.
- Failed event handoff is reported and persistent command processing follows its retry policy. Re-execution may invoke the handler again; library acceptance does not require atomicity with the handler's business changes.
- Partial event/subscriber delivery is not falsely described as rollback; earlier accepted events remain accepted, and persistent retry may revisit successful recipients. Application-owned recovery is evaluated separately under QA-06.
- Zero matching subscribers is successful; unrelated subscribers never receive the event.
- Inherent and trait-based event gateway calls have the approved error classification.

### QA-14 — Typed envelopes, identities, and trace metadata

**Scope:** `kernel::NewCommandEnvelope::into_gateway_envelope`, `wrap_handler`, all envelope conversions, app event serializers. **Coverage:** selected inbox/outbox conversions only. **Decisions:** D-05, D-11.

Every command/event variant preserves its discriminator, payload, supplied identity, and optional fields at the applicable envelope boundary. Invalid serialization/deserialization fails at the stated boundary without invoking the handler. Explicit correlation survives, and fallback correlation/causation follows the command contract. Source propagation and subscriber-generated command metadata have explicit outcomes. Follow/unfollow and like/unlike event representations remain distinguishable.

**Identity acceptance:** `wrap_handler` assigns fresh UUID v7 event IDs to outputs of each command execution, including re-execution of the same command; tests must not require stable generated IDs across those executions or assume event-ID deduplication merges their business effects. Redelivery of an already-recorded event preserves its stored event ID, and publication retries of the same outbox entry preserve its message ID. Application-handler recovery obligations remain separate from these identity guarantees.

### QA-15 — Registry validation and exact routing

**Scope:** [`command_handler_registry.rs`](../kernel/src/command_handler_registry.rs), [`event_subscriber_registry.rs`](../kernel/src/event_subscriber_registry.rs), `KernelBuilder::validate`. **Coverage:** absent.

Cover duplicate command handlers; duplicate `(event_type, subscriber_name)` pairs across direct/factory registrations; the same subscriber name on different event types; reserved `_outbox` names; repeated outbox registration; unknown and empty command/event types; registry construction outside the builder.

**Acceptance:** supported registrations resolve only to their exact recipients; unknown commands invoke no unrelated handler; duplicates/reserved names produce the agreed error. Builder validation and independently constructed registries must have explicitly defined differences, not accidental last-write-wins behavior.

### QA-16 — Persistent draining, workers, cancellation, and shutdown

**Scope:** `PersistentKernelState::dispatch_command`, both kernel handles, [`workers.rs`](../kernel/src/workers.rs), inbox `WorkerLoop::run`. **Coverage:** absent directly. **Decision:** D-07.

- Zero drain rounds mean durable acceptance, not completed execution; finite rounds leave excess entries and long command/event chains recoverable.
- Command-then-event sequencing and subscriber-produced work follow the configured round boundary, including queues larger than the 64-entry drain batch.
- Background workers process pending work without an HTTP-triggered drain. Per-entry failures retain existing retry behavior; an unrecoverable worker exit or panic does not cancel siblings or trigger automatic restart.
- Cancellation before polling or during idle receive/backoff waits stops new intake. Workers finish their already-started batch or delivery and its completion/failure recording and acknowledgements, but do not claim another batch or drain queued work.
- Repeated shutdown is idempotent. `wait()` does not initiate cancellation and returns only after all kernel-owned workers stop, reporting any unrecoverable error/panic even if siblings later stop cleanly. Waiting while siblings still run is not a successful shutdown.
- No kernel shutdown deadline is promised: blocked in-flight work can keep `wait()` pending indefinitely. Existing operation-specific timeouts remain unchanged. Parent/child cancellation propagation follows the token contract.
- New external dispatch after shutdown through state, gateways, or clones is rejected before side effects/durable acceptance. A previously obtained handle does not exempt a new request.
- Accepted in-flight executions may finish event handoff and issue follow-up commands. Persistent follow-ups may remain queued for restart without additional batch claims; direct follow-ups may complete inline. The exemption follows the accepted execution, not handle possession.
- `wait()` does not join application-owned request tasks. Direct mode with no owned workers does not imply all application-owned direct calls have finished; stopping and awaiting those tasks remains an application responsibility.
- Supported runtime paths require a live multi-thread Tokio runtime: async workers/publication asynchronously, synchronous publication on associated blocking threads, and `block_on_blocking` in multi-thread runtime tasks. No panic/deadlock is permitted within these supported paths; broader contexts are not promised.

**Implementation status:** worker joins, dispatch gating, and the in-flight follow-up exemption remain pending. These are future acceptance criteria, not newly executed or passing tests.

### QA-17 — AMQP intake validity and stream lifecycle

**Scope:** inbox `AmqpClient`, `AmqpTransport`, `WorkerLoop`. **Coverage:** one metadata example. **Decisions:** D-03, D-07.

Cover valid UUID message IDs without an additional version restriction; absent, empty, and malformed message-ID properties; unchanged correlation-ID handling; empty exchange/routing key; empty/non-ASCII payloads; invalid UTF-8; stream closure; transport errors; and cancellation during receive/backoff.

**Acceptance:** valid bytes and metadata retain their declared meaning. Invalid UTF-8 is never stored and receives the specified nonretry/poison-message disposition; later valid deliveries remain processable. No polling occurs during contractual backoff. Stream closure without explicit cancellation is a transport error; explicit cancellation is the clean shutdown path. Cancellation during idle receive/backoff stops without another delivery, while an already-started delivery finishes its recording/acknowledgement boundary and preserves any errors. An unrecoverable worker exit leaves siblings running; kernel `wait()` reports failure after all owned workers stop, without an automatic restart guarantee. Reconnection/redelivery behavior satisfies only the explicitly supported guarantee.

**Decided message-ID outcomes (enforcement pending):** missing, empty, or non-UUID message IDs cause no insertion, generated replacement ID, or success acknowledgement. Their AMQP deliveries are rejected without requeue; processing continues after successful rejection, including a following valid delivery. Rejection failure is surfaced rather than ignored. Dead-lettering depends on broker configuration and is not guaranteed by rejection alone. The existing metadata example does not establish these outcomes, and no new runtime verification is claimed.

### QA-18 — Persistence conversion and error-history integrity

**Scope:** all domain `ExtraInfo` types, JSONB adapters, row/envelope conversions, failure/sweep updates. **Coverage:** partial helpers and outbox conversions.

Cover nullable metadata/history, missing `errors`, empty/multiple errors, Unicode, malformed JSON shapes/types, unsupported encoded JSONB representations, invalid status codes, and malformed rows within otherwise valid batches.

**Acceptance:** valid payloads and IDs round-trip unchanged; failures and timeouts append exactly one ordered reason without deleting earlier history. No-reason transitions preserve history. `last_error` and completion behavior follow their declared contracts. Bad stored data produces an explicit error, not invented metadata; conversion failure after reservation does not silently strand valid work forever. Unknown metadata keys must not be granted a preservation guarantee absent a contract.

### QA-19 — Database failures, pool behavior, and migrations

**Scope:** [`mulac_diesel::build_pool`](../libs/mulac_diesel/src/lib.rs), all storage adapters, both apps' migrations/`schema.rs`, [`outbox schema`](outbox_entries.sql). **Coverage:** app migration use is indirect; failure/upgrade coverage absent.

Cover invalid/unavailable database configuration, exhausted pools, lost connections, statement/transaction failure, empty databases, already-migrated databases, and upgrades containing business data and queued work.

**Acceptance:** errors do not masquerade as acceptance/completion; failed transactions leave connections and state usable. No connections/transactions leak across users. Supported migrations retain existing data and produce schemas compatible with actual storage code, including metadata nullability, lifecycle fields, uniqueness, and `extra_info`. Repeated migration is harmless; supported downgrade behavior, if claimed, has an explicit data-loss boundary.

## P1 — application behavior

### QA-20 — Full inbox command/result matrices

**Scope:** Todo inbox `to_app_command`; Twitter `TwitterCommand`, `to_app_command`, `materialize_entity`. **Coverage:** partial.

Remaining Todo coverage includes complete, reopen, update, delete, and applicable validation/not-found outcomes. Remaining Twitter coverage includes retweet, follow, unfollow, like, successful direct message, and duplicate/no-op variants.

**Acceptance:** target IDs and supplied fields map correctly; domain mutation, event type/payload, response entity, and processed/failed inbox state agree. A contractual no-op produces no event/outbox effect. Invalid mapping never acts on a different resource.

### QA-21 — Todo lifecycle and field preservation

**Scope:** Todo `task_create`, `task_update`, `task_complete`, `task_reopen`, `task_delete`; corresponding tests. **Coverage:** happy paths/basic errors. **Decision:** D-08.

Cover optional fields on creation, title trimming, description clearing, missing-resource updates, complete→reopen, repeated complete/reopen, archived-state operations, delete after completion, and repeated delete.

**Acceptance:** only intended fields change; ID and creation time remain stable; update time and event snapshot match the accepted operation. Failed changes preserve prior state and produce no success event. Deleted rows disappear from get/list. Repeated and archived transitions follow the approved policy rather than assumed idempotency.

### QA-22 — Todo dates and read contracts

**Scope:** `task_schedule_due_dates.rs`, `task_get.rs`, `task_list.rs`. **Coverage:** basic dates/filtering. **Decision:** D-08.

Cover set/change/clear, omitted versus null due dates, equivalent timezone offsets, invalid dates, past dates, and creation with a date. Cover empty list, omitted filter, `all`, each supported status including archived, invalid filters, ordering, and get after updates/deletion.

**Acceptance:** dates represent the same instant in storage, response, and event; clearing returns null; unrelated fields remain unchanged. Filters include only matching rows; creation-time ordering follows the declared contract. Invalid input causes no domain mutation.

### QA-23 — Twitter remaining command outcomes and deletion semantics

**Scope:** `tweet_delete`, `tweet_retweet`, `tweet_like`, `tweet_unlike`, `user_follow`, `user_unfollow`, `direct_message_send`; [temporal semantics](test_app_twitter.md#temporal-semantics) and [command outcome matrix](../test_apps/twitter/AGENTS.md#command-outcome-matrix-94). **Coverage:** partial. **Decision:** D-10.

Cover already-deleted tweets, like/unlike/retweet against deleted originals, conflicting retweet IDs, wrong-author deletion, independent users' relationships, and remaining conflict/no-op outcomes.

**Acceptance:**
- Documented not-found/conflict/no-op responses agree with state; rejection preserves existing rows and produces no additional domain event/outbox effect. Soft-delete timestamps are correct.
- Deleting an original or intermediate retweet changes only the selected tweet, retaining existing likes, timeline rows, and retweet descendants without cascading or derived deletion events.
- Like, retweet, and unlike targeting missing or soft-deleted tweets return 404 without mutation/event/outbox effects. Deleted-target unlike covers both existing and absent likes, not just a missing-row case.
- Active-tweet unlike removes an existing like with its normal event/outbox effect; an absent like is still a successful no-op without those effects.
- Retweet-of-retweet preserves the immediate target in `retweeted_from` and event `original_tweet_id`; it does not normalize to a root. Active descendants of deleted ancestors remain eligible for like/retweet and active-target unlike. Direct operations on the deleted ancestor still return 404.
- Duplicate-retweet-ID behavior remains unchanged. Timeline read visibility/filtering is outside these stored-data and command assertions.

**Pending:** unlike currently checks row existence rather than active status. The deleted-target 404 rule is not implemented or verified by this decision; application-owned event recovery remains a separate obligation under D-05.

### QA-24 — Concurrent application mutations

**Scope:** both apps' mutation handlers and domain persistence. **Coverage:** absent.

Cover simultaneous follows/likes, follow versus unfollow, like versus unlike, delete versus like/retweet, duplicate creations, and Todo update/complete/reopen/delete against one item.

**Acceptance:** responses, final state, and emitted effects correspond to a valid ordering of accepted operations. There are no duplicate relationships, lost successful mutations, misleading success responses, overwritten conflicting identities, or events describing impossible states.

### QA-25 — Timeline fan-out recipients and recovery

**Scope:** [`timeline_fan_out.rs`](../test_apps/twitter/src/timeline_fan_out.rs). **Coverage:** happy fan-out/repeated fan-out. **Decision:** D-10.

Cover no followers, unrelated follower groups, multiple authors, repeated source events, partial fan-out followed by retry, malformed/wrong-variant events, follow/unfollow between posting and initial fan-out, membership changes between attempts, and deletion before delayed fan-out.

**Acceptance:** each attempt selects followers visible when it reads membership, not a post-time or first-attempt snapshot. A successful attempt leaves a row for each selected recipient with correct tweet/author IDs; no `(user_id, tweet_id)` pair is duplicated. Retries may add newly joined followers. Existing rows remain after unfollow, but a departed follower with no prior inserted row need not receive one on retry. No row is newly added for a user outside that attempt's selected set; historical rows from prior attempts may remain.

Delayed fan-out after soft deletion still creates historical references for current followers and preserves prior rows. Fan-out emits no domain event/outbox effect and adds no new triggers for retweets or follow changes. This contract does not establish timeline display rules, queue ordering, or stronger concurrency guarantees. Full scenario verification remains pending; existing happy-path/repetition tests are not proof of these temporal cases.

### QA-26 — Real event replay and application outbox reads

**Scope:** both apps' `OutboxSubscriber`, Twitter `record_event_payload`, both outbox HTTP modules. **Coverage:** partial Twitter; Todo lacks dedicated outbox HTTP tests. **Decision:** D-09.

Cover repeated delivery of the **same source event**, distinct events with identical payload, malformed event/metadata, subscriber failure, and outbox reads with empty/all-status data, attempts, optional publication time, ordering, and corrupt stored payload.

**Acceptance:** Twitter retains one original row per source event ID; distinct identities remain distinct. Todo follows its approved replay policy. Invalid delivery creates no misleading row. Reads are nonmutating and preserve each app's actual response contract: Todo array versus Twitter `items` envelope.

**Evidence weakness:** Twitter's current duplicate-named outbox test sends one command and counts one output; it does not redeliver an event. Todo currently generates a new outbox ID per delivery.

### QA-27 — HTTP input, error, and response contracts

**Scope:** both apps' HTTP modules, `AppError`, `interpret_dispatch_error`. **Coverage:** mostly status-only negative cases.

Cover malformed JSON, missing/null required values, wrong field types, invalid UUID/enum/date, unknown command tags, wrong nested shapes/content type, unsupported route/method, and database failure.

**Acceptance:** each case has the documented exact status and response shape; framework parsing and application errors are distinguished where their contracts differ. No unintended business/event/outbox mutation occurs. Responses do not disclose sensitive database details. Successful responses contain accurate IDs, optional fields, timestamps, and entity tags rather than merely a success code.

### QA-28 — Production binary wiring and restart behavior

**Scope:** both `src/assembly/bin/*app.rs`, startup configuration, migration entry points, worker startup. **Coverage:** absent; current app tests construct their own servers.

Cover default/`serve`/`migrate`/unknown commands, missing/invalid configuration, occupied bind address, failed migration, startup with pending work, shutdown during processing, and restart.

**Acceptance:** exit outcomes are correct, configured addresses/ports are honored, production routes/health/documentation endpoints match their declared contract, and pending work advances without a new HTTP request. Restart preserves accepted work. Startup/health success does not falsely represent unavailable required dependencies. Shutdown follows D-07: application-owned intake/tasks are stopped and awaited by the application, kernel-owned workers finish in-flight attempts without queue draining, and `wait()` does not cancel them or return before they stop. New external dispatch through existing kernel handles is rejected, while accepted in-flight follow-ups remain allowed; persistent follow-ups left queued survive restart. An unrecoverable worker failure leaves siblings running and is reported after all owned workers stop. A blocked shutdown is not falsely reported as complete, and no kernel deadline or automatic restart is assumed. These lifecycle outcomes remain pending implementation/verification.

## P1 — quality of the verification evidence

### QA-29 — Database-backed coverage is actually represented in release evidence

**Scope:** [`libs/outbox/tests`](../libs/outbox/tests), [`Makefile`](../Makefile), [CI workflows](../.github/workflows).

**Acceptance:** release evidence identifies which database and broker requirements executed, passed, failed, or were skipped. A green unit/library run is not represented as proof of PostgreSQL reservation or broker behavior. Required application suites are represented independently.

**Current gaps:** all eight outbox database tests are ignored by default. `.github/workflows/ci.yml` refers to a separate Twitter workflow, but the repository has only `ci.yml` and `test-app-todo.yml`. The format-check step runs `make fmt`, whose target formats files rather than rejecting formatting differences. These are verification gaps, not proof of application failure.

### QA-30 — Existing tests have valid expectations and independent outcomes

**Scope:** outbox `diesel_storage.rs`/`lifecycle.rs`; both app `tests/utils.rs`; inbox AMQP conversion test.

- Repeated/full/concurrent suite results do not depend on earlier table resets, surviving workers, or process-local locks in another test binary.
- Startup failure is reported as startup failure; no health wait silently continues into unrelated assertions.
- Failed tests leave no accumulating servers, workers, or connections that alter later results.
- Assertions follow one coherent time/retry contract.
- Test setup uses valid values, and exercised code represents the production behavior being claimed.

**Observed weaknesses:** outbox schema locks cover reset only and are separate per file; a stale-sweep test asserts a future retry time then immediately expects reservation. App locks are process-local; Twitter cancellation uses a fixed delay; health loops can exhaust without explicit startup failure. The inbox metadata test uses unsafe partially initialized delivery construction and a test-only conversion implementation, limiting the evidence it provides for production intake.

## P2 — compatibility, security boundaries, and operational assurance

### QA-31 — Supported public APIs and feature combinations

**Scope:** all package manifests/`lib.rs`, kernel public API, [feature ADR](adr/006-cargo-feature-flags.md), repository module rules. **Coverage:** outbox export smoke tests only. **Decision:** D-11.

**Acceptance:** supported default, no-default, Diesel, AMQP, and combined configurations expose usable public APIs. External consumers can construct required application types through the approved `io` interface without internal-module access. Disabled adapters do not impose unintended infrastructure dependencies. Supported public imports and serialized contracts remain compatible across upgrades.

**Source risk:** commanding exposes `NewCommand` under the Diesel feature although direct-mode envelope construction also needs it. Crate compilation alone does not establish caller usability. Kernel root-level exports also need reconciliation with the single-interface rule.

### QA-32 — Packaged artifacts and reproducible validation boundaries

**Scope:** root Makefile package/publish targets, Cargo manifests, `.cargo/config.toml`, clean-checkout workflows.

**Acceptance:** packaged crates contain their required source/assets and resolve supported dependencies outside the source checkout. Declared versions/registry dependencies agree. Validation does not rely on untracked build artifacts or an undocumented local registry being available. The supported toolchain/platform envelope is explicit, and each supported configuration has an observed result.

**Current risk:** package targets use `--no-verify`; registry configuration points to a local `margo` index. Neither fact alone proves packaging failure, but normal local test success does not establish distributability.

### QA-33 — Security and privacy at the declared application boundary

**Scope:** caller-supplied user/author IDs, mutation endpoints, inbox ingestion, outbox responses, error/log content. **Decision:** D-12. **Coverage:** no comprehensive security boundary established.

**Acceptance:** all declared trust and access restrictions hold for reads and mutations; malicious text is treated as data rather than query/control input; malformed/oversized input has a defined noncorrupting outcome. Errors and diagnostics reveal no credentials or unintended sensitive payloads. Direct-message content in outbox output is visible only within the explicitly approved exposure boundary.

The demonstration apps currently accept caller-supplied identities and do not establish authentication. Authenticated-user isolation must not be described as an existing guarantee. Deployment acceptance remains incomplete until the intended exposure is stated.

### QA-34 — Capacity, recovery, and operational visibility

**Scope:** queue consumers, pool, workers, fan-out, list/outbox reads, error-history growth, logging. **Decision:** D-12. **Coverage:** absent.

Cover sustained queues, retry storms, unavailable/slow database or broker, many followers, large payloads/results, growing error histories, simultaneous consumers, and recovery after dependency restoration.

**Acceptance:** within the approved workload envelope, throughput, latency, memory, connections, shutdown time, and recovery meet their declared limits; accepted work is neither lost nor permanently starved. Operational evidence distinguishes transient failure, exhausted/dead work, stale reservations, lost ownership, and worker failure, with sufficient identity/trace context and without sensitive leakage. Numeric pass/fail criteria remain unspecified until D-12 is resolved.

## Completion criteria for this backlog

An item is complete only when:

1. Its applicable contract decisions are resolved and its expected outcomes are explicit.
2. Every listed scenario has an observed result for each named component/mode, or a documented scope exclusion.
3. Assertions cover the relevant response, durable state, ownership, metadata, and side effects—not only absence of an error.
4. Failures and skipped cases remain visible; indirect happy-path coverage is not substituted for an unexercised failure boundary.
5. Any source-level risk identified here is either disproved by evidence, confirmed as a defect, or recorded as an intentional supported limitation.

**Highest-priority handoff:** QA-01 through QA-10, together with D-01 through D-09. These contain the most consequential risks of silent success, message loss, unrecoverable retries, validation bypass, and misleading API outcomes.
