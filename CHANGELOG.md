# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Atomic batch recording for the outbox: `OutboxRecorder::record_batch` (envelopes),
  `OutboxStorePort::record_batch` (entries) and, with the `diesel` feature,
  `record_batch_on_connection`, which records on the caller's own Diesel connection
  inside the caller's transaction so business writes and outbox events commit or
  roll back together. Recording one event is a batch of one.
- `RecordingError` and `RecordingOutcome` (`NoRowsAdded` or `Uncertain`), separate
  from the reservation and publishing `OutboxError`.
- A handler example with a bounded retry loop (`examples/record_with_retry.rs`) that
  states and tests the repeat-safe business write requirement, the
  reasoning for the deadlock guarantee, release-build batch-size measurements, and
  specification sections on recording, timeouts, blocking sessions and recovery.
- A dedicated PostgreSQL CI job that runs every outbox database suite from a required
  inventory, and package-isolated `serde_json` feature and feature-combination checks.

### Changed (breaking)

- **Recording returns `RecordingError`.** Existing matches on the old recording errors
  stop compiling. Applications that convert recording failures into the kernel's
  `OutboxError` (it is still re-exported by the kernel, which is unchanged) must now
  convert them explicitly; backend errors never convert implicitly.
- **Third-party production recording stores are unsupported.** Only the Diesel
  PostgreSQL repository is. `OutboxStorePort` stays unsealed for test doubles, which
  must adopt the required `record_batch` operation and the recording error type.
  Custom implementations of the reservation, processing, sweep and publisher ports are
  unaffected.
- **New validation applies to single-event recording too, with no opt-out.** Whole
  batches are validated before any database access. They reject: a repeated event ID
  (batch calls only), an entry ID that differs from its metadata event ID, a payload
  that is not exactly one JSON object, duplicate decoded keys, lone surrogates, more
  than 127 nested containers, NUL in a metadata string, a blank routing key, and a
  sequence outside 1 to `i64::MAX`. Sequence bounds, blank routing keys through the
  recorder, and NUL in metadata or raw NUL in a payload were already failing
  (the last two as database errors); everything else previously succeeded.
- **Payloads must be JSON objects.** Producers of non-object payloads have no direct
  path: unchanged they are rejected, and wrapped they conflict with their stored rows
  and change the bytes consumers receive. Inventory retryable and exhausted source
  events and retained requests that may reuse old IDs, not just pending outbox rows.
  Finish old-format processing before cutover or keep an explicit old-format recovery
  procedure, and coordinate with consumers before upgrading. Deleting or rewriting
  existing outbox rows is not a safe conflict repair.
- **Conflicting replays now fail.** Recording an existing event ID previously always
  succeeded and kept the stored row. It now succeeds only if the immutable content
  matches (payload by exact written form, message ID, routing key, event type, content
  type, sequence), including for existing rows with mismatched event IDs or unreadable
  metadata, which now conflict.
- **Retries must keep the same content.** Producers that rebuild an event on retry with
  regenerated payload fields (such as a timestamp), fresh message IDs or sequences, or
  differently written numbers will now conflict on every retry. Audit them before
  upgrading. A later deploy that changes an event's payload shape, routing or content
  type does the same to in-flight redeliveries. A fresh event ID on retry causes no
  conflict but creates another row that can publish a duplicate.
- **Event subscribers that record must expect permanent conflicts.** Under TwoPhased
  eventing, propagating one fails the event and retries it with the same event ID and
  payload: earlier subscribers re-run, later ones never run, and the exhausted event
  stays `Failed`. Under Direct eventing the error returns to the caller with no retry,
  and each command execution assigns new event IDs. A subscriber that uses the delivered
  event ID as its outbox event ID therefore gets replacement IDs if the caller
  re-dispatches the command; after an uncertain outcome that can publish a duplicate.
  Such a subscriber must take its IDs and replay-compared content from something stable
  across command retries, or the caller must reconcile or escalate.
- **Rolling upgrades look intermittent.** Old instances accept conflicts that new ones
  reject, so failures appear on some requests only. Rolling a version back needs no
  schema rollback, but it does not reverse already published bytes or restore the
  stricter recording guarantees on old instances.

## [0.5.0] - 2026-11-05

### Changed

- Rename public export facades from `io` to `prelude` without compatibility aliases.
- Consolidate HTTP contracts and Axum/Poem adapters into core crate features.
- Use generic consumers backed by unified repositories and update kernel assembly.
- Bump all crate versions and internal dependency requirements to `0.5.0`.

### Added

- Shared `mulac_http` support crate and database-free HTTP adapter test harness.
- Prelude architecture checks and updated CI and publishing workflows.

### Removed

