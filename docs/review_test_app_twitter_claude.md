# Review: test_app_twitter

Reviewer: Miro
Date: 2026-05-24
Scope: `test_apps/twitter/` — source modules, assembly, and integration test suite

---

## Summary

The twitter test app is a well-structured integration harness that exercises the mulac messaging infrastructure (commanding, eventing, inbox, outbox) through a realistic Twitter-like domain. All 11 integration test binaries were read. The implementation is largely correct and the tests cover the spec. The findings below range from structural observations to concrete improvement suggestions.

---

## What Works Well

**Module structure follows the CLAUDE.md convention.** Every feature module (e.g. `tweet_post`, `tweet_like`, `user_follow`) exposes exactly one `pub mod io` and keeps `models`, `handler`, `http`, and `infra_diesel` private. The top-level `lib.rs::io` re-exports a clean flat surface for test consumers.

**Assembly is separated correctly.** `assembly/application.rs` owns all mulac wiring (command registry, event registry, consumer loops, gateway construction). Feature modules are not aware of the registry — they only implement `CommandHandlerPort`. This matches the layered architecture the kernel specifies.

**Two-round dispatch loop is intentional and documented.** `MulacState::dispatch_command` runs two command+event consume rounds. The inline comment explains the second round picks up secondary commands (e.g. `FanOutTweet` queued by `timeline_fan_out` when `TweetPosted` is processed). This is a deliberate tradeoff to keep HTTP handlers synchronous.

**Test isolation is handled carefully.** `utils::start_test_app` acquires an async mutex before each test (serialising concurrent tests), cancels the previous worker token, flushes background workers with a 50 ms sleep, runs migrations, and truncates all tables. This is a correct approach for integration tests sharing a single database.

**Idempotency is tested.** The `fan_out_is_idempotent` test re-submits a `FanOutTweet` command for an already-fanned-out tweet and asserts only one timeline row per follower. The `outbox_no_duplicate_row_per_event_id` test asserts the outbox does not write a duplicate row for the same event.

**Unicode-aware content validation.** `validate_content` uses `.chars().count()` (Unicode scalar count) rather than `.len()` (byte count). The `post_tweet_unicode_content_within_limit_succeeds` test verifies that 280 `é` characters (560 UTF-8 bytes) pass through correctly.

---

## Findings

### 1. Shared pool + table-level TRUNCATE couples tests to a single schema

`shared_pool` uses a `OnceLock` to return the same pool across all test binaries run in the same process. Each `start_test_app` call truncates all tables. This means any test that runs a slow background operation after the table clear can interfere with the next test in the same binary.

The current 50 ms sleep after cancelling the previous worker token is a timing assumption, not a guarantee. If a blocking `consume` call takes longer than 50 ms (e.g. under load), the next test's truncation can wipe rows the previous worker is still writing.

**Suggestion:** after cancelling the token, `join` the spawned task handles rather than sleeping. `tokio::spawn` returns a `JoinHandle`; storing these handles in the shared state would allow an explicit `await` in `start_test_app` before truncating. If that is architecturally inconvenient, at minimum increase the sleep to 200 ms and document the risk.

### 2. `tweet_like` duplicate like treated as 200 OK — no idempotency contract stated in spec

`like_duplicate_is_noop_200` asserts that liking a tweet a second time returns 200 and creates only one `likes` row. The `unlike_absent_like_returns_204_no_event` test asserts that unliking a non-existent like returns 204 with no event.

These are reasonable product decisions, but they differ from how `post_tweet` and `send_direct_message` treat duplicates (409 Conflict). The spec (`docs/test_app_twitter.md`) does not state which commands are idempotent and which are conflict-producing. This inconsistency can confuse consumers of the inbox API.

**Suggestion:** add a short section to `docs/test_app_twitter.md` that explicitly lists which commands are idempotent-on-duplicate and which return 409.

### 3. `interpret_dispatch_error` uses string-prefix matching on handler error messages

`interpret_dispatch_error` in `assembly/application.rs` maps `KernelError` variants to HTTP responses by matching substrings of the error message string (e.g. `message.starts_with("tweet not found")`). This creates an implicit contract between the error message text produced by handlers and the HTTP mapping layer.

If a handler changes its error message wording, the HTTP status code silently changes. There are currently nine feature modules, each with its own error strings; some of these strings are not tested.

**Suggestion:** introduce a structured error type in the handler layer (e.g. a `DomainError` enum with `NotFound`, `Conflict`, `Validation` variants) and return that from handlers instead of free-form strings inside `CommandError::HandlerExecution`. Map the structured type to HTTP in `interpret_dispatch_error`. This removes the fragile string coupling.

### 4. Background worker loop polls on a 1-second interval

