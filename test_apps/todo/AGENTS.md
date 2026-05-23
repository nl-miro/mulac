# test_app_todo — Codebase Overview

Quick reference for AI assistants. Read this instead of scanning every source file.

---

## What it is

A small but realistic Rust todo REST API used to exercise the `mulac` workspace architecture:
CQRS-style command/event pipeline (`kernel` + `commanding` + `eventing` crates), inbox/outbox messaging,
and `poem` / `poem-openapi` HTTP layer.

**Stack:** Rust · Tokio · Poem · poem-openapi · SQLx · PostgreSQL · UUID v7

---

## Source map

```
src/
  lib.rs                — AppState { pool, mulac }, public re-exports, TodoEvent enum,
                          inbox + outbox module helpers
  assembly/
    application.rs      — MulacState, MulacHandle, start_mulac(), command/event wiring,
                          EventSubscriberRegistry, CommandHandlerRegistry,
                          block_on_blocking() helper, run_command_worker, run_event_worker
    domain.rs           — TodoDto, TodoStatus, Clock
    infra_sqlx_pg.rs    — connect(), migrate() via sqlx::migrate!, fetch_todo(),
                          record_event_payload(), OutboxSubscriber, TodoRow entity
    mod.rs              — public io module with facade exports
    bin/todoapp.rs      — entry point: "serve" sub-command, wires HTTP + workers
  task_create.rs        — CreateTodoCommand, CreateTodoHandler, TodoCreated event, POST /todos
  task_complete.rs      — CompleteTodoCommand, CompleteTodoHandler, TodoCompleted event,
                          POST /todos/:id/complete
  task_reopen.rs        — ReopenTodoCommand, ReopenTodoHandler, TodoReopened event,
                          POST /todos/:id/reopen
  task_update.rs        — UpdateTodoCommand, UpdateTodoHandler, TodoUpdated event, PUT /todos/:id
  task_delete.rs        — DeleteTodoCommand, DeleteTodoHandler, TodoDeleted event, DELETE /todos/:id
  task_schedule_due_dates.rs — UpdateDueDateCommand, UpdateDueDateHandler,
                          TodoDueDateChanged event, PUT /todos/:id/due-date
  task_list.rs          — GET /todos?status=active|completed|archived|all
  task_get.rs           — GET /todos/:id

migrations/
  0001_init.sql         — todos, inbox_messages, outbox_messages tables + indexes
  0002_write_side.sql   — command_entries, event_entries tables + indexes (mulac write-side)

tests/
  todo_create.rs        — POST /api/todos happy path + blank title 400
  todo_update.rs        — PUT /api/todos/:id happy path + blank title 400
  todo_delete.rs        — DELETE /api/todos/:id happy path (204) + nonexistent 404
  todo_get.rs           — GET /api/todos/:id happy path + nonexistent 404
  todo_list.rs          — GET /api/todos all + status filter
  todo_complete.rs      — POST /api/todos/:id/complete happy path + nonexistent 404
  todo_reopen.rs        — POST /api/todos/:id/reopen happy path + nonexistent 404
  todo_due_date.rs      — PUT /api/todos/:id/due-date happy path + nonexistent 404
  inbox.rs              — inbox lifecycle, idempotency (409), malformed payload
  utils.rs              — shared test utilities: start_test_app(), row structs,
                          STATUS_COMPLETED constant
```

---

## Feature module convention

Every feature file (task_*.rs) exposes **only** `pub mod io` — all internal sub-modules are private:

```
pub const COMMAND_NAME: &str = "...";
pub const EVENT_NAME: &str = "...";

mod models { ... }       // command + event structs + traits
mod handler { ... }      // CommandHandlerPort impl
mod infra_sqlx_pg { ... } // raw sqlx queries
mod http { ... }         // poem-openapi OpenApi impl + request/response structs

pub mod io {
    pub use super::COMMAND_NAME;
    pub use super::EVENT_NAME;
    pub use super::models::{Command, Event};
    pub use super::handler::Handler;
    pub use super::http::Api;
}
```

Callers always import via `feature::io::*`, never internal sub-modules.

---

## Domain model

```rust
struct TodoDto {
    id: Uuid,            // UUIDv7
    title: String,       // non-blank
    description: Option<String>,
    status: TodoStatus,  // active | completed | archived
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    due_at: Option<DateTime<Utc>>,
}
```

---

## REST API

All state-changing endpoints dispatch commands through Mulac. All operations have domain events.
Application API endpoints are mounted under the `/api` prefix. Health and Swagger UI remain mounted at `/health` and `/swagger`.

| Method | Path                    | Command           | Event              |
|--------|-------------------------|-------------------|--------------------|
| POST   | /api/todos              | CreateTodo        | TodoCreated        |
| GET    | /api/todos              | —                 | —                  |
| GET    | /api/todos/:id          | —                 | —                  |
| PUT    | /api/todos/:id          | UpdateTodo        | TodoUpdated        |
| POST   | /api/todos/:id/complete | CompleteTodo      | TodoCompleted      |
| POST   | /api/todos/:id/reopen   | ReopenTodo        | TodoReopened       |
| DELETE | /api/todos/:id          | DeleteTodo        | TodoDeleted        |
| PUT    | /api/todos/:id/due-date | UpdateDueDate     | TodoDueDateChanged |
| POST   | /api/messages/commands  | (any TodoCommand) | (dispatched event) |
| GET    | /api/messages/outbox    | —                 | —                  |
| GET    | /health                 | —                 | —                  |
| GET    | /swagger                | —                 | —                  |

