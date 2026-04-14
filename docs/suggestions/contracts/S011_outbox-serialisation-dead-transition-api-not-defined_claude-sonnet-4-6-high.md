# S011 - Outbox serialisation dead transition API not defined

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | high                                  |
| File                     | `docs/contracts.md`                   |
| Decision                 | accepted                              |
| Implementation reference | 2e8240f                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 |                                       |

## Issue
The Outbox transformation contract states "serialisation errors occur after the `OutboxEntry` exists; the entry transitions to `dead` immediately and is not retried." However, the consumer-facing operations section only exposes `completed(entry_id)` and `failed(entry_id)`. There is no `dead(entry_id)` operation. It is therefore unclear how a consumer that encounters a serialisation error actually triggers the `dead` transition: does it call `failed` with some flag or metadata that bypasses backoff, does the system detect a serialisation error automatically, or is there an undocumented third operation?

## Suggestion
Add the mechanism for triggering an immediate `dead` transition to the consumer-facing operations or the transformation contract. Options include: a dedicated `dead(entry_id)` operation alongside `completed` and `failed`; a `failed(entry_id, permanent: true)` variant; or a note that the system detects serialisation errors internally without the consumer needing to signal them. Whichever approach is chosen should be consistent with how serialisation errors bypass the retry policy for all entry types covered by the shared retry policy section.
