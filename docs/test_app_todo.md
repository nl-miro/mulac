# Test App Todo — Functional Scope

This document defines the initial functional scope for the `test_app_todo` application.

## Location

The application should live in:

```text
test_app_todo/
```

## Goal

`test_app_todo` should be a small but realistic todo application used to exercise the current architecture, module boundaries, HTTP integration, and inbox/outbox messaging patterns in this repository.

## Required functionalities

The app should support these 11 functionalities:

1. **Create a todo item**  
   A user can add a new todo with a title and optional description.

2. **List todo items**  
   The app can return all existing todos.

3. **Get todo details**  
   A user can fetch a single todo by its identifier.

4. **Update todo content**  
   A user can edit the title and description of an existing todo.

5. **Mark todo as completed**  
   A user can mark a todo as done.

6. **Reopen a completed todo**  
   A completed todo can be moved back to an active state.

7. **Delete a todo item**  
   A user can remove a todo permanently.

8. **Filter todos by status**  
   The app can show active, completed, archived, or all todos.

9. **Assign due date / schedule**  
   A todo may optionally have a due date or scheduled time.

10. **Emit and process todo-related messages/events**  
    The app should integrate with the repository messaging/inbox-outbox direction so todo actions can produce and consume messages for testing architectural flows.

11. **Archive and unarchive a todo**  
    An active or completed todo can be archived, which makes it read-only. Unarchiving returns it to active.

## Suggested domain model

A todo item should at minimum contain:

- `id`
- `title`
- `description`
- `status`
- `created_at`
- `updated_at`
- optional `due_at`

Recommended identifier type:

- `Uuid`

## Statuses and transitions

Statuses: `active` (the initial status on create), `completed`, `archived`.

| Command | from `active` | from `completed` | from `archived` |
| --- | --- | --- | --- |
| Complete — `POST /todos/:id/complete` | → `completed`, `TodoCompleted` | 409 | 409 |
| Reopen — `POST /todos/:id/reopen` | 409 | → `active`, `TodoReopened` | 409 |
| Archive — `POST /todos/:id/archive` | → `archived`, `TodoArchived` | → `archived`, `TodoArchived` | 409 |
| Unarchive — `POST /todos/:id/unarchive` | 409 | 409 | → `active`, `TodoUnarchived` |
| Update — `PUT /todos/:id` | updated, `TodoUpdated` | updated, `TodoUpdated` | 409 |
| Due date — `PUT /todos/:id/due-date` | updated, `TodoDueDateChanged` | updated, `TodoDueDateChanged` | 409 |
| Delete — `DELETE /todos/:id` | 204, `TodoDeleted` | 204, `TodoDeleted` | 204, `TodoDeleted` |

Rules:

- **409 Conflict** means the command is not allowed from the current status. It changes no row, leaves `updated_at` alone, and produces no event and no outbox entry. Repeating a transition (completing a completed todo, reopening an active one, archiving an archived one) is a conflict, not a silent no-op.
- **404 Not Found** for every command whose todo does not exist, including a repeated delete.
- A transition only applies if the todo is still in the expected status when it is written. Two concurrent identical transitions therefore produce exactly one success and one 409.
- Every successful change sets `updated_at` to the time of the change. `id` and `created_at` never change.
- Each successful change emits exactly one event, whose `todo` snapshot equals the state after the change (for `TodoDeleted`, the state just before deletion).

## Field and date semantics

- `PUT /todos/:id` and `PUT /todos/:id/due-date` replace the whole value; there are no partial updates.
- `title` is required and trimmed before storage; a blank title is a validation error (400) and changes nothing.
- `description` is optional. Omitting it and sending `null` both clear it.
- `due_at` is optional. Omitting it and sending `null` both clear it.
- `due_at` is an RFC 3339 timestamp with an offset, normalised to UTC. Storage, the HTTP response and the event all carry the same instant, whatever offset was sent. An invalid timestamp is rejected and changes nothing.
- Past due dates are accepted.
- `POST /todos` accepts optional `description` and `due_at` under the same rules.

## List contract

