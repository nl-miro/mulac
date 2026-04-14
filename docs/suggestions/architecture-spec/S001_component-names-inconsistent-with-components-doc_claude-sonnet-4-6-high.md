# S001 - Component names inconsistent with components doc

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | high                                  |
| File                     | `docs/architecture-spec.md`           |
| Decision                 | accepted                              |
| Implementation reference | a1fbd2f                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 |                                       |

## Issue
The Full flow uses "inbox service" (steps 1–2) and "outbox service" (steps 10–11) while `docs/components.md` and `docs/contracts.md` consistently use "Inbox" and "Outbox" as the canonical component names. The Regular flow and Externally triggered flow use "inbox consumer" without naming the component itself. The inconsistency makes it harder to cross-reference flows with the component and contract definitions.

## Suggestion
Normalise all component references in the spec to match the names used in `components.md`: "Inbox", "Command Dispatcher", "Event Dispatcher", and "Outbox" (capitalised, no "service" suffix). Sub-actors such as consumers should be named consistently too — e.g. "inbox consumer", "command queue consumer", "event queue consumer", "outbox consumer" — matching the terminology already used in `contracts.md`.
