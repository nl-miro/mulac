# Commit Review Template

## Commit Message

**`<commit message>`**

Evaluate:
- Does it follow the project's conventional commits format (`type(scope): description`)?
- Is it accurate — does it reflect the actual scope of the changes?
- Is it concise without losing important context?

---

## Content Review

**What changed** — Summarise the diff: what was added, modified, or removed.

**What's done well** — Call out good decisions: structure, naming, consistency with existing patterns, important design choices.

**Issues:**

Number each issue and indicate severity:

1. **High: <description>** — Incorrect behaviour, missing invariant, or contradiction with existing documented rules. Should be fixed before merging.
2. **Medium: <description>** — Gap in coverage, ambiguous wording, or inconsistency that could cause confusion during implementation. Worth addressing.
3. **Low: <description>** — Cosmetic, formatting, or minor wording improvements. Fix if convenient.

For each issue, explain *why* it matters and suggest a direction when possible.

---

**Overall:** One or two sentences summarising the quality of the commit and highlighting the most important action item, if any.