- `GET /todos?status=<filter>` accepts `active`, `completed`, `archived` or `all`. Omitting `status` means `all`. Any other value is a 400 error.
- A filtered list contains exactly the todos in that status.
- Ordering: `created_at` ascending, ties broken by `id` ascending.
- An empty result is `200` with `{"items": []}`.
- `GET /todos/:id` returns the current state; a deleted todo is 404 there and absent from every list.

## Inbox command response

`POST /messages/commands` returns `200 {"message_id": ..., "todo": ...}` once the command has been applied:

- For every command except `DeleteTodo`, `todo` is the todo as it is after the command.
- For `DeleteTodo`, `todo` is `null`. The deletion is committed and the inbox message is marked `processed`.
- If the command is rejected (400, 404, 409), the inbox message is marked `failed` and the same status code is returned.

## HTTP/API expectations

For the HTTP API, use:

- `poem` as the web framework
- `poem-openapi` for OpenAPI schema and endpoint definition

The application should:

- expose its web API through Poem routes
- derive its API contract from `poem-openapi` types and annotations
- expose generated OpenAPI / Swagger UI, for example at `/swagger`
- expose a simple health endpoint, for example `/health`

## Messaging expectations

The messaging requirements should be concrete, not only conceptual.

At minimum, define and use a small set of todo-related commands and events, such as:

- commands:
  - `CreateTodo`
  - `CompleteTodo`
  - `ReopenTodo`
  - `UpdateTodo`
  - `DeleteTodo`
  - `ArchiveTodo`
  - `UnarchiveTodo`
- events:
  - `TodoCreated`
  - `TodoCompleted`
  - `TodoDueDateChanged`
  - `TodoArchived`
  - `TodoUnarchived`

At least one inbound path should be exercised through the inbox, and at least one outbound path should be exercised through the outbox.

## Persistence expectations

The app should persist its state in PostgreSQL.

At minimum, persist:

- todo items
- inbox state used by inbound message processing
- outbox state used by outbound event/message dispatching

Database migrations should be part of the app setup and test flow.

## Non-goals for the first version

These are not required unless explicitly added later:

- authentication / authorization
- multi-user collaboration
- attachments
- comments
- tags / labels
- search
- pagination
- UI polish
- notifications

## Application structure

`test_app_todo` should be an executable application with a `main.rs` that boots and runs the service.

Recommended shape:

```text
test_app_todo/
  src/
    main.rs
    lib.rs
    commanding.rs
    db.rs
    workers.rs
    todos.rs
    todos/
      create.rs
      list.rs
      get.rs
      update.rs
      delete.rs
      complete.rs
      reopen.rs
    schedule_due_dates.rs
    messaging.rs
```

Each feature should live in its own named module file, and that file may define and own its internal submodules. Filtering by status is handled within `todos/list.rs` via an optional query parameter rather than a separate module.

The feature modules should be integrated into the Poem application so the final HTTP service is assembled from these feature-specific route and API definitions.

`main.rs` should focus on wiring and startup only.

## Local test environment

To run tests for `test_app_todo`, the local environment should include Docker-based infrastructure for:

- PostgreSQL
- RabbitMQ

The app test flow should assume these services are started through Docker rather than installed manually on the host.

## Developer ergonomics

`test_app_todo` should include a `Makefile` that keeps common workflows simple.

At minimum, the Makefile should help with:

- starting required Docker services
- stopping Docker services
- resetting local test data if needed
- running database migrations
- running application tests
- running the full local verification flow
- running the application locally

Suggested targets:

- `make up`
- `make down`
- `make migrate`
- `make test`
- `make reset`
- `make serve`
- `make check`

The goal is that a developer can get the app and its tests running with a small number of predictable `make` commands.

## Acceptance expectations

The initial implementation should be considered acceptable when:

- all 10 required functionalities exist
- the app starts from `main.rs`
- the HTTP API is integrated with Poem and `poem-openapi`
- Docker-backed PostgreSQL and RabbitMQ are enough to run tests locally
- the Makefile covers the common development and verification flow
