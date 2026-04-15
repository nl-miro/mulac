# S004 - Externally triggered flow missing InboxEntry lifecycle

| Field                    | Value                                |
|--------------------------|--------------------------------------|
| Priority                 | medium                               |
| File                     | `docs/architecture-spec.md`          |
| Decision                 | accepted                             |
| Implementation reference | b0a3e4e                              |
| Created at               | 2026-04-14                           |
| Author                   | Claude Code, claude-sonnet-4-6, high |
| Reviewer                 |                                      |

## Issue
In the Externally triggered flow, step 2 says "the AMQP consumer sends the message to the inbox" and step 3 jumps directly to "the inbox consumer picks up the message and sends a `CommandEnvelope` to the command dispatcher". The creation and reservation of an `InboxEntry` is entirely absent. The Full flow (steps 2–3) correctly shows the inbox storing the message as an `InboxEntry` and the inbox consumer reserving it before producing a `CommandEnvelope`. The Externally triggered flow gives the false impression that the inbox processes messages synchronously rather than through a durable entry.

## Suggestion
Expand the Externally triggered flow to show the `InboxEntry` lifecycle between steps 2 and 3: the inbox stores the message as an `InboxEntry` durably, and the inbox consumer reserves the `InboxEntry` before converting it to a `CommandEnvelope`. This keeps the flow consistent with the Full flow and accurate with respect to the Inbox contract.
