# Outbox bulk inserts and repositories: implementation report

## TL;DR

The `adding-outbox-repository` branch implements **real PostgreSQL bulk inserts**, not a loop of independent single-row transactions. It validates the batch, sorts rows by ID, inserts chunks of up to **5,000 rows inside one transaction**, and checks existing content when an insert encounters an existing ID. The pooled recording API and the caller-owned-connection API share this implementation.

Recording is idempotent by **event ID**, but only when the persisted message content matches the replay policy. Duplicate IDs *within one request* are rejected, even if the entries are identical. Payloads must be JSON objects. A conflicting replay fails the whole PostgreSQL recording operation; it does not overwrite the stored row.

The repositories are **existing, use-case-specific wrappers around port traits**, not a newly unified database repository. The recorder repository delegates writes; the consumer repository delegates reservation and lifecycle updates. PostgreSQL transactionality comes from the storage implementation, not from the repository abstraction. To include business writes and outbox writes in the same transaction, use `record_bulk_on_connection` with the **same connection inside an outer transaction**.

The design is coherent and the executed checks pass, but its database and concurrency guarantees are **not fully verified by this investigation**: 10 PostgreSQL tests were ignored, and several important acceptance cases have no test yet. Repeated validation, full-batch allocation, and full-row replay read-back are identifiable performance costs, not measured bottlenecks.

## Itemized findings