Query parameters:
- `GET /api/todos?status=active|completed|archived|all` — filter by status (default: all)

State-changing endpoints return `TodoDto` as JSON, except `DELETE /api/todos/:id` which returns `204 No Content`.
`GET` endpoints return `TodoDto` or `{ "items": [...] }`.
Errors: `{ "error": "..." }` with appropriate HTTP status codes (404, 400, 409, 500).

### POST /api/messages/commands — inbox JSON shapes

**HTTP request body** — poem-openapi `#[derive(Union)]` with `discriminator_name = "type"`. No wrapper object around the command fields:

```json
{
  "id": "<uuid-v7>",
  "command": {
    "type": "CreateTodo",
    "title": "Buy milk",
    "description": "Optional"
  }
}
```

Supported `type` values and their fields:

| type          | required fields               | optional fields |
|---------------|-------------------------------|-----------------|
| CreateTodo    | `title`                       | `description`   |
| UpdateTodo    | `todo_id`, `title`            | `description`   |
| CompleteTodo  | `todo_id`                     |                 |
| ReopenTodo    | `todo_id`                     |                 |
| DeleteTodo    | `todo_id`                     |                 |
| UpdateDueDate | `todo_id`, `due_at` (RFC3339) |                 |

**Stored payload in `outbox_messages`** — serialized with `serde` `#[serde(tag = "type", content = "payload")]`, which adds a `"payload"` wrapper:

```json
{
  "type": "CreateTodo",
  "payload": {
    "title": "Buy milk",
    "description": "Optional"
  }
}
```

The two shapes differ: HTTP input has no `"payload"` key; the stored outbox record does.

---

## Command / event flow (two-phased via mulac kernel)

All state-changing HTTP endpoints follow this flow:

```
HTTP handler
  └─ dispatch_command(envelope)           // MulacState
       ├─ CommandGateway.dispatch()       // persists to command_entries (status=pending)
       ├─ CommandConsumer.consume()       // reserves + executes handler
       │    └─ <Feature>Handler.execute()
       │         └─ infra_sqlx_pg::* (INSERT/UPDATE/DELETE in transaction)
       │              └─ emits Vec<TodoEvent> via handler return
       └─ EventConsumer.consume()         // reserves + dispatches events
            └─ EventSubscriberRegistry
                 └─ OutboxSubscriber
                      └─ record_event_payload() writes to outbox_messages
```

Background workers in `assembly/application.rs` repeat the consume step every 1 s for reliability.

**All state-changing operations** (create, complete, reopen, update, delete, due-date change)
**emit events** that are persisted to the outbox via subscribers.

---

## Database tables

| Table             | Purpose                                                                                  |
|-------------------|------------------------------------------------------------------------------------------|
| `todos`           | canonical todo rows                                                                      |
| `outbox_messages` | domain events pending publication (status: pending/published/failed)                     |
| `inbox_messages`  | inbound commands received via /api/messages/commands (status: received/processed/failed) |
| `command_entries` | mulac write-side command journal (status int, 5=completed)                               |
| `event_entries`   | mulac write-side event journal (status int, 5=completed)                                 |

---

## AppState

```rust
struct AppState {
    pool: PgPool,         // sqlx connection pool (read queries, command dispatch)
    mulac: MulacState,    // command gateway + consumers (command dispatch path)
}
```

Both are available in every HTTP handler via `Data<&AppState>`.

---

## Error handling

`AppError` variants → HTTP status:
- `NotFound` → 404
- `Validation(msg)` → 400
- `Conflict(msg)` → 409
- `Storage(anyhow::Error)` → 500

All errors serialize as `{ "error": "<message>" }`.

---

## Running locally

```bash
# Infrastructure
make up          # docker compose up (postgres, rabbitmq)
make migrate     # run migrations
make serve       # cargo run --release -- serve (binds 127.0.0.1:33001)

# Tests (requires running postgres)
DATABASE_URL=postgres://... make test

# Reset
make reset       # docker compose down -v && up && migrate
```

Environment variables:
- `DATABASE_URL` — required (e.g., `postgres://todo:todo@localhost:5432/test_app_todo`)
- `BIND_ADDR` — optional, default `127.0.0.1:33001`
- `RUST_LOG` — optional, controls tracing output (e.g., `debug`, `info`)

---

## Workspace crate dependencies

- **`kernel`** — `ApplicationCommand`, `ApplicationEvent`, `CommandHandlerPort`,
  `EventSubscriberPort`, `NewCommandEnvelope`, `KernelError`, etc.
- **`commanding`** — `CommandGateway`, `CommandConsumer`, `CommandRecorder`,
  storage types, `wrap_handler`
- **`eventing`** — `EventGateway`, `EventConsumer`, `EventRecorder`, storage types
- **`mulac_diesel`** — `build_pool()` for database setup

These live in `../kernel`, `../commanding`, `../eventing`, and `../mulac_diesel` relative to this crate.
