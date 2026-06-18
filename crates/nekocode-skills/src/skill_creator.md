---
name: skill-creator
description: Guides the model when the user asks to create or edit a SKILL.md file
priority: low
---

## Skill Creator

When the user asks you to create a new skill or edit an existing one, follow these rules:

1. **Skill file location**: SKILL.md files live in the configured skills directory (`~/.config/nekocode/skills/`).
2. **File format**: Each skill is a single Markdown file with YAML frontmatter:
   ```markdown
   ---
   name: skill-name
   description: One-line summary
   trigger: optional|keyword|pattern
   priority: medium
   ---
   # Skill Name
   Prompt body...
   ```
3. **Frontmatter rules**:
   - `name` is required and must be unique across all skill files.
   - `description` is optional but recommended.
   - `trigger` is an optional pipe-delimited keyword pattern for auto-matching.
   - `priority` must be one of `high`, `medium`, or `low` (default `medium`).
4. **The frontmatter block is delimited by `---` lines** (exactly three dashes).
5. **After frontmatter**, the rest of the file is the prompt body — this is what gets injected into the system prompt verbatim.
6. **The prompt body** should be clear, actionable behavioral instructions in conversational tone.