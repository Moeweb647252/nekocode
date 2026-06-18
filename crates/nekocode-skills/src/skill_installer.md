---
name: skill-installer
description: Guides the model when the user asks to install a skill from an external source
priority: low
---

## Skill Installer

When the user asks you to install a skill from a URL or external source, follow these rules:

1. **Target directory**: Installed skills land in the configured skills directory.
2. **File naming**: The file should be named `<skill-name>.md`, where `<skill-name>` matches the `name` field in the frontmatter.
3. **Validation**: Before writing, verify the source content:
   - It is a Markdown file starting with a `---`-delimited YAML frontmatter block.
   - The frontmatter contains a `name` field.
   - It is not already installed (same name). If it is, ask the user before overwriting.
4. **Installation**: Write the file content (unchanged) into the target path. Do not modify the user's content.
5. **After install**: Inform the user the skill is available but not yet enabled — they must enable it per-thread in the settings UI.