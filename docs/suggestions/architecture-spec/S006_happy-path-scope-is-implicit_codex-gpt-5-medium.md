# S006 - Happy-path scope is implicit

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | medium                                |
| File                     | `docs/architecture-spec.md`           |
| Decision                 | accepted                              |
| Implementation reference | b96b6fc                               |
| Created at               | 2026-04-14                            |
| Author                   | Codex, gpt-5, medium                  |
| Reviewer                 |                                       |

## Issue
The introduction says this document describes the flows "at a detailed, step-by-step level", but every flow currently shows only the success path. Failure handling, retries, rejection points, and sweep-driven reprocessing are all absent, even though those behaviors are core to the architecture and are documented in `components.md` and `contracts.md`. As written, a reader can easily infer that entries simply progress linearly from `received` to `completed` with no operational branches.

## Suggestion
Make the scope explicit. Either add a short note near the introduction or the `## Flows` heading stating that these are happy-path sequences only, with failure and retry behavior defined in `components.md` and `contracts.md`, or expand each flow with the main non-happy-path branches (for example failed handoff, retry scheduling, and stale reservation release). The lighter-weight option is probably enough, but the document should stop implying that the listed steps are the whole lifecycle.
