---
title: Supported agents
description: The 21 coding agents gglint reads, where each keeps its MCP servers, and how each refers to environment variables.
---

gglint picks up any of these agents on its own, from its config file or its folder at the root of
your repository. A `CLAUDE.md`, `AGENTS.md` or `.cursorrules` file alone doesn't count. Neither
do `.agents/`, `.github/`, `.gemini/`, `.vscode/` or `.zed/` folders without a config file, since
most repositories have those for other reasons. When `--fix` copies a server, it takes it from
the first agent in this table that has it, unless you pick one with `--source`.

| Agent         | `--source`    | MCP config file                     | Agent directory | Format                                     |
| ------------- | ------------- | ----------------------------------- | --------------- | ------------------------------------------ |
| Claude Code   | `claude-code` | `.mcp.json`                         | `.claude/`      | JSON, `mcpServers`                         |
| Codex         | `codex`       | `.codex/config.toml`                | `.codex/`       | TOML, `[mcp_servers.<name>]`               |
| Cursor        | `cursor`      | `.cursor/mcp.json`                  | `.cursor/`      | JSON, `mcpServers`                         |
| OpenCode      | `opencode`    | `opencode.json`, `opencode.jsonc`   | `.opencode/`    | JSON with comments, `mcp.servers` or `mcp` |
| Amp           | `amp`         | `.amp/settings.json`                | `.amp/`         | JSON, `amp.mcpServers` among other settings |
| Antigravity   | `antigravity` | `.agents/mcp_config.json`           | none            | JSON, `mcpServers`                         |
| Continue      | `continue`    | `.continue/mcpServers/`, every YAML and JSON file | `.continue/` | YAML, an `mcpServers` list in each file; or JSON, `mcpServers` |
| Copilot CLI   | `copilot-cli` | `.github/mcp.json`, or `.mcp.json`  | none            | JSON, `mcpServers` or servers at the top   |
| Crush         | `crush`       | `.crush.json`, `crush.json`         | none            | JSON, `mcp`, each with a `type`            |
| Factory Droid | `droid`       | `.factory/mcp.json`                 | `.factory/`     | JSON, `mcpServers`                         |
| Gemini CLI    | `gemini-cli`  | `.gemini/settings.json`             | none            | JSON, `mcpServers` among other settings    |
| Junie         | `junie`       | `.junie/mcp/mcp.json`               | `.junie/`       | JSON, `mcpServers`                         |
| Kilo Code     | `kilo`        | `.kilo/kilo.json`, `kilo.json`, or `.jsonc` | `.kilo/` | JSON with comments, `mcp`                |
| Kimi Code     | `kimi-code`   | `.kimi-code/mcp.json`               | `.kimi-code/`   | JSON, `mcpServers`                         |
| Kiro          | `kiro`        | `.kiro/settings/mcp.json`           | `.kiro/`        | JSON, `mcpServers`                         |
| Mistral Vibe  | `vibe`        | `.vibe/config.toml`                 | `.vibe/`        | TOML, `[[mcp_servers]]` with a `name` each |
| Qoder         | `qoder`       | `.qoder/settings.json`, `.mcp.json` | `.qoder/`       | JSON, `mcpServers` among other settings    |
| Qwen Code     | `qwen-code`   | `.qwen/settings.json`               | `.qwen/`        | JSON, `mcpServers` among other settings    |
| Trae          | `trae`        | `.trae/mcp.json`                    | `.trae/`        | JSON, `mcpServers`                         |
| VS Code       | `vscode`      | `.vscode/mcp.json`                  | none            | JSON, `servers`, each with a `type`        |
| Zed           | `zed`         | `.zed/settings.json`                | none            | JSON with comments, `context_servers` among other settings |


## Environment variables

Every agent writes environment variables its own way. gglint translates between them when it
copies a server or replaces a secret. If an agent has no way to read one, a server that needs it
isn't copied there.

| Agent                                     | Environment variable `KEY`                                                                      |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Claude Code, Amp                          | `${KEY}`                                                                                        |
| Cursor, VS Code                           | `${env:KEY}`                                                                                    |
| OpenCode                                  | `{env:KEY}`                                                                                     |
| Gemini CLI, Qoder, Qwen Code, Crush       | `$KEY` or `${KEY}`, written `${KEY}`                                                            |
| Kiro, Factory Droid                       | `${KEY}`, in environment variables and headers only                                             |
| Continue                                  | `${{ secrets.KEY }}` in a YAML file, read from its `.env` files and then the environment       |
| Codex                                     | `env_vars = ["KEY"]`, `env_http_headers`, or `bearer_token_env_var` for `Authorization: Bearer` |
| Kimi Code                                 | `bearerTokenEnvVar` for `Authorization: Bearer`                                                 |
| Mistral Vibe                              | `api_key_env` for a header                                                                      |
| Kilo Code, Zed                            | None that works in a repository's config, so a server that reads the environment isn't copied to them |
| Antigravity, Copilot CLI, Junie, Trae     | None documented, so a server that reads the environment isn't copied to them                    |

## Agent quirks

**Qoder** reads Claude Code's `.mcp.json` as well as its own file: a server there counts as
Qoder's, and a server missing from both is added to `.mcp.json` only.

**Copilot CLI** reads `.mcp.json` instead of `.github/mcp.json` whenever `.mcp.json` exists, so
it gets the same servers as Claude Code, and gglint warns that `.github/mcp.json` goes unread.
`--fix` creates a `.mcp.json` only if every server in `.github/mcp.json` is written to it.
Copilot also skips a `.github/mcp.json` that lists no servers under `mcpServers` or at its top,
such as one with VS Code's `servers`: gglint warns about it and leaves it alone.

