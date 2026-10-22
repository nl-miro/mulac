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
| D-01 | Retry count definition, default limit, backoff formula, cap, and stale-release state for each component | [contracts.md](contracts.md) describes exponential delays, failure-based counting, and sweep to `received`; reservation code increments attempts on reserve, and component implementations use linear delays and sweep to `failed`. |
| D-02 | Terminal behavior for exhausted stale reservations | Inbox/command/event sweeps can write `failed` while reservation eligibility excludes exhausted entries. Sweep limits/caps are not consistently applied. |
| D-03 | Inbox deduplication key and retention window | Contracts describe source-plus-message-ID deduplication with a retention window; current recording uses UUID identity. Missing/malformed transport IDs can generate a new ID on each delivery. |
| D-04 | Supported kernel capabilities in direct and persistent modes | Direct startup substitutes no-ops for command-gateway subscriber factories; both modes register no-op outbox subscribers; direct inbox storage is a no-op. Configuration is stored as `_config` without being applied. |
| D-05 | Identity and business effects across command retries | Command execution generates fresh event IDs; an idempotent/no-op repeated mutation may return no event even when an earlier event handoff failed. At-least-once execution is not an exactly-once business-effect guarantee. |
| D-06 | Broker acceptance versus routing success | Outbox accepts `Confirmation::NotRequested` and every `Ack(_)`, including returned-message information. Missing confirmation and mandatory returns need explicit outcomes. Durable broker persistence must not be inferred from acceptance alone. |
| D-07 | Cancellation, stream closure, shutdown, and supported runtime contexts | Inbox stream closure is an error in code but described differently in older specification prose. Kernel `wait` methods differ and do not own worker joins. Synchronous publishing/runtime helpers have context restrictions. |
| D-08 | Todo status, update, date, and inbox-delete contracts | Repeated complete/reopen, archived transitions, omitted versus null fields, past due dates, equal-time list ordering, and successful inbox-delete response are incompletely specified. |
| D-09 | Application inbox recovery and Todo outbox replay policy | Recovery of recorded-but-undispatched or failed app inbox messages is unclear. Todo outbox assigns a fresh UUID on delivery; Twitter deduplicates by event ID. |
| D-10 | Twitter temporal semantics | Follower membership at post-time versus fan-out-time; deleted originals' existing likes/timelines/retweets; retweet-of-retweet behavior. |
| D-11 | Public API and error/metadata consistency | Repository guidance requires an `io` facade, while kernel exposes root-level types. Event gateway inherent/trait calls classify some errors differently. Source/correlation propagation is incomplete in some paths. |
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
- Acknowledgement errors, negative-acknowledgement errors, and recording-task failures remain distinguishable and cannot falsely signal acceptance.
- Cancellation at the recording/acknowledgement boundary preserves the documented in-flight guarantee.

### QA-05 — Outbox publication requires the agreed broker evidence

**Scope:** [`amqp_publisher.rs`](../libs/outbox/src/amqp_publisher.rs); [`outbox_consumer.rs`](../libs/outbox/src/outbox_consumer.rs). **Coverage:** metadata/property tests only for the adapter. **Decisions:** D-06, D-07.

- No successful publication/completion precedes the required broker acknowledgement.
- Negative confirmation, failed confirmation setup, channel/connection closure, and interrupted publication yield transport failure rather than completion.
- Missing confirmation and mandatory returned/unroutable messages have the explicit agreed outcome.
- Default and configured exchanges/routing keys preserve destination semantics and payload identity.
- Supported synchronous/asynchronous runtime contexts neither panic nor deadlock.
- Broker restart behavior matches the explicitly claimed durability boundary; broker acceptance is not misreported as downstream consumer processing.

### QA-06 — Recovery across every responsibility-transfer boundary

**Scope:** inbox → command → event → subscriber/outbox → broker; application handlers and subscribers. **Coverage:** one outbox completion-failure case; otherwise incomplete. **Decisions:** D-05, D-09.

Cover interruption after durable recording, reservation, business mutation, event acceptance, subscriber side effects, broker acceptance, and immediately before completion.

**Acceptance:** accepted work remains recoverable or has reached its legitimate terminal state; no stage reports completion before the next responsibility boundary accepts it. Retries may repeat processing under the at-least-once contract, but required events cannot disappear because a repeated mutation becomes a conflict/no-op. Repeated publication of the same outbox entry preserves payload and message ID, including caller-supplied IDs. Completed/dead entries do not spontaneously re-enter processing.

### QA-07 — Recording deduplication preserves accepted data

**Scope:** all `record_*` modules and storage insert paths. **Coverage:** selected outbox row-count coverage; otherwise incomplete. **Decision:** D-03.

- Sequential and concurrent recording of the same identity have the agreed single-entry outcome.
- Same identity with different payload, type, routing, metadata, or timestamps does not silently replace accepted data.
- Re-recording reserved, failed, completed, or dead work does not reset its lifecycle or attempts.
- Distinct IDs with identical payload remain distinct work.
- Inbox source and retention-window boundaries follow the approved deduplication contract.
- Missing/malformed inbound IDs have explicit redelivery semantics; generated IDs must not be described as stable deduplication across independent deliveries.
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
- Zero, one, and multiple produced events follow the declared handoff semantics.
- Partial event/subscriber delivery is not falsely described as rollback; persistent retry may revisit successful recipients.
- Zero matching subscribers is successful; unrelated subscribers never receive the event.
- Inherent and trait-based event gateway calls have the approved error classification.

