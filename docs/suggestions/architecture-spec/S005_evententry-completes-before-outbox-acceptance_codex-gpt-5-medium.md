# S005 - EventEntry completes before Outbox acceptance

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | high                                  |
| File                     | `docs/architecture-spec.md`           |
| Decision                 | accepted                              |
| Implementation reference | 1bd169e                               |
| Created at               | 2026-04-14                            |
| Author                   | Codex, gpt-5, medium                  |
| Reviewer                 |                                       |

## Issue
In the Full flow, step 9 says the event queue consumer delivers the `EventEntry` to all subscribers and marks it `completed`, but the Outbox-specific subscriber path is only described afterward in steps 10-12. That ordering makes it read as though the Event Dispatcher can complete the `EventEntry` before the Outbox has durably accepted the event. This conflicts with `docs/contracts.md`, which requires the Event Dispatcher not to consider delivery complete until the Outbox has confirmed storage of the `OutboxEntry`.

## Suggestion
Restructure the Full flow so the Outbox acceptance happens inside the event delivery step rather than after the `EventEntry` is already completed. A concrete fix would be to show: the event queue consumer delivers the event to subscribers; if one subscriber is the Outbox, the Outbox stores an `OutboxEntry`; once all subscribers have accepted the event, the `EventEntry` is marked `completed`; only afterward does the outbox consumer reserve the `OutboxEntry` and publish it to AMQP. This keeps the end-to-end sequence aligned with the subscriber contract boundary.
