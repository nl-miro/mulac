# test_app_twitter

A Twitter-like reference application that exercises the Mulac command/event framework with Diesel persistence.

## Overview

`test_app_twitter` mirrors `test_app_todo` in architecture but uses Diesel instead of SQLx for all database operations. It demonstrates:

- Flat feature modules, each with exactly one `pub mod io` facade
- App-owned `AppState { pool, mulac }` with two-phased command/event gateways
- App-owned `inbox_messages` and `outbox_messages` tables separate from kernel `*_entries` tables
- Idempotent outbox with PK reuse (`outbox_messages.id == source event_id`)
- Timeline fan-out via an event subscriber that dispatches internal commands; each follow-up command chains its metadata from the `TweetPosted` event (`causation_id` = `event_id`, `correlation_id` = the event's correlation), per [contracts.md — Metadata propagation](contracts.md#metadata-propagation)

## Source layout

```
test_app_twitter/
  Cargo.toml
  Makefile
  AGENTS.md                          — detailed agent/developer reference
  docker-compose.yml
  migrations/
    2025-01-01-000001_infrastructure/ — kernel tables
    2025-01-01-000002_app_tables/     — domain tables
    2026-05-23-000001_app_messages/   — app inbox/outbox tables
  src/
    lib.rs                            — TwitterEvent, AppState, pub mod io
    schema.rs                         — Diesel schema (auto-generated)
    assembly/
      application.rs                  — errors, AppCommand, MulacState, start_mulac
      domain.rs                       — Clock, DTOs, InboundEntity
      infra_diesel.rs                 — DbPool, migrations, OutboxSubscriber
      bin/twitterapp.rs               — serve / migrate binary
    tweet_post.rs, tweet_delete.rs, tweet_retweet.rs
    user_follow.rs, user_unfollow.rs
    tweet_like.rs, tweet_unlike.rs
    direct_message_send.rs
    timeline_fan_out.rs
    inbox.rs, outbox.rs
  tests/
    utils.rs                          — reqwest harness, row structs, helpers
    tweet_post.rs, ..., outbox.rs     — one file per feature + inbox + outbox
```

## Running locally

```bash
make up        # start Postgres on port 5433
make migrate   # apply migrations
make serve     # start server on 127.0.0.1:33002
make test      # run integration tests
make reset     # wipe and re-migrate
```

Environment variables:
- `DATABASE_URL` — default: `postgres://twitter:twitter@localhost:5433/twitter`
- `BIND_ADDR` — default: `127.0.0.1:33002`

## REST API

| Method | Path                    | Response                  |
|--------|-------------------------|---------------------------|
| GET    | /health                 | `ok`                      |
| GET    | /swagger                | Swagger UI                |
| POST   | /api/tweets             | `TweetDto`                |
| DELETE | /api/tweets/:id         | `204 No Content`          |
| POST   | /api/tweets/:id/retweet | `TweetDto`                |
| POST   | /api/users/follow       | `FollowDto`               |
| POST   | /api/users/unfollow     | `FollowDto` (synthesized) |
| POST   | /api/tweets/:id/like    | `LikeDto`                 |
| DELETE | /api/tweets/:id/like    | `204 No Content`          |
| POST   | /api/messages/direct    | `DirectMessageDto`        |
| POST   | /api/messages/commands  | `InboundResponse`         |
| GET    | /api/messages/outbox    | `OutboxList`              |

Errors: `{ "error": "..." }` with HTTP 400 / 404 / 409 / 500.

## Temporal semantics

**D-10: decided 2026-10-22; deleted-target unlike enforcement and full scenario verification remain pending.** These rules define stored relationships and command outcomes, not display/filtering behavior for the deferred timeline read API. They introduce no schema/API changes, recipient snapshots, extra fan-out triggers, queue ordering guarantee, or stronger concurrency guarantee.

### Follower membership and fan-out

- Each `FanOutTweet` attempt selects recipients from the follower relationships visible when that attempt reads them, not from membership at tweet post-time or at a previous attempt.
- A follow/unfollow between posting and fan-out affects that attempt's recipient set. A retry reads membership again and may add newly joined followers.
- Existing timeline rows remain after an unfollow. A follower who left before a retry and has no timeline row from an earlier attempt need not receive one on that retry; the old recipient set is not retained for recovery.
- There is at most one timeline row per `(user_id, tweet_id)`. A successful attempt leaves a row for every recipient it selected without duplicating existing rows; partial attempts may leave rows that later attempts preserve.
- Delayed fan-out still runs after the tweet is soft-deleted, using current followers and creating historical timeline references. Deletion does not cancel previously queued fan-out.
- Fan-out itself emits no domain event or outbox effect. Existing `TweetPosted`-driven triggering remains unchanged; these rules do not add fan-out for retweets or relationship changes.

### Deletion and dependent records

- A successful delete soft-deletes only the selected tweet. Existing likes, timeline rows, and retweets referencing it remain unchanged. No cascading deletion or derived deletion events occur; the selected tweet still has its normal `TweetDeleted` event.
- Like, retweet, and unlike targeting a missing or soft-deleted tweet return 404 with no domain mutation, domain event, or outbox effect, regardless of whether a like already exists.
- For an active tweet, unlike removes an existing like and emits `TweetUnliked`; an absent like remains a successful no-op with no event/outbox effect. Existing endpoint response conventions are unchanged.
- Retaining dependent rows does not promise that a future read API will display deleted content. Visibility and filtering remain deferred with that API.

### Retweet chains

- Retweeting an active retweet is allowed. The new row's `retweeted_from` and its event's `original_tweet_id` identify the immediate target, not a resolved root tweet.
- Eligibility checks the immediate target only. A deleted original or intermediate ancestor does not invalidate an otherwise active descendant: that descendant may still be liked or retweeted, and its likes may be removed under the active-target rules.
- Deleting an intermediate retweet leaves its descendants intact. Operations targeting the deleted intermediate row itself still return 404.
- Duplicate `retweet_id` behavior remains unchanged; this decision does not introduce ancestry normalization or cascading cleanup.

### Implementation and verification status

Current source reads followers on each attempt, preserves existing timeline rows, fans out without checking tweet deletion, soft-deletes only the selected tweet, and permits retweet chains with immediate-target checks. These observations are not proof of complete temporal coverage.

**Pending behavior change:** `tweet_unlike.rs` currently checks only whether the tweet row exists, so it can remove a like from a soft-deleted tweet. The decided contract requires 404 with no mutation or event for that case, including when the like is absent. Enforcement and regression coverage remain future work; no runtime change or new passing test is claimed here.

Application handlers retain ownership of business-effect idempotency and recovery of required events across command retries under [D-05](contracts.md#command-dispatcher). Resolving temporal semantics does not establish compliance with those recovery obligations.

## Inbox and outbox

**`inbox_messages`** — records every command envelope received at `POST /api/messages/commands`.
- `received`: recorded; the outcome is not known yet.
- `processed`: the command was applied.
- `failed`: the command was **rejected** (validation 400, not-found 404, conflict 409). Terminal; never re-dispatched.
- An infrastructure error (storage, inline drain, worker) does **not** mark the row `failed`. The row stays `received` with `error` set to the latest failure, and the request returns 5xx. The command may already be durably recorded and may still complete.
- Redelivering a message ID that already exists returns `409 Conflict` whatever its status. Redelivery is not a recovery path.
- **Background recovery:** an app-level job runs on a configurable interval (default: every minute). It re-dispatches `received` rows whose `received_at` is older than a configurable threshold (default: 5 minutes, so in-flight requests are left alone). It re-dispatches with the original message ID as the command ID, and applies the same status rules to the result.
  - Rows are claimed with `FOR UPDATE SKIP LOCKED`, so concurrent recovery runs never re-dispatch the same row at once.
  - There is no attempt cap: a row stays `received` until it resolves, and `error` makes long-stuck rows visible to monitoring.
  - Re-dispatch is safe because the kernel records commands idempotently by command ID; a message produces at most one command entry.

**`outbox_messages`** — app-level journal of published events.
- `id` equals the originating kernel `event_id`, inserted with `ON CONFLICT (id) DO NOTHING`. A replayed event (the kernel delivers events at least once) adds no row.
- An event without an `event_id` fails the subscriber, so the kernel retries the event entry instead of writing an unkeyed row.
- `status` stays `pending`: nothing publishes from the app outbox table; it is a test record of emitted events (AMQP publishing is not implemented; see deferred items).
- Payload shape: `{"type": "TweetPosted", "payload": { ... }}`

**Do not confuse with kernel tables:**
- `command_entries` / `event_entries` — kernel write-side journal
- `inbox_entries` / `outbox_entries` — kernel transport tables (not used for app-level messaging)

## Persistence

Diesel 2.x with PostgreSQL. All handlers and infra functions are synchronous — Diesel does not support async. All Diesel calls in async poem handlers are wrapped in `tokio::task::spawn_blocking` via the `run_blocking` helper in `assembly/application.rs`.

## Deferred features

- **Timeline read API** (`GET /api/timeline/:user_id`) — not yet implemented.
- **Mention notifications** — not yet implemented; identity model (handle text vs user-profile table) must be decided first.

See `AGENTS.md` for the full list of intentional divergences from `test_app_todo`.
