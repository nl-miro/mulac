# S001 - Command Dispatcher direct handoff failure missing

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | high                                  |
| File                     | `docs/components.md`                  |
| Decision                 | accepted                              |
| Implementation reference | 96d2031                               |
| Created at               | 2026-04-14                            |
| Author                   | Codex, gpt-5, medium                  |
| Reviewer                 |                                       |

## Issue
`docs/components.md` says it covers reliability boundaries, but the Command Dispatcher section only describes the post-handler handoff failure case for the `TwoPhased` variant. Rule 3 says "If execution succeeds but handoff to event dispatch fails, the command entry is retried", which applies only when a durable `CommandEntry` exists. After the contract update, the Direct variant now has an explicit and materially different boundary: if the handler succeeds and inline handoff to the Event Dispatcher fails, the caller receives an error, handler side-effects are not rolled back, and the produced events are not retained for automatic redelivery. That high-level boundary is currently absent from the component overview.

## Suggestion
Add a Direct-specific reliability note to the Command Dispatcher behaviour or rules. At minimum, state that if the handler succeeds but inline handoff to the Event Dispatcher fails, the error propagates to the caller, no automatic retry record exists, and any handler side-effects remain in place. Keep the existing TwoPhased retry note, but distinguish it clearly from the Direct variant.
