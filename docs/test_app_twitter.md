# Twitter Test App

A demonstration application in `test_app_twitter/` that exercises the mulac messaging infrastructure through a simplified Twitter-like use case.

## Location

The application should live in:

```text
test_app_twitter/
```

## Goal

`test_app_twitter` should be a realistic messaging-heavy test application that demonstrates how HTTP entrypoints, inbox consumption, outbox publishing, and event-driven flows work together in this repository.

## HTTP/API expectations

The HTTP API is built with [poem](https://github.com/poem-web/poem) and [poem-openapi](https://github.com/poem-web/poem/tree/master/poem-openapi).
Each endpoint is defined using `poem-openapi` derive macros, which generates an OpenAPI 3.0 schema and serves a Swagger UI.

The application should expose:

- the main HTTP API
- generated OpenAPI / Swagger UI, for example at `/swagger`
- a health endpoint, for example `/health`

```toml
# test_app_twitter/Cargo.toml
[dependencies]
poem = "3"
poem-openapi = { version = "5", features = ["swagger-ui"] }
tokio = { version = "1", features = ["full"] }
```

## Application structure

`main.rs` is responsible only for wiring: it builds shared state, constructs feature APIs, assembles the Poem app, and starts the server.

Recommended layout:

```text
test_app_twitter/
├── Cargo.toml
├── Makefile
├── docker-compose.yml
└── src/
    ├── main.rs
    ├── tweets.rs
    ├── tweets/
    │   ├── post_tweet.rs
    │   ├── delete_tweet.rs
    │   └── retweet.rs
    ├── users.rs
    ├── users/
    │   ├── follow_user.rs
    │   └── unfollow_user.rs
    ├── likes.rs
    ├── likes/
    │   ├── like_tweet.rs
    │   └── unlike_tweet.rs
    ├── notifications.rs
    ├── notifications/
    │   └── mention.rs
    ├── timeline.rs
    ├── timeline/
    │   └── fan_out.rs
    ├── messages.rs
    └── messages/
        └── send_direct_message.rs
```

Module convention:

- each feature is rooted in a named module file such as `tweets.rs`
- that module file owns its internal submodules in the matching folder such as `tweets/`
- each feature module exposes one `Api` struct and the request/response types it owns

### main.rs

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let state = AppState::from_env()?;

    let api_service = OpenApiService::new(
        (
            tweets::Api::new(state.clone()),
            users::Api::new(state.clone()),
            likes::Api::new(state.clone()),
            notifications::Api::new(state.clone()),
            timeline::Api::new(state.clone()),
            messages::Api::new(state.clone()),
        ),
        "Twitter Test App",
        "0.1.0",
    )
    .server("http://localhost:3000");

    let ui = api_service.swagger_ui();

    poem::Server::new(TcpListener::bind("0.0.0.0:3000"))
        .run(Route::new().nest("/", api_service).nest("/swagger", ui))
        .await?;

    Ok(())
}
```

### Feature module layout

Each feature module exposes one `Api` struct with `#[OpenApi]` and keeps all its request/response types local:

```rust
// src/tweets.rs
mod post_tweet;
mod delete_tweet;
mod retweet;

use poem_openapi::{payload::Json, OpenApi};

pub struct Api { /* shared state */ }

#[OpenApi(prefix_path = "/tweets")]
impl Api {
    #[oai(path = "/", method = "post")]
    async fn post_tweet(&self, body: Json<post_tweet::Request>) -> post_tweet::Response { ... }

    #[oai(path = "/:id", method = "delete")]
    async fn delete_tweet(&self, id: Path<Uuid>) -> delete_tweet::Response { ... }

    #[oai(path = "/:id/retweet", method = "post")]
    async fn retweet(&self, id: Path<Uuid>) -> retweet::Response { ... }
}
```

## Functionalities

Each functionality should clearly indicate whether it is primarily exercised via HTTP, AMQP, or both.

1. **Post a tweet** — **both**  
   A user submits a short text message (≤ 280 characters). The system should support the HTTP path and at least one AMQP/inbox-driven path that results in storing and processing a `PostTweet` command.

2. **Follow a user** — **HTTP**  
   A user follows another account; triggers a `FollowUser` command that updates the follower/following relationship.

3. **Unfollow a user** — **HTTP**  
   A user removes a follow; triggers an `UnfollowUser` command.

4. **Like a tweet** — **HTTP + outbox event**  
   A user likes a tweet; triggers a `LikeTweet` command and emits a `TweetLiked` event.

5. **Unlike a tweet** — **HTTP**  
   A user removes a like; triggers an `UnlikeTweet` command.

6. **Retweet** — **HTTP + outbox event**  
   A user retweets an existing tweet; triggers a `Retweet` command and emits a `TweetRetweeted` event.

7. **Delete a tweet** — **HTTP + outbox event**  
   The author removes their own tweet; triggers a `DeleteTweet` command and emits a `TweetDeleted` event.

8. **Mention notification** — **event-driven**  
   When a tweet contains `@username`, a `UserMentioned` event is emitted and routed via the Outbox to a notification queue.

9. **Home timeline fan-out** — **event-driven**  
   After a tweet is posted, a `TweetPosted` event fans out to the author's followers through the event dispatcher, writing timeline data for followers.

10. **Direct message** — **both**  
    A user sends a private message to another user. The system should support the HTTP path and at least one AMQP/inbox-driven path that results in processing a `SendDirectMessage` command and publishing any related outbound messages.

## Messaging expectations

The application should exercise the messaging architecture in a concrete way.

At minimum, it should demonstrate:

- inbound message handling through the inbox
- outbound event/message publishing through the outbox
- at least one fan-out or downstream event-driven workflow
- message persistence and retriable processing behavior

## Persistence expectations

The app should persist its core state in PostgreSQL.

At minimum, persist:

- tweets
- follow relationships
- likes
- direct messages
- inbox state
- outbox state

Database migrations should be part of setup and test flow.

## Running tests

Tests require a running PostgreSQL instance and a RabbitMQ broker. Both are provided via Docker Compose.

A `Makefile` in `test_app_twitter/` should wrap all common steps:

| Target         | Description                                                      |
|----------------|------------------------------------------------------------------|
| `make up`      | Start PostgreSQL and RabbitMQ containers                         |
| `make down`    | Stop and remove containers                                       |
| `make migrate` | Run database migrations                                          |
| `make test`    | Run the full test suite against live containers                  |
| `make reset`   | Tear down containers, wipe volumes, and bring them back up clean |
| `make serve`   | Build and run the HTTP server on `localhost:3000`                |

### Quick start

```sh
make up
make migrate
make test
make down
```

### Services

| Service    | Default port                           | Credentials                                      |
|------------|----------------------------------------|--------------------------------------------------|
| PostgreSQL | `5432`                                 | `postgres` / `postgres`, database `twitter_test` |
| RabbitMQ   | `5672` (AMQP), `15672` (management UI) | `guest` / `guest`                                |

Connection strings are read from environment variables so they can be overridden without editing source:

```sh
DATABASE_URL=postgres://postgres:postgres@localhost:5432/twitter_test
AMQP_URL=amqp://guest:guest@localhost:5672
```

## Non-goals

These are not required for the first version unless added later:

- authentication / authorization
- media uploads
- recommendation or ranking algorithms
- advanced pagination or cursor semantics
- moderation workflows
- real-time push delivery
- production-grade notification preferences
