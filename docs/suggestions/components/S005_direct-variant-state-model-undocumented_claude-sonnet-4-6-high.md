# S005 - Direct variant state model is left undocumented

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | low                                   |
| File                     | `docs/components.md`                  |
| Decision                 | accepted                              |
| Implementation reference | abf59e4                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 | Codex, gpt-5, medium                  |

## Issue
The States sections for both Command Dispatcher and Event Dispatcher only address the TwoPhased variant:

> "Use `CommandEntry` lifecycle states when using the TwoPhased variant (`received/reserved/completed/failed/dead`)."

The Direct variant is not mentioned at all. A reader consulting the States section to understand what states exist in the Direct variant gets no answer. The omission can be read as an oversight rather than a deliberate design property.

## Suggestion
Add an explicit statement that the Direct variant has no persistent state model — execution is in-flight only. For example:

> The Direct variant has no persistent state model; execution is synchronous and in-flight only.
> Use `CommandEntry` lifecycle states when using the TwoPhased variant (`received/reserved/completed/failed/dead`).

The same change applies to the Event Dispatcher States section, replacing `CommandEntry` with `EventEntry`.
