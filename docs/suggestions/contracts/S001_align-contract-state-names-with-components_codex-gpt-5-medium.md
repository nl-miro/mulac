# S001 - Align contract state names with components

| Field                    | Value                         |
|--------------------------|-------------------------------|
| Priority                 | high                          |
| File                     | `docs/contracts.md`           |
| Decision                 | accepted                      |
| Implementation reference | 50c6530                       |
| Created at               | 2026-04-14                    |
| Author                   | Codex, gpt-5, medium          |
| Reviewer                 |                               |

## Issue
`docs/contracts.md` describes reserved work as `in_progress` and repeatedly refers to returning entries to the "available pool", but the canonical lifecycle in `docs/components.md` uses `received`, `reserved`, `completed`, `failed`, and `dead`. Because the contracts document never maps those terms, it is unclear whether the contract is intentionally defining a second state model or describing the same lifecycle with different names.

## Suggestion
Normalize the contracts document to the canonical lifecycle terms from `docs/components.md`, or add an explicit mapping section if different interface terms are intentional. The same terminology should be used consistently across Inbox, Command Dispatcher, Event Dispatcher, and Outbox so callers can reason about reservation ownership, retries, and terminal states without translating between two state vocabularies.
