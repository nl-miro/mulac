# S003 - Outbox dead state does not cover serialisation

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | high                                  |
| File                     | `docs/components.md`                  |
| Decision                 | accepted                              |
| Implementation reference | 5671529                               |
| Created at               | 2026-04-14                            |
| Author                   | Codex, gpt-5, medium                  |
| Reviewer                 |                                       |

## Issue
The Outbox state model says `dead` means "the retry policy was exhausted and the entry will not be retried automatically." That is now incomplete relative to `docs/contracts.md`, which defines a second path to `dead`: a post-acceptance serialisation failure causes the outbox consumer to call `dead(entry_id)` immediately, bypassing the retry schedule. Because `docs/components.md` is the lifecycle-oriented document, readers who start there will currently infer that every `dead` Outbox entry must have consumed the full retry budget, which is no longer true.

## Suggestion
Update the Outbox section so the high-level lifecycle includes both `dead` paths: retry exhaustion and immediate terminal failure on non-retriable post-acceptance transformation errors such as serialisation. The simplest fix is to adjust the `dead` state description and add a short rule or behaviour note that transport failures use the retry path, while serialisation failures go straight to `dead`.
