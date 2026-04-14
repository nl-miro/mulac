# S007 - Dead state operator resolution not defined

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | medium                                |
| File                     | `docs/contracts.md`                   |
| Decision                 | accepted                              |
| Implementation reference | 3b14213                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 |                                       |

## Issue
The entry retry policy states "`dead` entries must be resolved by an operator" but does not define what resolution means. Operators and system integrators cannot design observability, alerting, or recovery runbooks without knowing whether `dead` entries can be re-queued to `received`, must be deleted, or are handled through some other mechanism. The contract currently leaves this as an implementation detail invisible to callers and operators.

## Suggestion
Add a sentence to the entry retry policy describing what "operator resolution" means at the contract level. At minimum, state whether `dead` entries can be re-queued to `received` (and if so, whether the retry counter resets), or whether they must be discarded. If the mechanism is intentionally left to implementation, say so explicitly so operators know the contract boundary stops at `dead`.