**Crush**'s JSON files are its older format: gglint reads and fixes `.crush.json` or `crush.json`
but never creates one. Crush also reads servers from `.crushrc` or `crushrc`, shell scripts
gglint doesn't read, and gglint says so when one is there. Crush fills in `$VAR` and runs
`$(command)` in every value, so a value with `$` in it isn't copied to Crush.

**Kilo Code** reads no environment variable in a repository's config: though its docs show
`{env:VAR}`, it skips a whole `kilo.json` that has one, with every server in it. So a server
that reads a variable isn't copied to Kilo Code, and for a secret in its file, `gglint secrets`
suggests your own Kilo Code config instead.

**VS Code** can ask for a value once with `${input:id}`, such as a token; a server that uses one
isn't copied to another agent. Zed doesn't fill in environment variables, so a server that
reads one isn't copied to Zed, and a Zed server started by an extension is named but not
compared.

**Continue** reads every YAML and JSON file in `.continue/mcpServers/`. In a YAML file, it fills
in `${{ secrets.NAME }}` from its `.env` files and then the environment; in a JSON file, written
as Claude Code writes it, only `${NAME}` in `env`. `--fix` adds each missing server in a YAML
file of its own, named after the server, and never changes a file that's there.

**Mistral Vibe** writes values as they are, and reads a header from the environment with
`api_key_env`, so a server that reads a variable another way isn't copied to it. It reaches
remote servers over streamable HTTP only, so a server reached over SSE isn't copied to it
either.

**Gemini CLI** loads only the servers listed in `mcp.allowed`, when its settings have that
list, and never those in `mcp.excluded`; gglint reports the servers these lists leave out as
turned off.

**Amp, Codex, Copilot CLI, Gemini CLI, Mistral Vibe, Qoder, Qwen Code and Zed** load a
repository's MCP servers only once you trust the project or approve its servers. **Trae**
reads `.trae/mcp.json` once project MCP is turned on in its settings.

## Files `gglint secrets` reads

On top of the MCP config files above, `gglint secrets` reads these. For a folder, it reads the
Markdown, JSON, TOML and YAML files directly inside it. Personal files are reported by
[`local-file`](/gglint/reference/rules/local-file/) whenever git doesn't ignore them.

| Agent         | Instructions and settings                                                         | Rules, commands and hooks                                                           | Personal files                                                                  |
| ------------- | --------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Claude Code   | `CLAUDE.md`, `.claude/settings.json`                                              | `.claude/commands/`, `.claude/agents/`                                              | `.claude/settings.local.json`, `CLAUDE.local.md`, `.claude/.credentials.json`   |
| Codex         | `AGENTS.md`                                                                       |                                                                                     | `.codex/auth.json`                                                              |
| Cursor        | `.cursorrules`, `.cursor/cli.json`                                                | `.cursor/rules/`, `.cursor/commands/`                                               |                                                                                 |
| OpenCode      | `AGENTS.md`                                                                       | `.opencode/agents/`, `.opencode/agent/`, `.opencode/commands/`, `.opencode/command/` |                                                                                 |
| Amp           | `AGENTS.md`, `AGENT.md`                                                           |                                                                                     |                                                                                 |
| Antigravity   |                                                                                   | `.agents/rules/`, `.agents/workflows/`, `.agent/rules/`, `.agent/workflows/`        |                                                                                 |
| Continue      |                                                                                   | `.continue/mcpServers/`, `.continue/models/`, `.continue/rules/`, `.continue/prompts/` | `.continue/.env`                                                             |
| Copilot CLI   | `.github/copilot-instructions.md`, `AGENTS.md`, `.github/copilot/settings.json`   | `.github/instructions/`, `.github/hooks/`                                           | `.github/copilot/settings.local.json`                                           |
| Crush         | `AGENTS.md`, `CRUSH.md`, `.crushrc`, `crushrc`                                    |                                                                                     |                                                                                 |
| Factory Droid | `AGENTS.md`, `.factory/settings.json`                                             | `.factory/droids/`, `.factory/commands/`                                            | `.factory/settings.local.json`                                                  |
| Gemini CLI    | `GEMINI.md`                                                                       | `.gemini/commands/`                                                                 | `.gemini/.env`                                                                  |
| Junie         |                                                                                   | `.junie/`                                                                           |                                                                                 |
| Kilo Code     | `AGENTS.md`                                                                       |                                                                                     |                                                                                 |
| Kimi Code     | `AGENTS.md`                                                                       |                                                                                     |                                                                                 |
| Kiro          |                                                                                   | `.kiro/steering/`, `.kiro/hooks/`                                                   |                                                                                 |
| Mistral Vibe  | `AGENTS.md`                                                                       | `.vibe/agents/`                                                                     |                                                                                 |
| Qoder         | `AGENTS.md`                                                                       |                                                                                     | `.qoder/settings.local.json`                                                    |
| Qwen Code     | `QWEN.md`                                                                         | `.qwen/commands/`                                                                   | `.qwen/.env`                                                                    |
| Trae          |                                                                                   | `.trae/rules/`                                                                      |                                                                                 |
| VS Code       | `.github/copilot-instructions.md`, `AGENTS.md`                                    | `.github/instructions/`, `.github/prompts/`                                         |                                                                                 |
| Zed           | `.rules`, `AGENTS.md`                                                             |                                                                                     |                                                                                 |

Windsurf, Cline and Goose aren't in the first table because they only read MCP servers from your
home directory, so there's nothing in the repository to compare. `gglint secrets` still reads the
`.windsurfrules` and `.clinerules` files. To add an agent, see
[Contributing](/gglint/project/contributing/).