### QA-14 — Typed envelopes, identities, and trace metadata

**Scope:** `kernel::NewCommandEnvelope::into_gateway_envelope`, `wrap_handler`, all envelope conversions, app event serializers. **Coverage:** selected inbox/outbox conversions only. **Decisions:** D-05, D-11.

Every command/event variant preserves its discriminator, payload, supplied identity, and optional fields. Invalid serialization/deserialization fails at the stated boundary without invoking the handler. Generated events have the required identity format; explicit correlation survives, and fallback correlation/causation follows the command contract. Source propagation and subscriber-generated command metadata have explicit outcomes. Follow/unfollow and like/unlike event representations remain distinguishable. Replay identity expectations are separate from ordinary fresh-event uniqueness.

### QA-15 — Registry validation and exact routing

**Scope:** [`command_handler_registry.rs`](../kernel/src/command_handler_registry.rs), [`event_subscriber_registry.rs`](../kernel/src/event_subscriber_registry.rs), `KernelBuilder::validate`. **Coverage:** absent.

Cover duplicate command handlers; duplicate `(event_type, subscriber_name)` pairs across direct/factory registrations; the same subscriber name on different event types; reserved `_outbox` names; repeated outbox registration; unknown and empty command/event types; registry construction outside the builder.

**Acceptance:** supported registrations resolve only to their exact recipients; unknown commands invoke no unrelated handler; duplicates/reserved names produce the agreed error. Builder validation and independently constructed registries must have explicitly defined differences, not accidental last-write-wins behavior.

### QA-16 — Persistent draining, workers, cancellation, and shutdown

**Scope:** `PersistentKernelState::dispatch_command`, both kernel handles, [`workers.rs`](../kernel/src/workers.rs), inbox `WorkerLoop::run`. **Coverage:** absent directly. **Decision:** D-07.

- Zero drain rounds mean durable acceptance, not completed execution; finite rounds leave excess entries and long command/event chains recoverable.
- Command-then-event sequencing and subscriber-produced work follow the configured round boundary, including queues larger than the 64-entry drain batch.
- Background workers process pending work without an HTTP-triggered drain; errors and blocking-task failures have the declared continuation behavior.
- Cancellation before polling, during idle waits/backoff, and during active work preserves the defined in-flight boundary.
- Parent/child cancellation, repeated shutdown, dispatch after shutdown, and `wait` have explicit outcomes without falsely claiming joined workers.

### QA-17 — AMQP intake validity and stream lifecycle

**Scope:** inbox `AmqpClient`, `AmqpTransport`, `WorkerLoop`. **Coverage:** one metadata example. **Decisions:** D-03, D-07.

Cover valid/absent/malformed UUID properties, empty exchange/routing key, empty/non-ASCII payloads, invalid UTF-8, stream closure, transport errors, and cancellation during receive/backoff.

**Acceptance:** valid bytes and metadata retain their declared meaning. Invalid UTF-8 is never stored and receives the specified nonretry/poison-message disposition; later valid deliveries remain processable. No polling occurs during contractual backoff. Closure and failure are distinguishable from clean cancellation. Reconnection/redelivery behavior satisfies only the explicitly supported guarantee.

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

**Scope:** `tweet_delete`, `tweet_retweet`, `tweet_like`, `tweet_unlike`, `user_follow`, `user_unfollow`, `direct_message_send`; [outcome specification](twitter-app-reorganize-plan-final.md). **Coverage:** partial. **Decision:** D-10.

Cover already-deleted tweets, like/unlike/retweet against deleted originals, conflicting retweet IDs, wrong-author deletion, independent users' relationships, and remaining conflict/no-op outcomes.

**Acceptance:** documented not-found/conflict/no-op responses agree with actual state. Rejection preserves original rows and produces no additional event/outbox effect. Soft-delete timestamps are correct. Existing likes, timelines, retweets, and retweets of retweets follow the approved policy.

### QA-24 — Concurrent application mutations

**Scope:** both apps' mutation handlers and domain persistence. **Coverage:** absent.

Cover simultaneous follows/likes, follow versus unfollow, like versus unlike, delete versus like/retweet, duplicate creations, and Todo update/complete/reopen/delete against one item.

**Acceptance:** responses, final state, and emitted effects correspond to a valid ordering of accepted operations. There are no duplicate relationships, lost successful mutations, misleading success responses, overwritten conflicting identities, or events describing impossible states.

### QA-25 — Timeline fan-out recipients and recovery

**Scope:** [`timeline_fan_out.rs`](../test_apps/twitter/src/timeline_fan_out.rs). **Coverage:** happy fan-out/repeated fan-out. **Decision:** D-10.

Cover no followers, unrelated follower groups, multiple authors, repeated source events, partial fan-out followed by retry, malformed/wrong-variant events, relationship changes, and tweet deletion around processing.

**Acceptance:** exactly one row exists per intended `(user_id, tweet_id)`; IDs/author are correct; unrelated users receive no row; missing intended recipients are eventually completed after transient failure. Fan-out itself produces no unintended domain event/outbox effect. Recipient timing and deletion behavior follow the approved contract.

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

**Acceptance:** exit outcomes are correct, configured addresses/ports are honored, production routes/health/documentation endpoints match their declared contract, and pending work advances without a new HTTP request. Restart preserves accepted work. Startup/health success does not falsely represent unavailable required dependencies. Shutdown follows D-07.

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
