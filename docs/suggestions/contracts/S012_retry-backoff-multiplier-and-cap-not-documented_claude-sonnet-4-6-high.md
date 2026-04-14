# S012 - Retry backoff multiplier and cap not documented

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | medium                                |
| File                     | `docs/contracts.md`                   |
| Decision                 | accepted                              |
| Implementation reference | fad8265                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 |                                       |

## Issue
The entry retry policy states "exponential backoff starting at 30 seconds" but does not define the multiplier or any cap on the per-attempt delay. With 5 retries and a typical 2× multiplier the delays would be approximately 30 s, 60 s, 120 s, 240 s, 480 s — a total wait of roughly 16 minutes before an entry reaches `dead`. Without knowing the multiplier or cap, operators and callers cannot calculate the worst-case time-to-dead for any entry, which is necessary for setting alerts, SLAs, and consumer timeout budgets.

## Suggestion
Add the backoff multiplier (default) and any per-attempt maximum delay to the entry retry policy. At minimum state the default multiplier and whether there is an upper bound on a single delay interval. If these are configurable per component, note that alongside the existing "retry count and backoff parameters are configurable" statement.