1. **[Repository composition](#2-repository-architecture):** recorder and consumer repositories remain separate wrappers using `Arc<dyn ...>` storage ports; these wrappers predate the branch's bulk changes.
2. **[Public recording paths](#3-public-apis-and-call-flows):** `record`, `record_bulk`, and the Diesel-only `record_bulk_on_connection` have distinct entry points but converge on the same PostgreSQL bulk-write logic.
3. **[Validation before writes](#4-validation-identity-and-new-row-state):** empty batches are no-ops; duplicate IDs, blank routes, invalid sequences, and non-object JSON are rejected before insertion.
4. **[Atomic chunking](#5-postgresql-bulk-insert-algorithm):** globally sorted rows are inserted in 5,000-row chunks under one transaction, with no insert `RETURNING` and no read-back on the all-new fast path.
5. **[Replay policy](#6-idempotency-and-replay-semantics):** parsed payload and selected metadata must match; changed tracing context is tolerated but does not update the stored row.
6. **[Transaction composition](#7-transactions-and-error-behavior):** the connection-based helper supports nested savepoints; the pooled API does not automatically join a business transaction.
7. **[Integration limits](#8-consumer-and-application-integration):** bulk recording is not bulk broker publication; the sample applications write a different table, and kernel-managed automatic outbox recording is unsupported.
8. **[Performance and locking](#9-performance-and-concurrency-review):** sorting helps establish insertion lock order, but it is not a deadlock-freedom guarantee; validation and allocation still scale with the entire batch.
9. **[Validation evidence](#10-tests-and-observed-results):** 28 no-default-feature tests and 36 default-feature tests passed; all-target/all-feature compilation passed; PostgreSQL execution remains unverified here.
10. **[Priorities](#11-prioritized-recommendations):** run the database suite and add concurrency, transaction-composition, replay-state, and boundary coverage before treating the implementation as production-validated.

## 1. Scope and evidence

| Item | Investigated snapshot |
|---|---|
| Investigation date | 2026-11-06 |
| Branch | `adding-outbox-repository` |
| Commit | `74513331bd725fddee23c5854a9ceeb607b3e4fd` (`wip`) |
| Comparison baseline | Parent commit `87083a8` |
| Crate | `mulac-outbox`, workspace version `0.3.0` |
| Public facade | `mulac_outbox::io` |
| Scope | Recording, repository composition, Diesel persistence, relevant consumers/callers, and tests |

This is a report about the checked-out branch, **not** the later `0.5.0`/`prelude` architecture discussed elsewhere. The working tree was clean before creating this report. No implementation changes are included.

Evidence comes from source inspection, the parent-to-HEAD diff, repository-wide caller searches, and the commands in section 10. Statements about SQL behavior distinguish implementation intent from database behavior actually tested. No throughput benchmark, concurrent-writer experiment, live broker test, or PostgreSQL integration test was run.

Primary sources are [`record_events.rs`][recording], [`assembly/infra_diesel.rs`][storage], [`assembly/application.rs`][ports], [`outbox_consumer.rs`][consumer], and [`tests/bulk_recording.rs`][bulk-tests]. The branch's [recording-parity decisions][parity] supply intent; source and test assertions supply the implementation evidence.

### What changed relative to the parent

The parent already had `OutboxRecorder`, `OutboxRecorderRepository`, `OutboxConsumerRepository`, and the pooled Diesel storage types. Its single-row storage method performed `INSERT ... ON CONFLICT (id) DO NOTHING`, without the new replay-content verification.

The relevant additions at `7451333` are:

- Bulk methods on the recorder, recorder repository, and storage port.
- Whole-batch validation and typed payload/duplicate/conflict/missing errors.
- A shared transactional PostgreSQL writer with sorting, chunking, and replay verification.
- A public, Diesel-gated helper for caller-owned connections.
- Sequence metadata handling and unit/database tests for the new behavior.

This is an extension and tightening of an existing repository design. It is not a replacement of the two wrappers with a single CRUD repository. The comparison can be reproduced with:

```bash
git diff 87083a8 7451333 -- libs/outbox
```

## 2. Repository architecture

### Recorder-side composition

`OutboxRecorder` owns `Arc<OutboxRecorderRepository>`. The repository owns `Arc<dyn OutboxStorePort>` and exposes a public constructor, but its `insert` and `insert_bulk` methods are `pub(super)`: application callers construct the repository and use the recorder rather than invoking these methods directly.

The repository validates entries and delegates to the port. It contains **no SQL, connection acquisition, transaction implementation, retry loop, or persistence buffer**. Its own comment correctly says that atomicity depends on the adapter. A custom adapter can satisfy the trait's signatures without supplying PostgreSQL's atomicity or replay policy. [Source: `record_events.rs:19–79`][recording]

`OutboxStorePort: Send + Sync` requires both methods:

```rust
fn record(&self, entry: &NewOutboxEntry) -> Result<(), OutboxError>;
fn record_bulk(&self, entries: &[NewOutboxEntry]) -> Result<(), OutboxError>;
```

There is no default bulk implementation that loops over `record`. That avoids silently providing a non-atomic fallback, but adding the required method is a source-breaking change for custom implementations. The trait documentation itself does not specify an enforceable all-or-nothing contract. [Source: `assembly/application.rs:120–129`][ports]

The Diesel factory exported as `recorder_repository(pool)` constructs `OutboxStoreStorage`, wraps it as the port implementation, and returns `Arc<OutboxRecorderRepository>`. `OutboxStoreStorage` holds the pool and implements both write methods. [Sources: `record_events.rs:1–16`][recording], [`assembly/infra_diesel.rs:379–405`][storage]

### Consumer-side composition

`OutboxConsumerRepository` is a separate, cloneable wrapper with two fields:

- `Arc<dyn OutboxReservePort>` for reserving messages.
- `Arc<dyn OutboxProcessPort>` for marking them completed, failed, or dead.

Its public methods forward to those ports. The default `consumer_repository(pool)` factory creates one `OutboxConsumerStorage` and shares it between both roles. Independent doubles can be injected for tests. This repository does not record messages and does not own the publisher. [Source: `outbox_consumer.rs:6–18,99–183`][consumer]

### Visibility and feature boundaries

External imports go through the crate's `io` facade. Core envelopes, ports, repositories, and the recorder exist without Diesel. The pooled factories, PostgreSQL storage types, and `record_bulk_on_connection` are gated by `diesel`, which is enabled by default. AMQP is a separate optional feature.

The actual entry-writing helper, `record_entries_on_connection`, is crate-visible, not an external API. The public connection helper accepts envelopes, while the low-level store trait accepts entries. [Sources: `lib.rs:1–50`][facade], [`Cargo.toml:6–10`][manifest], [`record_events.rs:317–329`][connection-helper]

## 3. Public APIs and call flows

### API map

Signatures below omit module prefixes and show `&self` where applicable.

| Entry point | Input / result | Purpose |
|---|---|---|
| `OutboxRecorder::new` | `Arc<OutboxRecorderRepository> -> Self` | Assemble a storage-independent recorder |
| `OutboxRecorder::record` | `&self, &NewOutboxEnvelope -> Result<(), OutboxError>` | Validate and record one envelope |
| `OutboxRecorder::record_bulk` | `&self, &[NewOutboxEnvelope] -> Result<(), OutboxError>` | Validate and record a batch |
| `OutboxRecorderRepository::new` | `Arc<dyn OutboxStorePort> -> Self` | Inject a concrete store or test double |
| `recorder_repository` | `DbPool -> Arc<OutboxRecorderRepository>` | Assemble the Diesel-backed recorder repository |
| `OutboxStoreStorage::new` | `DbPool -> Self` | Construct the pooled PostgreSQL store |
| `OutboxStorePort::record_bulk` | `&self, &[NewOutboxEntry] -> Result<(), OutboxError>` | Adapter-level batch contract |
| `record_bulk_on_connection` | `&mut PgConnection, &[NewOutboxEnvelope] -> Result<(), OutboxError>` | Record using a caller's connection/transaction |

Recording returns **unit on success**, not an inserted-row count or per-entry result. In PostgreSQL, successful replay and successful new insertion are intentionally indistinguishable through these APIs. The affected-row count is internal to the fast-path decision. [Sources: `record_events.rs:27–78,321–327`][recording], [`assembly/infra_diesel.rs:420–450`][storage]

### Pooled path

```text
OutboxRecorder::record_bulk(envelopes)
  → empty check; validate envelope routing/sequence
  → convert all envelopes to NewOutboxEntry
  → OutboxRecorderRepository::insert_bulk(entries)
      → validate all entries
      → OutboxStorePort::record_bulk(entries)
          → OutboxStoreStorage: validate; acquire pooled connection
          → record_entries_on_connection(connection, entries)
              → validate; build records; sort IDs
              → one transaction containing all chunks and verification
```

For single recording, the recorder invokes repository `insert`, which delegates to port `record`. The PostgreSQL implementation of `record` calls its own `record_bulk` with a one-element slice, so single-entry writes gain the same transaction and replay policy. This unification is specific to that adapter; a custom store can implement the two methods differently. [Sources: `record_events.rs:34–78`][recording], [`assembly/infra_diesel.rs:389–452`][storage]

### Caller-owned connection path

```text
record_bulk_on_connection(connection, envelopes)
  → convert all envelopes to NewOutboxEntry
  → record_entries_on_connection(connection, entries)
      → validate; build records; sort IDs
      → transaction, or nested savepoint inside a caller transaction
```

This path bypasses `OutboxRecorder`, its repository wrapper, and pool checkout, but **not validation**: the shared storage helper validates entries before issuing SQL. Conversion happens before validation on this path. [Source: `record_events.rs:317–329`][connection-helper]

## 4. Validation, identity, and new-row state

### Validation rules

| Condition | Behavior |
|---|---|
| Empty batch | Returns success without insertion; pooled bulk storage does not acquire a connection |
| Same entry ID twice in one batch | `Duplicate { id }`, even for identical content |
| Blank/whitespace-only routing key | `Routing("routing_key is required")` |
| `sequence == Some(0)` or exceeds `i64::MAX` | `Routing("sequence must be between 1 and i64::MAX")` |
| `sequence == None` | Accepted |
| Invalid JSON or a JSON array/scalar/null | `InvalidPayload { id }` |
| JSON object, including `{}` | Accepted; original text is retained |

`validate_entries` sorts references by row ID, checks duplicate neighbors, then validates routing, sequence, and payload in that sorted order. It parses JSON into `serde_json::Value` and requires the `Object` variant. The existing SQL schema stores payload as `TEXT`; it does not enforce this object-only rule for unrelated raw SQL writers. [Sources: `record_events.rs:282–314`][validation], [`outbox_entries.sql:1–15`][schema]

Validation is fail-fast, not a full validation report. The recorder also pre-checks envelope routing/sequence in input order before entry conversion. Consequently, error precedence can differ between the recorder and connection helper when a batch contains several kinds of invalid input. For example, the recorder can return a routing error before entry-level duplicate detection, whereas the connection helper reaches duplicate detection first. This does not permit partial writes; it affects which error callers observe. [Sources: `record_events.rs:63–95,282–327`][recording]

Validation does not trim or normalize a valid routing key: `" route "` passes the nonblank test and remains distinct from `"route"`. There is no corresponding nonblank event-type check, content-type/JSON consistency check, or prohibition on a nil UUID in these validators. These are observed boundaries, not automatically defects.

### Identity and conversion

- `NewOutboxEntry.id` is copied from `NewOutboxEnvelope.metadata.event_id`.
- Omitted `message_id` becomes `event_id`; explicitly provided `message_id` is preserved.
- The database primary key is the entry ID, not message ID. Equal message IDs with different event IDs can therefore produce separate rows.
- Each envelope conversion calls `Utc::now()` and sets both `scheduled_at` and `received_at` to that value. A batch does not receive one shared timestamp.
- Conversion clones payload and metadata rather than consuming the caller's envelopes.

[Sources: `record_events.rs:99–119`][recording], [`assembly/application.rs:98–117`][ports], [`assembly/domain.rs:97–139`][domain], [`outbox_entries.sql`][schema]

The insertion record initializes status to `Received` (`0`), attempts to `0`, and `updated_at` to `received_at`. `extra_info` is absent; reservation and completion fields are not part of the insertable record. In the provided schema those omitted nullable fields start null. Payload bytes are not reformatted into canonical JSON. [Source: `assembly/infra_diesel.rs:108–119,180–215`][record-shape]

**Lower-level invariant caveat:** callers can construct `NewOutboxEntry` directly and invoke the public store port. Its `id` and `meta.event_id` are separate public fields. Validation does not enforce equality between them. The envelope APIs establish that equality, but direct adapter clients are responsible for it. Replay verification also does not compare stored `meta.event_id` separately.

## 5. PostgreSQL bulk-insert algorithm

The substantive algorithm lives in `storage::record_entries_on_connection`, not in the repository wrapper. [Source: `assembly/infra_diesel.rs:408–452`][storage]

1. **Return immediately for an empty slice.** No transaction is opened.
2. **Validate the entire input.** A malformed later entry cannot allow earlier entries to be inserted.
3. **Build all insertable records.** Each record clones its payload and metadata.
4. **Sort the complete record vector by UUID.** Chunking occurs after sorting so each batch's insertion order is globally consistent.
5. **Enter `conn.transaction`.** All insert chunks and any verification query execute within this scope.
6. **Insert chunks of at most 5,000 records.** Each uses Diesel's multi-row `.values(chunk)`, `ON CONFLICT (id) DO NOTHING`, and `.execute(conn)`. Add the affected-row counts.
7. **Fast path:** if the sum equals the input length, return success without fetching rows.
8. **Otherwise, read back all requested IDs.** The query selects the full `OutboxEntryRecord`, filtered by input IDs, and applies `.for_share()`.
9. **Verify existence and replay content.** Missing rows or mismatches return typed errors from inside the transaction. That makes the operation fail rather than silently accepting a partial batch.

Equivalent control flow, rather than a literal SQL dump:

```text
BEGIN / SAVEPOINT
  INSERT up to 5,000 rows ... ON CONFLICT (id) DO NOTHING
  INSERT next chunk ...
  ...
  if affected_rows != requested_rows:
      SELECT full rows for requested IDs FOR SHARE
      verify all requested IDs and immutable content
COMMIT / RELEASE SAVEPOINT
```

On a normal transactional failure, the transaction/savepoint is rolled back. Newly inserted rows from earlier chunks do not remain just because those statements finished first. Pre-existing rows are not changed by `DO NOTHING`.

The fixed chunk size is documented in code as a safeguard against PostgreSQL's bind-parameter limit. The current record has eight non-optional fields plus optional `extra_info`; it is not a dynamically calculated batch limit or a payload-byte limit. Any future change to the record shape should revisit the chunking rationale. No benchmark justifies the particular value in the inspected code.

### What the fast path does and does not guarantee

The all-new path relies on the affected-row count and normal storage semantics; it does not re-read newly inserted content. If even one row was skipped, verification reads **all** IDs, not only the conflicted ones. That supports a mix of new and replayed entries and detects requested IDs absent from the result.

This is deliberately not an `INSERT ... RETURNING` design. A trigger or custom schema that transforms inserted payload/metadata is not independently checked on the all-new path. Such database customization would need a separate compatibility decision and tests; no such trigger is present in the supplied schema. The branch's [parity decisions][parity] explicitly retain the fast path and exclude adding insert `RETURNING`.

## 6. Idempotency and replay semantics

`verify_recorded_entries` constructs a hash map of returned rows keyed by entry ID and sorts incoming entry references by ID. It first collects **all missing IDs** in sorted order. If any are missing, it returns `Missing` before evaluating content conflicts. Otherwise it returns the first content mismatch in sorted order—the lowest conflicting entry ID. [Source: `assembly/infra_diesel.rs:455–499`][verification]

### Comparison matrix

| Field | Replay treatment |
|---|---|
| Entry `id` | Identity used for database conflict detection and result lookup |
| Payload | Equality of parsed `serde_json::Value`, not equality of text bytes |
| `message_id` | Must match |
| `routing_key` | Must match exactly |
| `event_type` | Must match exactly |
| `content_type` | Must match as an `Option<String>` |
| `sequence` | Must match as an `Option<u64>` |
| `meta.event_id` | Not explicitly compared; envelope conversion normally makes it equal to row ID |
| `correlation_id`, `causation_id`, `source` | Not compared; changed replay values are ignored |
| `scheduled_at`, `received_at`, `updated_at` | Not compared or overwritten |
| Status, attempts, reservation/completion fields, errors | Not compared or overwritten |

For example, `{"a":1,"b":2}` and `{ "b":2, "a":1 }` compare equal. This means equality according to `serde_json::Value`, not an independently defined JSON canonicalization standard. The original stored representation remains unchanged. Tests explicitly cover reordered object keys, formatting differences, allowed source/correlation changes, and a conflicting sequence. [Source: `assembly/infra_diesel.rs:538–557`][verification-tests]

Three cases must not be conflated:

1. **Two equal IDs in one input batch:** rejected before persistence, even if identical.
2. **A later request repeats an already stored row with matching content:** succeeds without rewriting it.
3. **A later request repeats an ID with conflicting compared content:** returns `Conflict`; new rows inserted by that same operation roll back.

The private verification helper has a test accepting identical duplicate inputs. That is **not** the public recording contract: public recording validates duplicates before this helper is reachable. [Sources: `record_events.rs:200–220,285–292`][recording], [`assembly/infra_diesel.rs:560–565`][verification-tests]

Existing rows with arbitrary/non-object payload text remain in place. The current recorder will not accept equivalent non-object input to replay them. No schema migration or data rewrite implements the stricter rule; compatibility is enforced at the application boundary. [Source: `outbox-recording-parity-plan.md:15–19`][parity]

## 7. Transactions and error behavior

### Atomicity is scoped to the connection

| Path | Transaction boundary |
|---|---|
| Recorder backed by `OutboxStoreStorage` | Checks out a connection and opens its own recording transaction |
| `record_bulk_on_connection` with no active transaction | Opens and completes a transaction on the supplied connection |
| `record_bulk_on_connection` inside a caller transaction | Uses Diesel's nested transaction/savepoint behavior; does not independently commit the outer transaction |
| Recorder backed by a custom `OutboxStorePort` | Depends on that adapter's implementation; the wrapper supplies validation, not rollback |

The connection helper is the branch's route to a transactional outbox coupled to business data. Sharing a `DbPool` alone is insufficient: an unrelated business operation can be using a different checked-out connection. [Sources: `record_events.rs:32–39,317–329`][recording], [`assembly/infra_diesel.rs:398–452`][storage], [`tests/bulk_recording.rs:67–86`][bulk-tests]

Illustrative composition using this branch's actual public API:

```rust
use diesel::{Connection, PgConnection};
use mulac_outbox::io::{NewOutboxEnvelope, OutboxError, record_bulk_on_connection};

fn persist_with_outbox(
    conn: &mut PgConnection,
    envelopes: &[NewOutboxEnvelope],
    write_business: impl FnOnce(&mut PgConnection) -> Result<(), OutboxError>,
) -> Result<(), OutboxError> {
    conn.transaction::<(), OutboxError, _>(|conn| {
        write_business(conn)?;
        record_bulk_on_connection(conn, envelopes)
    })
}
```

An application will normally adapt this pattern to its own error type. Propagating the helper's failure aborts the outer closure; deliberately handling it can allow other work in the outer transaction to continue. A successful nested call is not durable independently of the caller's eventual commit. Transaction guarantees do not imply that a client can always resolve an uncertain outcome after a connection failure during commit; replay remains important.

### Error taxonomy

| Error | Origin / significance |
|---|---|
| `Duplicate { id }` | Repeated ID detected within the input batch |
| `InvalidPayload { id }` | Payload does not parse as a JSON object |
| `Routing(String)` | Blank route or invalid sequence range |
| `Conflict { id }` | Existing row differs in compared message content |
| `Missing { ids }` | Read-back lacks requested IDs; IDs are sorted |
| `Database(diesel::result::Error)` | Recording SQL/transaction error, preserving the underlying Diesel error through `#[from]` |
| `Storage(String)` | Pool acquisition failure in this write path |

These are not per-row success reports. The API returns one error, except that `Missing` carries the complete missing-ID list found during verification. No automatic retry policy is implemented in the recording loop.

The typed database error should not be generalized to every outbox operation: consumer reservation still maps database errors into `Storage(String)`. [Sources: `assembly/application.rs:31–68`][ports], [`assembly/infra_diesel.rs:398–452`][storage], [`assembly/infra_diesel.rs:612–657`][reservation]

## 8. Consumer and application integration

### Batch recording is not batch publication

`OutboxConsumer::publish_batch` reserves a batch, then iterates over entries and publishes each separately. Each successful publish is followed by `completed`; publication failure leads to `failed`; conversion failure leads to `dead`. Errors are collected while subsequent entries continue. A batch-level error does not undo an earlier external publication. [Source: `outbox_consumer.rs:185–241`][consumer]

The PostgreSQL consumer uses a reservation CTE with `FOR UPDATE SKIP LOCKED`, updates reservation metadata and attempts, and returns rows. That is a different algorithm from bulk recording. Its concurrency test does not establish that overlapping bulk **writers** are safe. [Sources: `assembly/infra_diesel.rs:601–663`][reservation], [`tests/diesel_storage.rs:152–199`][diesel-tests]

Stable recording identity does not create exactly-once broker delivery. A publish can succeed while completion persistence fails, so the row may be published again. The lifecycle test `duplicate_publish_after_completion_failure_keeps_message_id_stable` exercises this distinction without a real database. [Source: `tests/lifecycle.rs:238–342`][lifecycle-tests]

### Current callers do not demonstrate automatic bulk adoption

The kernel's `outbox_subscriber` builder method is documented as unsupported, and startup validation returns `OutboxSubscriberUnsupported` for such registration. Applications must provide their own subscriber and routing policy. [Source: `kernel/src/lib.rs:301–308,475–488`][kernel]

The Todo and Twitter example subscribers persist to their own **`outbox_messages`** table, not the core crate's `outbox_entries` table:

- Todo generates a new UUID and directly inserts an application row.
- Twitter extracts the event ID and directly inserts with `ON CONFLICT (id) DO NOTHING`.

Neither call chain demonstrates the new core bulk recorder or its content-verification policy. These application paths must not be used as evidence that business writes already participate in the new connection helper's transaction. [Sources: `todo/.../infra_diesel.rs:49–88`][todo], [`twitter/.../infra_diesel.rs:150–190`][twitter]

The separate `outbox_api::OutboxService` wraps the consumer and runs publication batches. It is not an HTTP bulk-recording endpoint. [Source: `outbox_api/src/service.rs:18–53`][outbox-api]

## 9. Performance and concurrency review

### Work per nonempty batch

For `n` valid entries, the PostgreSQL writer issues approximately `ceil(n / 5000)` insert statements, plus transaction-control statements. When all rows are new, no verification select is issued. When at least one row is skipped, one read-back query retrieves the full requested set.

This reduces insert statement count relative to calling the single-row API `n` times, but it does not establish a measured speedup. The recorder processes an in-memory slice and returns synchronously; it is not streaming ingestion or a background batching queue.

### Repeated validation and allocation

On the normal pooled recorder path, `validate_entries` runs in three places: the repository, the pooled adapter before checkout, and the shared connection writer. Each invocation allocates and sorts a vector of references and parses every payload. The recorder also makes its envelope-level route/sequence pass. The writer then independently sorts the insertable records; replay verification sorts entry references again and parses both sides' payloads.

The redundancy protects several independently callable boundaries, but it has a real computational cost: repeated `O(n log n)` sorting and repeated work proportional to total payload size. The source alone does not tell us whether that cost dominates SQL or network latency. [Sources: `record_events.rs:34–78,285–313`][recording], [`assembly/infra_diesel.rs:394–418,467–490`][storage]

Chunking bounds statement size, **not total batch memory**. Before insertion, the caller's envelopes, converted entries, and cloned insertion records can coexist. A replay adds a vector of all returned rows, an ID vector, a lookup map, and temporary parsed JSON values. There is no overall input-count or payload-byte limit in these methods.

Read-back selects all row fields even though comparison needs only ID, payload, and selected metadata. This increases transfer/deserialization work and couples replay to decoding fields such as stored error metadata. A narrower projection is a possible improvement after correctness coverage, not a change made by this report.

### Lock ordering and isolation

Sorting before chunking intentionally makes overlapping writers attempt unique keys in the same order. The read-back then requests shared row locks. Those locks can interact with lifecycle updates and are retained according to transaction scope; an enclosing business transaction can extend the overall lock lifetime.

The implementation does not specify a transaction isolation level, configure lock timeouts, or retry deadlocks/serialization failures. Consistent insert ordering reduces one source of lock-order inversion, but does not prove deadlock freedom across read-back, other tables, triggers, or caller transactions. There is no reverse-order concurrent bulk-writer test on this branch. Treat concurrency safety under realistic contention as an open verification item, not as an observed failure.

## 10. Tests and observed results

The following commands were rerun successfully for this report on 2026-11-06, using the offline dependency cache:

```bash
cargo test --offline --manifest-path libs/Cargo.toml -p mulac-outbox --no-default-features
cargo test --offline --manifest-path libs/Cargo.toml -p mulac-outbox
cargo check --offline --manifest-path libs/Cargo.toml -p mulac-outbox --all-targets --all-features
```

| Check | Observed result |
|---|---|
| No default features | 27 unit tests + 1 facade test passed; database-gated targets contained no tests |
| Default features (`diesel`) | 33 unit tests + 2 facade tests + 1 database-free lifecycle test passed |
| Default-feature PostgreSQL suites | 10 ignored: 2 bulk-recording, 4 storage, 4 lifecycle |
| All targets, all features | Compilation/check passed; this was not an all-feature test run |

`OUTBOX_TEST_DATABASE_URL` was not configured. No PostgreSQL tests were enabled, and no tables were created or reset during this investigation. AMQP broker behavior and performance were not tested.

### Coverage map

| Concern | Existing evidence | Limitation |
|---|---|---|
| Nonempty batch and empty batch | `bulk_records_all_entries_and_accepts_empty_batch` | Asserts stored entries, not exact adapter call count or pool checkout avoidance |
| All-envelope validation | `bulk_validates_all_envelopes_before_storage` | Exercises a later invalid route through a fake store |
| Duplicate IDs and object-only JSON | `rejects_duplicates_and_non_object_payloads_before_storage` | Covers several invalid payload forms, not every mixed-error precedence |
| Identity conversion and single writes | Recorder conversion/single-write tests | Fake-store evidence, not persisted row-state verification |
| Semantic replay and changed context | `semantic_replay_and_missing_ids` | Pure verification helper; no PostgreSQL row locking exercised |
| Missing rows and mismatch | Verification helper tests | No database trigger/suppressed-insert scenario |
| More than two chunks | `chunks_replay_and_cross_chunk_rollback` | Present but ignored in this run |
| Nested savepoint and outer rollback | `caller_rollback_and_savepoint_isolation` | Present but ignored; operates on outbox rows, not a separate business table |
| Stable message ID after completion failure | Database-free lifecycle test | Demonstrates at-least-once behavior, not database-backed transactional atomicity |

[Sources: `record_events.rs:171–279`][recorder-tests], [`assembly/infra_diesel.rs:502–585`][verification-tests], [`tests/bulk_recording.rs:48–86`][bulk-tests], [`tests/lifecycle.rs:238–342`][lifecycle-tests]

### What the two database bulk tests actually assert

`chunks_replay_and_cross_chunk_rollback` inserts 10,001 entries supplied in reverse order, asserts the stored count, and repeats the request. It then supplies 10,001 new IDs plus one previously stored ID with changed payload, expects `Conflict`, and asserts the final count remains 10,001. Since conflict verification follows all insert chunks, this is intended to check rollback of already executed chunks. It does not simulate a failure during a later insert statement or concurrent writers.

`caller_rollback_and_savepoint_isolation` starts an outer transaction, records one entry, catches a conflicting nested recording request containing another entry, and checks that only the original entry remains. It then explicitly rolls back the outer transaction and checks the table is empty. It is a useful savepoint test, but does not prove preservation/rollback of writes to an independent business table or successful outer commit.

Both tests use a connection-local temporary `outbox_entries` table. By contrast, the older storage/lifecycle database suites drop and recreate a shared table, so those suites require a disposable database and serial execution. [Sources: `tests/bulk_recording.rs:6–14,48–86`][bulk-tests], [`tests/diesel_storage.rs:72–88`][diesel-tests], [`tests/lifecycle.rs:62–78`][lifecycle-tests]

To exercise the database tests later, point `OUTBOX_TEST_DATABASE_URL` at a dedicated disposable database and run:

```bash
cargo test --manifest-path libs/Cargo.toml -p mulac-outbox \
  --test bulk_recording -- --ignored --test-threads=1
cargo test --manifest-path libs/Cargo.toml -p mulac-outbox \
  --test diesel_storage -- --ignored --test-threads=1
cargo test --manifest-path libs/Cargo.toml -p mulac-outbox \
  --test lifecycle -- --ignored --test-threads=1
```

The main CI workflow invokes `make test-libs`; that target runs ordinary `cargo test --all-features`, without `--ignored`. Therefore its success alone does not establish that these PostgreSQL tests ran. [Sources: `.github/workflows/ci.yml:29–32`][ci], [`Makefile:41–43`][makefile]

## 11. Prioritized recommendations

These are proposed follow-ups, **not changes made in this investigation**.

### Priority 1 — establish the persistence guarantees experimentally

1. **Run the existing ignored database tests in a dedicated CI job.** Record PostgreSQL version and isolation configuration. Preserve the distinction between database verification and database-free unit tests.
2. **Add overlapping concurrent bulk writers with reversed input order.** Include all-new rows, identical replay, and conflicting replay. Assert bounded completion and final persisted content; collect actual database errors rather than inferring deadlock freedom from sorting.
3. **Add business-table transaction composition tests.** Cover outer commit, propagated outbox failure, caught nested failure preserving earlier business writes, and outer rollback removing both business and outbox writes.
4. **Add replay tests against delivered/reserved/failed rows.** Assert unchanged payload text, timestamps, status, attempts, reservation fields, and error history. Test every compared metadata field independently and verify that allowed context changes leave old context intact.

### Priority 2 — close boundary and contract gaps

5. **Test chunk boundaries and late failures:** 4,999, 5,000, 5,001, and 10,001 entries; a later-chunk database error; and missing IDs produced by a deliberately suppressed insertion in an isolated test schema.
6. **Test typed database errors and sources.** Verify that write/transaction errors retain their Diesel variant and source, while pool acquisition uses the documented storage-error path.
7. **Strengthen adapter contract tests.** Assert zero storage calls for empty/invalid batches, one bulk call for a valid batch, and the single-to-bulk delegation of the PostgreSQL adapter. Document what custom adapters must guarantee.
8. **Resolve the lower-level identity contract.** Explicitly document or validate `NewOutboxEntry.id == meta.event_id` for direct-store clients. Keep event-ID identity unless a separately approved migration changes it.
9. **Update the general outbox specification.** Its recording section describes single-row insertion and idempotent conflict acceptance without the full content-comparison policy. Link or incorporate the newer parity decisions; explain the caller-owned transaction exception to any blanket “success means durable” wording. [Source: `outbox-spec.md:44–80`][spec]

### Priority 3 — measure before optimizing

10. **Benchmark all-new, all-replay, and mixed batches** at realistic row counts and payload sizes. Measure statement count, elapsed time, peak memory, validation time, and lock waits. Do not extrapolate throughput from unit tests.
11. **Consider eliminating redundant validation passes** only while preserving protection for independently callable entry points. A private validated representation is one possible design, not a requirement of this report.
12. **Consider narrower replay projections and explicit input limits** after documenting compatibility. Retain the branch's chosen fast path, immutable replay behavior, transaction scope, and no-insert-`RETURNING` design unless requirements are deliberately revised.

### Overall assessment

The branch provides a substantive, internally shared bulk-recording implementation with deliberate replay and transactional-composition semantics. Its repository wrappers are thin and testable; the actual persistence guarantees correctly reside in the PostgreSQL adapter. The main unresolved issue is **confidence and integration coverage**, not evidence of a demonstrated atomicity failure. Before deployment, verify the database scenarios, establish the contract for lower-level/custom storage callers, and measure the cost of repeated validation and whole-batch read-back.

## Source index

Line references refer to commit `74513331bd725fddee23c5854a9ceeb607b3e4fd`. Relative links resolve within the repository; later edits may shift line numbers.

- [Public exports][facade] and [crate features][manifest].
- [Recorder/repository implementation][recording], [entry validation][validation], and [caller-owned connection helper][connection-helper].
- [Application ports and errors][ports], [domain data models][domain], and [SQL schema][schema].
- [PostgreSQL writer][storage], [insertable record shape][record-shape], [replay verifier][verification], and [consumer reservation][reservation].
- [Consumer repository and publication loop][consumer].
- [Recorder tests][recorder-tests], [verification tests][verification-tests], [bulk database tests][bulk-tests], [storage tests][diesel-tests], and [lifecycle tests][lifecycle-tests].
- [Recording-parity decisions][parity] and [general outbox specification][spec].

[recording]: ../libs/outbox/src/record_events.rs#L1-L329
[validation]: ../libs/outbox/src/record_events.rs#L282-L314
[connection-helper]: ../libs/outbox/src/record_events.rs#L317-L329
[ports]: ../libs/outbox/src/assembly/application.rs#L24-L169
[domain]: ../libs/outbox/src/assembly/domain.rs#L97-L139
[storage]: ../libs/outbox/src/assembly/infra_diesel.rs#L379-L500
[record-shape]: ../libs/outbox/src/assembly/infra_diesel.rs#L108-L215
[verification]: ../libs/outbox/src/assembly/infra_diesel.rs#L455-L499
[verification-tests]: ../libs/outbox/src/assembly/infra_diesel.rs#L502-L585
[reservation]: ../libs/outbox/src/assembly/infra_diesel.rs#L601-L663
[consumer]: ../libs/outbox/src/outbox_consumer.rs#L6-L241
[facade]: ../libs/outbox/src/lib.rs#L1-L50
[manifest]: ../libs/outbox/Cargo.toml#L1-L20
[recorder-tests]: ../libs/outbox/src/record_events.rs#L171-L279
[bulk-tests]: ../libs/outbox/tests/bulk_recording.rs#L1-L86
[diesel-tests]: ../libs/outbox/tests/diesel_storage.rs
[lifecycle-tests]: ../libs/outbox/tests/lifecycle.rs
[schema]: outbox_entries.sql
[parity]: outbox-recording-parity-plan.md
[spec]: outbox-spec.md
[kernel]: ../kernel/src/lib.rs#L301-L488
[todo]: ../test_apps/todo/src/assembly/infra_diesel.rs#L49-L88
[twitter]: ../test_apps/twitter/src/assembly/infra_diesel.rs#L150-L190
[outbox-api]: ../libs/outbox_api/src/service.rs#L18-L53
[ci]: ../.github/workflows/ci.yml#L29-L32
[makefile]: ../Makefile#L41-L43
