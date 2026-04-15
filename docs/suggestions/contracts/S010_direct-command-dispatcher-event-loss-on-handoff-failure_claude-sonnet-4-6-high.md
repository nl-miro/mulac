# S010 - Direct Command Dispatcher event loss on handoff failure

| Field                    | Value                                |
|--------------------------|--------------------------------------|
| Priority                 | high                                 |
| File                     | `docs/contracts.md`                  |
| Decision                 | accepted                             |
| Implementation reference | d2f2de9                              |
| Created at               | 2026-04-14                           |
| Author                   | Claude Code, claude-sonnet-4-6, high |
| Reviewer                 |                                      |

## Issue
The Direct Command Dispatcher guarantee states "the command is executed at most once; if the handler fails the error is propagated to the caller". This covers handler failure, but not the case where the handler succeeds yet the subsequent Event Dispatcher handoff fails. In Direct mode there is no `CommandEntry` to retry, so the handler has already run and produced events that can no longer be handed off. The `Requires from collaborators` section only addresses this availability requirement for TwoPhased ("if it is not [available], TwoPhased retries the `CommandEntry`"), leaving the Direct variant's handoff failure behaviour undefined for callers.

## Suggestion
Add a guarantee bullet for the Direct variant that explicitly states what happens when Event Dispatcher handoff fails after the handler has already executed: either the error is propagated to the caller with no retry (events are lost and the caller is responsible for recovery), or some other defined behaviour. If the error does propagate, note that the handler has already run and its side-effects are not rolled back, mirroring the existing partial-delivery hazard note on the Event Dispatcher.
