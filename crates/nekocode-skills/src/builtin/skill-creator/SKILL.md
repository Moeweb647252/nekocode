---
name: skill-creator
description: Guides the model when the user asks to create or edit a skill. Use when writing or modifying a SKILL.md file or building a skill directory.
---

## Skill Creator

When the user asks you to create a new skill or edit an existing one, follow the
[Agent Skills specification](https://agentskills.io/specification).

### 1. Skill layout

A skill is a **directory** under the configured skills directory
(`~/.config/nekocode/skills/`):

```
~/.config/nekocode/skills/<skill-name>/
├── SKILL.md          # required: YAML frontmatter + Markdown body
├── scripts/          # optional: executable code the model can run
├── references/       # optional: extra documentation the model can read
└── assets/           # optional: templates, data files, images
```

The directory name **must match** the `name` field in `SKILL.md`. Loose
single-file skills are not supported — always use the directory layout.

### 2. SKILL.md frontmatter

Only the fields defined by the spec are recognized.

```yaml
---
name: skill-name            # required, 1-64 chars, [a-z0-9-], matches dir name
description: One-line summary of what the skill does and when to use it. # required, 1-1024 chars
license: Apache-2.0         # optional
compatibility: Requires Python 3.14+ # optional, 1-500 chars
metadata:                   # optional, string→string map
  author: example-org
  version: "1.0"
allowed-tools: Bash(git:*) Read # optional, space-separated (experimental)
---
```

### 3. Field rules

- `name`: required. Lowercase letters, digits, and hyphens only. 1-64 chars.
  Must not start or end with a hyphen, must not contain consecutive hyphens,
  and must match the parent directory name.
- `description`: required. 1-1024 chars. Describe both what the skill does
  and when to use it, including trigger keywords.
- `license`, `compatibility`, `metadata`, `allowed-tools`: all optional.
- Do not invent new top-level fields. Anything not in the spec is ignored by
  other agentskills-compatible tools.

### 4. Markdown body

After the closing `---`, write the skill instructions in clear, actionable
Markdown. This body is **not** loaded automatically — it is fetched on demand
by the agent via the `read_skill` tool when the skill looks relevant to the
task. Reference bundled files via relative paths from the skill root, e.g.
`references/REFERENCE.md` or `scripts/extract.py`, one level deep.

### 5. Scripts

- Place executable code under `scripts/`. The agent reads these files via
  `read_skill_file` and runs them using the shell tools available in the
  session (e.g. `bash scripts/validate.sh "$INPUT"`).
- Scripts should be self-contained or document their dependencies inline
  (e.g. PEP 723 for Python, `deno run` for TypeScript).
- Scripts must not block on interactive prompts — agents run in
  non-interactive shells and a blocking prompt will hang indefinitely.

### 6. After creation

Tell the user the skill is available but not yet enabled — they must enable
it per-thread in the thread's settings dialog before it appears in the
catalog.
