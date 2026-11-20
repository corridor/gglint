---
title: Limitations
description: What gglint doesn't read or check yet.
---

## MCP servers

- Servers added by Claude Code plugins, enabled in `.claude/settings.json`, aren't read. A
  server provided by a plugin can be reported as missing from Claude Code.
- User-level settings such as `~/.claude.json` or `~/.codex/config.toml` aren't read.
- Only config files at the repository's root are read, so each package of a monorepo is
  checked separately.
- Windsurf, Cline and Goose read MCP servers only from the user's home directory, so there's
  nothing in a repository to compare.
- Amp, Codex, Copilot CLI, Gemini CLI, Mistral Vibe, Qoder, Qwen Code and Zed load a
  repository's MCP servers only once you trust the project or approve its servers, and Trae
  once project MCP is turned on in its settings. gglint checks the files either way.
- Gemini CLI loads only the servers listed in `mcp.allowed`, when its settings have that list.
  `--fix` doesn't edit the list, so a server it adds while `mcp.allowed` is set stays off until
  you add it there.

## Secrets

- A password made of one kind of letters or shorter than 8 characters, and in a command one
  with no digit or symbol, isn't found. See [How secrets are found](/gglint/concepts/secrets/).
- `.env` files, other source and git history aren't read. An agent's own `.env` file is
  reported once it's in git, but not read.
- `gglint secrets --fix` can't change a value for an agent that has no way to refer to an
  environment variable, such as Codex in `http_headers`; it reports it and says what to use
  instead.