- Remove `mulac_commanding_api`, `mulac_eventing_api`, `mulac_inbox_api`,
  `mulac_outbox_api`, `mulac_axum`, and `mulac_poem`. Their services and adapters
  now live behind features on the core crates. See
  [ADR-011](docs/adr/011-core-http-features.md) for the import migration.

## [0.4.1] - 2026-10-31

### Changed

- Rename `commanding_api`, `eventing_api`, `inbox_api`, and `outbox_api` to
  `mulac_commanding_api`, `mulac_eventing_api`, `mulac_inbox_api`, and
  `mulac_outbox_api`. Update dependency names and Rust imports accordingly.
  This is a source-level migration for users of these APIs, despite the patch
  version; the previously published package names remain available at `0.4.0`.
- Bump library, kernel, and test application versions and internal dependency
  requirements to `0.4.1`.
- Publish through `~/work/labs/crates` to the hosted GitHub Pages registry.
  `make pack_and_publish` packages dependencies in order, verifies archive
  checksums, and commits and pushes the registry artifacts. Identical retries
  are safe; existing versions with different bytes are rejected.

## [0.4.0] - 2026-10-31

### Added

- Framework-neutral inbox, commanding, eventing, and outbox API crates.
- Poem and Axum HTTP adapters for queue APIs, including optional OpenAPI support.
- Optional application-owned event sequences carried through command dispatch,
  event storage, and outbox metadata while preserving framework delivery IDs.

### Fixed

- Kernel startup no longer installs successful no-op inbox stores or event
  subscribers. Direct startup now requires an explicit `inbox_store`; its
  recording errors propagate to callers.
- Both startup modes reject the unimplemented `outbox_subscriber` shortcut.
  Register a recorder-backed subscriber through `event_subscriber` instead.
- Direct startup rejects gateway-dependent subscriber factories rather than
  dropping them. These factories remain supported by `start_persistent`.

### Changed

- Migrate the todo test application from SQLx to Diesel, sharing one pool for
  application queries and kernel persistence.
- Bump all library, kernel, and test application versions and internal version
  requirements to `0.4.0`.
- **Startup compatibility:** applications using `start()` must supply an inbox
  store explicitly. `KernelConfig::database_url` does not provision one.
  The direct-only inbox store option is rejected by `start_persistent`.
  See [kernel startup contracts](kernel/SPEC.md) for supported configurations.

## [0.1.0] - 2026-05-24

Initial release. mulac is a set of architecture building blocks for reliable
event-based systems, providing Inbox/Outbox patterns alongside pluggable
command and event dispatchers.

### Added

- **Workspace layout**: unified Cargo workspace under `libs/` with shared
  dependencies, plus a top-level `kernel` crate and `test_apps/` integration
  examples.
- **`inbox` library**: core domain, assembly, feature modules, and AMQP
  transport for ingesting external messages reliably.
- **`outbox` library**: outbox flows, adapters, status model, and SQL schema
  for transactional message dispatch.
- **`commanding` library**: command gateway, handler registry, and status
  models implementing the commanding module along hexagonal-architecture
  boundaries.
- **`eventing` library**: event dispatcher and subscriber registry following
  the hexagonal-architecture layout.
- **`mulac_diesel` library**: shared Diesel-based persistence helpers.
- **`kernel` crate**: top-level composition crate exposing registries,
  workers, and helpers through a single `prelude` module.
- **Test applications**: `todo` and `twitter` example apps demonstrating
  end-to-end integration, with shared HTTP helpers and assertion macros.
- **Docker infrastructure**: consolidated local development setup.
- **CI**: GitHub Actions workflow running `make test` with all features.
- **Makefile**: `cargo make` targets including `fmt`, `test`, and
  `test-apps` workflows.

### Documentation

- Architecture and component specs (`docs/architecture-spec.md`,
  `docs/components.md`).
- Per-module specs and implementation plans for inbox, outbox, commanding,
  and eventing.
- Developer guidelines, codestyle guide, ADRs, and contracts reference.
- Reviews of the `todo` and `twitter` test apps.
- LLM tooling and attribution conventions (see `CLAUDE.md`, `AGENTS.md`).

### Conventions

- Every feature module exposes a single `pub mod prelude` as its only public
  interface; internal sub-modules (`commanding`, `eventing`, `http`,
  `infra_sqlx_pg`, ...) remain private.
- Commits follow the [Conventional Commits](https://www.conventionalcommits.org/)
  specification.

[0.1.0]: https://github.com/your-org/mulac/releases/tag/v0.1.0

[0.4.0]: https://github.com/nulllabsdev/mulac/releases/tag/v0.4.0

[0.4.1]: https://github.com/nulllabsdev/mulac/releases/tag/0.4.1

[0.5.0]: https://github.com/nulllabsdev/mulac/releases/tag/0.5.0
