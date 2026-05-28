# Todo feature organization migration plan

This note explains the intended result of the todo feature cleanup in plain
language.

## Goal

Make every feature easier to read by giving it the same internal shape and the
same public boundary.

The result should feel consistent across the codebase, so a reader can open any
feature and immediately know where to look.

## Target shape for every feature

Each feature should contain:

1. `io` — the public surface
2. `intention` — the feature story
3. `ui` — API-facing input and output
4. `implementation` — the working details
5. `temporary_adapters` — temporary adapter support code when needed
6. `tests` — local tests

Not every feature needs every section, but the order should stay consistent.

## What should stay true

This cleanup is about structure, not behavior.

That means:

- feature names stay the same
- public behavior stays the same
- routes and external contracts stay the same
- features continue to be accessed through `io`

## How to think about the result

After the migration:

- the top of each feature should explain what it does
- the outward-facing API layer should be easy to find
- the middle of each feature should contain the implementation detail
- the public surface should be small and obvious
- temporary glue should be clearly separated when it still exists
- tests should live close to the feature they protect

## Feature groups

### Write-oriented features

These features perform an action and produce a result:

- create
- complete
- reopen
- update
- delete
- change due date

For these features:

- `intention` should describe the action clearly
- `ui` should hold API requests, responses, and entry endpoints
- `implementation` should hold the technical work
- `temporary_adapters` should stay small or disappear when no longer needed

### Read-oriented features

These features answer questions:

- get one todo
- list todos

For these features:

- `intention` should make the read flow easy to understand
- `ui` should hold API-facing entry shapes
- `implementation` should hold query details
- `temporary_adapters` should stay lightweight if present

## What good looks like

The migration is successful when:

- every feature follows the same structure
- the public surface is easy to recognize
- feature intent is readable without digging through low-level code
- internal detail is kept out of the public boundary
- tests still pass

## Final expectation

Someone new to the project should be able to open any feature and quickly
understand:

- what the feature is for
- where it starts
- what is public
- where the API boundary lives
- where the detailed work lives
