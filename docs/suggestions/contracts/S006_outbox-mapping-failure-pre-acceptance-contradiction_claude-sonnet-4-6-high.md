# S006 - Outbox mapping failure pre-acceptance contradiction

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | high                                  |
| File                     | `docs/contracts.md`                   |
| Decision                 | accepted                              |
| Implementation reference | 990a961                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 |                                       |

## Issue
The Outbox section contains two statements that contradict each other. The `Accepts` section says "a missing or unresolvable routing key is a caller error and the `EventEnvelope` is rejected at acceptance time" — implying no `OutboxEntry` is created. The `Transformation contract` section says "mapping failures (invalid routing key, serialisation error) are caller errors and are not retried; the `OutboxEntry` transitions to `dead` immediately" — implying an `OutboxEntry` exists and transitions state. An entry cannot transition to `dead` if it was never created.

## Suggestion
Separate the two failure paths clearly. Routing key errors caught at acceptance should be described as rejections with no `OutboxEntry` created (consistent with the `Accepts` wording). Serialisation errors, which occur after the `OutboxEntry` exists and the consumer is attempting to publish, should be described as causing an immediate `dead` transition without entering the retry path. The transformation contract section should distinguish these two cases rather than grouping them together.
