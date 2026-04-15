# S002 - Define retry exhaustion and backoff

| Field                    | Value                |
|--------------------------|----------------------|
| Priority                 | high                 |
| File                     | `docs/contracts.md`  |
| Decision                 | accepted             |
| Implementation reference | e9d70b6              |
| Created at               | 2026-04-14           |
| Author                   | Codex, gpt-5, medium |
| Reviewer                 |                      |

## Issue
Across Inbox, Command Dispatcher, Event Dispatcher, and Outbox, the contract says `failed(entry_id)` makes work "eligible for retry", but it does not define whether retries are immediate or scheduled, whether backoff is part of the contract, or what happens when the retry policy is exhausted. `docs/components.md` already introduces a `dead` terminal state and retry waiting semantics, so the contracts document currently drops behavior that materially affects collaborators and operators.

## Suggestion
Add an explicit retry contract for every durable queue-backed entry type. At minimum, state whether `failed` returns the entry directly to the reservable pool or schedules it for a later attempt, whether retry timing/backoff is configurable, and whether retry exhaustion transitions the entry to a terminal `dead` state. If those semantics are shared across all four components, a short common subsection would keep the document compact and consistent.
