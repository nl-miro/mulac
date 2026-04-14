# S006 - Model section conflates Direct and TwoPhased models without labeling

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | low                                   |
| File                     | `docs/components.md`                  |
| Decision                 | accepted                              |
| Implementation reference | d8be889                               |
| Created at               | 2026-04-14                            |
| Author                   | Claude Code, claude-sonnet-4-6, high  |
| Reviewer                 | Codex, gpt-5, medium                  |

## Issue
The Model sections for Command Dispatcher and Event Dispatcher list both models together without indicating which applies to which variant:

> "The main models are `CommandEnvelope` (in-flight command + metadata) and `CommandEntry` (durable queued form used for retries)."

`CommandEnvelope` is used by both variants. `CommandEntry` is exclusive to TwoPhased. The same pattern holds for `EventEnvelope` and `EventEntry` in the Event Dispatcher. A reader building a system with only the Direct variant will wonder why `CommandEntry` appears in the model description, and may not realise it is irrelevant to their use case.

## Suggestion
Label each model by the variant it belongs to. For example:

> **Both variants:** `CommandEnvelope` — in-flight command + metadata.
> **TwoPhased only:** `CommandEntry` — durable queued form used for retries.

Apply the equivalent labeling to `EventEnvelope` / `EventEntry` in the Event Dispatcher Model section.
