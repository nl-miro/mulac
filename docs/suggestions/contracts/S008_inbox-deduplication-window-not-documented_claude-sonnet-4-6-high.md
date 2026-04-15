# S008 - Inbox deduplication window not documented

| Field                    | Value                                |
|--------------------------|--------------------------------------|
| Priority                 | medium                               |
| File                     | `docs/contracts.md`                  |
| Decision                 | accepted                             |
| Implementation reference | 103107e                              |
| Created at               | 2026-04-14                           |
| Author                   | Claude Code, claude-sonnet-4-6, high |
| Reviewer                 |                                      |

## Issue
The Inbox `Guarantees` section references "the deduplication retention window" as a key concept: messages with a known ID are absorbed within the window, and messages outside the window are accepted as new. However, the window's default duration and whether it is configurable are never stated. Transport adapters and callers must understand this window to reason about whether a redelivered message after a long delay will be deduplicated or produce a new `InboxEntry`.

## Suggestion
Add the deduplication retention window default value and its configurability to the Inbox contract — either in the `Guarantees` section alongside the existing deduplication bullet, or as a separate item under a configuration note similar to the sweep interval. Treat it consistently with the other configurable parameters (sweep interval, staleness threshold, retry count, backoff) already documented in the contracts.
