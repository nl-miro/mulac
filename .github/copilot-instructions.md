# GitHub Copilot Instructions

Follow the guidelines in the root AGENTS.md file for this repository.

## Key Requirements

### Conventional Commits
All commits must follow the [Conventional Commits](https://www.conventionalcommits.org/) format.
Format: `<type>(<scope>): <subject>`

Examples:
- `feat(kernel): add event handler initialization`
- `fix(inbox): resolve synchronization timing`
- `docs(dx): clarify llm attribution format`

### LLM Attribution
If you contribute substantively to code or documentation, use the appropriate trailer:
- **Author**: `Author: copilot::gpt-4::medium` (when you wrote the change)
- **CoAuthor**: `CoAuthor: copilot::gpt-4::medium` (when you helped write it)
- **Committer**: `Committer: copilot::gpt-4::medium` (when you only created the commit)

Use effort levels: `low`, `medium`, or `high`

See AGENTS.md for complete attribution rules.

## Allowed Commands

The following git commands may be run without confirmation:
- `git add *`
- `git commit *`
- `git status`
- `git diff`
- `git log *`
