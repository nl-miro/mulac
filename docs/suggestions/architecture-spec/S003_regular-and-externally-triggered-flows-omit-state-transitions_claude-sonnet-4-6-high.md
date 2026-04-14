# S003 - Regular and externally triggered flows omit state transitions

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | medium                                |
| File                     | `docs/architecture-spec.md`           |
| Decision                 | accepted                              |
| Implementation reference | d973dd1                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 |                                       |

## Issue
The Full flow explicitly notes when entries are marked `completed` (steps 3, 7, 9, 12), giving readers a clear picture of state ownership at each handoff. The Regular flow and Externally triggered flow are silent on state transitions entirely: neither mentions reserving entries, marking them `completed`, nor what happens on failure. This makes the two shorter flows less useful as reference material and inconsistent in detail level with the Full flow.

## Suggestion
Add state transition outcomes to the Regular flow and Externally triggered flow at the same points the Full flow covers them — specifically: when a `CommandEntry` is marked `completed` after event handoff, and when an `EventEntry` is marked `completed` after subscriber delivery. If the intent is for the shorter flows to be deliberately high-level summaries, add a note to that effect and direct readers to the Full flow for the complete state picture.
