# S002 - Event Dispatcher retry scope ambiguous

| Field                    | Value                                 |
|--------------------------|---------------------------------------|
| Priority                 | medium                                |
| File                     | `docs/components.md`                  |
| Decision                 | accepted                              |
| Implementation reference | 20f95c8                               |
| Created at               | 2026-04-14                            |
| Author                   | Codex, gpt-5, medium                  |
| Reviewer                 |                                       |

## Issue
The Event Dispatcher rules currently collapse two different failure models into one sentence: "Direct dispatch can observe partial delivery if one subscriber fails after others have succeeded; retries can re-invoke already-successful subscribers." In the Direct variant, retries are not owned by the dispatcher and happen only if the caller chooses to retry after the propagated error. In the TwoPhased variant, the dispatcher's own retry loop can automatically re-invoke already-successful subscribers. Because the sentence does not separate those cases, readers can misread automatic retries as part of Direct dispatch.

## Suggestion
Split the rule into Direct and `TwoPhased` bullets, or otherwise make retry ownership explicit. The Direct note should focus on partial delivery plus caller-responsible retry. The `TwoPhased` note should describe the automatic retry hazard that can re-invoke subscribers that already succeeded.
