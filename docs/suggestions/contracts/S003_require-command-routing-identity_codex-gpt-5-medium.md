# S003 - Require command routing identity

| Field                    | Value                |
|--------------------------|----------------------|
| Priority                 | high                 |
| File                     | `docs/contracts.md`  |
| Decision                 | accepted             |
| Implementation reference | 368a3ec              |
| Created at               | 2026-04-14           |
| Author                   | Codex, gpt-5, medium |
| Reviewer                 |                      |

## Issue
The Command Dispatcher contract requires callers to ensure that "a handler is registered for the command type before dispatching", but the `CommandEnvelope` contract does not define any field that carries that command type or routing identity. The `Accepts` section only requires payload plus correlation, causation, and created-at metadata, which leaves the handler resolution key implicit.

## Suggestion
Make command routing identity explicit in the `CommandEnvelope` contract. That can be a required `command type` field in metadata, a named envelope field outside metadata, or another clearly defined routing key, but the contract should say where it lives and whether the same value is preserved in durable `CommandEntry` records. That closes the gap between the dispatcher's routing rule and the accepted input shape.
