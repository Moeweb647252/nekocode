---
name: skill-installer
description: Guides the model when the user asks to install a skill from an external source such as a URL or another skill registry.
---

## Skill Installer

When the user asks to install a skill from a URL or external source, install
it as a spec-compliant skill directory under the configured skills directory
(`~/.config/nekocode/skills/`).

### 1. Target layout

The skill must be a **directory** containing at minimum a `SKILL.md`:

```
~/.config/nekocode/skills/<skill-name>/
├── SKILL.md
├── scripts/        # if present
├── references/     # if present
└── assets/         # if present
```

Recreate the full subtree (do not flatten it into a single `.md` file).

### 2. Validation before writing

Before writing, verify the source content:

- It is a directory containing a `SKILL.md`.
- The `SKILL.md` starts with a `---`-delimited YAML frontmatter block.
- The frontmatter contains a `name` field and a `description` field (both
  required by the spec).
- The `name` field matches the directory's basename and follows the naming
  rules: 1-64 chars, lowercase letters/digits/hyphens only, no leading or
  trailing hyphen, no consecutive hyphens.
- A skill with the same name is not already installed. If it is, ask the
  user before overwriting.

### 3. Installation

- Write the directory tree unchanged into
  `~/.config/nekocode/skills/<skill-name>/`. Do not modify the user's
  content.
- On unix, ensure shell scripts under `scripts/` have the executable bit
  set (`chmod +x`) if the source intended them to be runnable directly.

### 4. After install

Inform the user the skill is available but not yet enabled — they must
enable it per-thread in the thread's settings dialog. Mention that any
`compatibility` field in the skill's frontmatter describes environment
requirements they may need to satisfy (e.g. Python version, system
packages).
