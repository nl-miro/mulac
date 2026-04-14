# S004 - Clarify outbox mapping boundary

| Field                    | Value                         |
|--------------------------|-------------------------------|
| Priority                 | medium                        |
| File                     | `docs/contracts.md`           |
| Decision                 | accepted                      |
| Implementation reference | 1b00754                       |
| Created at               | 2026-04-14                    |
| Author                   | Codex, gpt-5, medium          |
| Reviewer                 |                               |

## Issue
The Outbox contract says it accepts an `EventEnvelope` and "owns the transformation to an outbound message", but it does not define the mapping boundary. It is unclear whether every event is expected to map to an outbound message, whether an event may map to zero, one, or multiple outbound messages, and whether an unmappable event is a normal no-op, a failed delivery, or a caller error.

## Suggestion
Add a short contract subsection that defines the outbox transformation semantics. Specify the expected cardinality between accepted events and produced outbound messages, how unsupported or unmappable events are handled, and whether mapping failures participate in the same retry path as transport failures. That gives event producers and outbox consumers a clear shared contract instead of relying on implicit implementation behavior.
