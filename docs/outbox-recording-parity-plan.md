# Outbox recording parity

## Implemented decisions

- Keep event ID as row identity; message ID remains delivery metadata.
- Reject duplicate IDs before storage, including identical duplicates.
- Require JSON objects, preserve original text, and compare parsed JSON on replay.
- Compare message ID, routing key, event type, content type, and sequence. Allow changed correlation ID, causation ID, and source; preserve the stored row.
- Return typed InvalidPayload, Duplicate, Conflict, and Missing errors. Report every missing ID in sorted order and the lowest conflict.
- Preserve underlying Diesel errors in Database.
- Expose Diesel-gated io::record_bulk_on_connection for caller-owned connections, using transactions or nested savepoints without committing the outer transaction.
- Preserve empty-batch early return, sorted IDs, 5,000-row chunks, and the inserted-count fast path.
- Keep bulk storage mandatory and repository insertion private to its parent module.

## Compatibility

Recording now rejects arbitrary strings, non-object JSON, and duplicate IDs within a batch. Existing non-JSON rows remain publishable but cannot be replayed through the stricter recorder. New error variants require downstream exhaustive matches to be updated. No schema migration or existing-row rewrite is needed.

Content type remains optional and must match on replay. Changed event IDs produce separate rows even with the same message ID. Call record_bulk_on_connection with the same connection used by business writes inside a Diesel transaction for atomic persistence.

## Verification

Added unit coverage for JSON validation, duplicates, semantic replay, context changes, and missing IDs. Added PostgreSQL tests for 10,001-row insertion and replay, cross-chunk rollback, caller rollback, and savepoint isolation. Database tests require a dedicated OUTBOX_TEST_DATABASE_URL.

Remaining acceptance coverage: reverse-order concurrent batches, exactly 5,001 rows, suppressed-insert missing IDs, preserved business writes and delivery state, and database error sources. Run default, no-default-feature, and all-feature checks. Existing database suites resetting shared tables must run serially.

Do not introduce RETURNING, remove the fast path, modify retailer sourcing, or migrate identity to message IDs.
