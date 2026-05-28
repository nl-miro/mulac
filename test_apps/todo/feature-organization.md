# Feature Organization

This document explains how to organize a feature inside an app crate.

The goal is simple:

- each feature should be easy to read
- each feature should be easy to change or remove
- the public surface should be small and obvious
- business intent should stay separate from implementation detail

## Core idea

One feature lives in one source file.

That file should read from top to bottom in a predictable order:

1. `io` — what other parts of the app are allowed to use
2. `intention` — what the feature means and does
3. `ui` — API-facing input and output types plus feature entry endpoints
4. `implementation` — how the feature actually works
5. `temporary_adapters` — temporary boundary support code when needed
6. `tests` — feature-local tests

Not every feature will need every section, but the order should stay predictable.

## The only public surface: `io`

`io` is the only public module in a feature.

Use it to expose only the small set of things the rest of the app truly needs:

- the feature entry point
- feature-owned commands or events
- any shared feature types that are intentionally public

Nothing outside the feature should reach into its internal modules.

## `intention`: the story of the feature

This is the most readable part of the file.

It should describe:

- the main feature types
- the action or question the feature represents
- the feature's business vocabulary
- the business rules of the feature, expressed as small methods on its types

When someone opens a feature, this section should tell them what the feature is
for without making them study low-level details.

Keep this section focused on meaning, not plumbing.

### Rules belong here, not in the mechanics

A feature's rules are part of its meaning, so they live in `intention` as small
methods on the command type — never buried in SQL or other `implementation`
detail. For example:

- a validation rule: `UpdateTodo::validate()` checks the title is present
- a creation rule: `CreateTodo::begin(now)` validates, then yields a `NewTodo`
  that always starts `Active` and is stamped now
- a state transition: `CompleteTodo::resulting_status()` returns `Completed`

`implementation` then asks `intention` for the decision and carries it out. If
you can read a feature's rules without leaving `intention`, this section is doing
its job.

## `ui`: the feature's outward-facing boundary

This section holds the user-facing or API-facing layer for the feature.

Put here:

- endpoint types
- request and response shapes
- feature entry endpoints
- simple endpoint orchestration

Keep this layer focused on boundary concerns. It should not become the home for
business rules or low-level technical detail.

## `implementation`: the working parts

This section holds the mechanics behind the feature.

Put here:

- helper methods
- persistence and translation details
- feature constants
- technical trait implementations
- runtime mechanics behind the feature flow

If `intention` explains the feature in plain language, `implementation` is where
the details live.

### Events own their schema

A feature's event should declare its own fields rather than embedding a shared
persistence type (such as a database row). The event then controls its own
published contract, and a change to the storage layer cannot silently reshape it.
`implementation` provides the structural mapping with a `From` conversion, e.g.
`impl From<TodoEntry> for TodoCreated`.

### Crate-wide concerns do not live in a feature

A translation or trait implementation that is shared by every feature — for
example mapping the app's error type into the command layer's error type — is not
the property of any single feature. Put it in the shared assembly layer so one
feature can be removed without breaking the others.

## `temporary_adapters`: temporary boundary support code

Use this section for code that translates between the outside world and the
feature's own types when a separate adapter layer is still useful.

Examples:

- temporary translation glue
- boundary helper code
- compatibility support during refactors

Do not let important domain logic settle here.

## `tests`

Keep feature-local tests in the same file.

These tests should confirm:

- the feature entry flow behaves correctly
- important branches are covered
- small helpers behave as expected

## Two common feature shapes

Most features fit one of these patterns:

### 1. Command-driven feature

Something asks the feature to do work, and the feature produces results that
other parts of the system can react to.

### 2. Event-driven feature

Something happens elsewhere, and this feature reacts to it and decides what to
do next.

The important point is not the exact technical shape. The important point is
that every feature should have one clear entry point and one clear purpose.

## Dependency rules

### Inside a feature

- `io` exposes the public surface
- `intention` explains the feature
- `ui` owns the outward-facing boundary
- `implementation` supports the feature mechanics
- `temporary_adapters` keeps temporary glue separate when needed
- `tests` may inspect internals

### Across features

- one feature may depend on another only through `io`
- features should not reach into each other’s internal structure
- shared concepts should be imported through the public surface only

### Import path standardization

Shared, app-owned concepts (error types, domain types, the command/envelope
helpers, persistence helpers) must be imported from the assembly's public
surface — `crate::assembly::io` — and nowhere else:

```rust
use crate::assembly::io::{AppError, TodoEntry, TodoStatus, dispatch_command};
```

Do not reach for these through the crate-root facade (`crate::io::…`) from inside
a feature. The crate-root `io` is the facade for the binary and other external
consumers; using it from a feature creates two import paths for the same type
and blurs which surface a feature actually depends on. One type, one path: a
feature depends on `assembly` through `crate::assembly::io`.

By default, import every dependency with a `use` statement at the top of the
module and refer to it by its short name in the body. Avoid inline fully
qualified paths like `Result<Json<crate::assembly::io::TodoEntry>, ApiError>` —
write `use crate::assembly::io::TodoEntry;` and then `Result<Json<TodoEntry>,
ApiError>`. A module's `use` block should be the single, readable list of
everything that module depends on.

## Writing style for features

When adding or refactoring a feature:

- prefer readability over cleverness
- keep the public surface small
- keep naming consistent
- keep the top-level flow easy to follow
- move technical detail downward into `implementation`
- keep API-facing shapes in `ui`

## Practical rule of thumb

If a new reader can quickly answer these questions, the feature is organized
well:

- What is this feature for?
- Where does it start?
- What is public?
- Where do the low-level details live?
- Can I change this feature without touching unrelated code?