`run_command_worker` and `run_event_worker` sleep for one second between consume calls. For an integration test app exercising the mulac infrastructure, this 1-second latency is noticeable: the `tweet_posted_fans_out_to_followers` test compensates with a `tokio::time::sleep(200ms)` inside the test, which only works because the test's direct call path uses `MulacState::dispatch_command` (synchronous) rather than the background workers.

However, if a test ever needs the background workers to pick up a message (e.g. a stale-command sweep scenario), it would have to sleep up to 1 second. This is not a bug in the current test suite but is a latent friction point.

**Suggestion:** expose a configurable poll interval in `run_command_worker` / `run_event_worker` (default 1 second for production, 10–50 ms for test mode). Alternatively, add a `notify` channel so the gateway can wake the worker immediately after dispatching.

### 5. `delete_tweet` sends the `author_id` in the request body, not in the URL

The HTTP handler for tweet deletion uses `DELETE /api/tweets/{id}` with a JSON body containing `author_id`. HTTP DELETE with a body is technically allowed by RFC 9110 but is non-standard; many proxies and clients discard bodies on DELETE requests.

**Suggestion:** either pass `author_id` as a query parameter (`DELETE /api/tweets/{id}?author_id=...`) or introduce an authorization context header. For this test app the body approach is acceptable as a simplification, but a comment noting the tradeoff would prevent confusion.

### 6. `retweeted_from` is present in `TweetRow` in utils but not verified in retweet tests

`utils::TweetRow` has a `retweeted_from: Option<Uuid>` field. The `retweet_success` test fetches the outbox and verifies the `TweetRetweeted` event is pending, but does not assert that `tweets.retweeted_from` is set to the original tweet's ID.

**Suggestion:** add one assertion to `tweet_retweet.rs`:

```rust
let row = fetch_tweet_by_id(&pool, retweet_id).unwrap();
assert_eq!(row.retweeted_from, Some(original_id));
```

### 7. `OutboxSubscriber` is registered once per event type but there is no name uniqueness enforcement

In `start_mulac`, `OutboxSubscriber::new(pool.clone())` is registered with a unique subscriber name per event type (e.g. `"tweet-posted-outbox"`). If a developer registers the same subscriber name twice for the same event, the second registration silently overwrites the first (since `EventSubscriberRegistry` uses a `Vec`, not a `HashMap`, for name-keyed deduplication). There is no compile-time or runtime guard.

**Suggestion:** in `EventSubscriberRegistry::from_subscribers`, assert that no two entries share the same `(event_type, name)` pair, or return an error from `start_mulac` if a collision is detected.

### 8. No test covers the `FanOutTweet` command when the author has zero followers

The `timeline_fan_out` tests cover the happy path (two followers) and idempotency. The zero-follower case (a new user posts a tweet, no one follows them) is the most common production path and is untested. If `FanOutTweet` panics or errors on an empty follower set, no test would catch it.

**Suggestion:** add a test:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn fan_out_with_no_followers_is_noop() {
    let (base_url, pool, _guard) = start_test_app().await;
    let author = Uuid::now_v7();
    post_tweet(&base_url, author, "no followers").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let timelines = fetch_timelines(&pool);
    assert!(timelines.is_empty());
    assert_command_completed(&pool, "FanOutTweet");
}
```

---

## Minor Observations

- `infra_diesel {}` (empty module) appears in `tweet_post.rs`, `tweet_like.rs`, and several other modules. This is a placeholder that aids discoverability of the module layout, which is fine. It would be worth adding a `// reserved for future queries` comment so readers know it is intentional.

- `utils::retry_fan_out` creates a brand-new `MulacHandle` via `start_mulac` and immediately calls `command_consumer().consume(...)` without starting background workers. This is fine for its purpose (synchronous re-dispatch in tests), but the function name `retry_fan_out` is somewhat domain-specific. If it were renamed `dispatch_command_sync` with a `command_type` parameter, it would be reusable for other synchronous replay scenarios.

- `assert_command_completed` and `assert_event_completed` compare `status` as `i32` against the constant `STATUS_COMPLETED = 5`. The value `5` is not self-documenting; its meaning depends on the kernel's internal status enum. A comment cross-referencing the kernel's `CommandStatus::Completed` would help readers.

- `utils::fetch_command_entries` orders by `received_at` — a column not declared in `CommandEntryRow`. If the column name changes in the kernel schema, this query silently fails or panics at runtime. Selecting only the columns declared in the struct would be safer.

---

## Conclusion

The implementation is solid and the test suite covers the most important flows including inbox, outbox, fan-out, idempotency, and error cases. The most actionable findings are findings 1 (test isolation timing), 3 (string-based error mapping), and 8 (missing zero-follower test). The rest are improvements of varying priority.
