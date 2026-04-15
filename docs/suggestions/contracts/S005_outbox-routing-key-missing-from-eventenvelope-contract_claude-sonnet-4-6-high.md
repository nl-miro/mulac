# S005 - Outbox routing key missing from EventEnvelope contract

| Field                    | Value                                |
|--------------------------|--------------------------------------|
| Priority                 | high                                 |
| File                     | `docs/contracts.md`                  |
| Decision                 | accepted                             |
| Implementation reference | ce2a249                              |
| Created at               | 2026-04-14                           |
| Author                   | Claude Code, claude-sonnet-4-6, high |
| Reviewer                 |                                      |

## Issue
The Outbox `Accepts` section states that "the `EventEnvelope` must carry a routing key that the Outbox can resolve to an outbound transport destination", but the Event Dispatcher contract defines `EventEnvelope`'s required metadata as: event type, correlation ID, causation ID, created-at timestamp — with no routing key field. The field that serves as the Outbox routing key is never identified anywhere in the document. A reader cannot determine whether the routing key is the event type, a separate metadata field, or something else.

## Suggestion
Clarify where the routing key lives in `EventEnvelope`. If the routing key is the event type field already present in required metadata, state that explicitly in the Outbox `Accepts` section (e.g., "the Outbox uses the `event type` field as the routing key"). If it is a distinct field, add it to the `EventEnvelope` required metadata in the Event Dispatcher contract and reference it from the Outbox section.
