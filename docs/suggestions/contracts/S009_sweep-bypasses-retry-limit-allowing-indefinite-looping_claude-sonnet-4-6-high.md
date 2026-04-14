# S009 - Sweep bypasses retry limit allowing indefinite looping

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | medium                                |
| File                     | `docs/contracts.md`                   |
| Decision                 | accepted                              |
| Implementation reference | 563e24e                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 |                                       |

## Issue
The entry retry policy states that "a stale-reservation release (sweep) returns the entry to `received` without incrementing the retry counter". This means a consumer that repeatedly reserves an entry and lets it go stale — without ever calling `failed` — causes the entry to cycle through `received → reserved → received` indefinitely, never approaching the retry limit and never reaching `dead`. The retry limit can only be enforced if the consumer cooperates by calling `failed`. This is a material operational edge case not currently described in the contract.

## Suggestion
Add a note to the entry retry policy acknowledging this behavior and its implication: consumers that consistently let entries go stale (by not reporting `failed`) prevent retry exhaustion. State whether the system provides any secondary mechanism to detect or break this cycle (for example, a configurable maximum sweep count per entry, or an operator alert on entries with high sweep counts), or explicitly state that sweep-only cycling is outside the retry contract and must be addressed through operational monitoring.
