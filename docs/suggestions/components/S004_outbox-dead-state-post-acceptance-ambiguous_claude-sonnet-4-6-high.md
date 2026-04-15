# S004 - Outbox dead state "post-acceptance" qualifier is ambiguous

| Field                    | Value                                |
|--------------------------|--------------------------------------|
| Priority                 | medium                               |
| File                     | `docs/components.md`                 |
| Decision                 | accepted                             |
| Implementation reference | 2f29b00                              |
| Created at               | 2026-04-14                           |
| Author                   | Claude Code, claude-sonnet-4-6, high |
| Reviewer                 | Codex, gpt-5, medium                 |

## Issue
The `dead` state description (state 5) reads: "the retry policy was exhausted, or a non-retriable **post-acceptance** transformation error occurred." The word "acceptance" is already used in Rule 3 with a specific meaning — broker acceptance ("If broker acceptance succeeds but marking `completed` fails…"). Serialisation and other transformation failures occur *before* broker publication, not after, so "post-acceptance" points in the wrong direction. A reader who has just read Rule 3 will interpret "post-acceptance" as meaning after the broker acknowledged the message, which makes the phrase incoherent.

## Suggestion
Replace "post-acceptance transformation error" in the dead state description with "transformation failure" to match the terminology used in Rule 4 ("non-retriable transformation failures (such as serialisation errors)"). The resulting description would read:

> `dead` — the retry policy was exhausted, or a non-retriable transformation failure occurred; the entry will not be retried automatically.

This eliminates the ambiguity without changing the meaning and makes the state description consistent with the rule that explains it.
