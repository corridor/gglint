# Changelog

All notable changes to gglint are recorded in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and gglint follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- gglint is on npm, so `npx gglint`, `pnpm dlx gglint` and `bunx gglint` run it, and a
  JavaScript project can pin a version with `npm install --save-dev gglint`. The `gglint`
  package runs the same program as the PyPI wheels, from an optional dependency built for
  each platform, `@gglint/cli-<platform>-<arch>`. It has no install script, so it works with
  pnpm and Bun, which don't run them. Each release publishes it through npm's trusted
  publishing, with provenance.

### Changed

- `gglint hook` in a repository with a `package.json` first adds gglint to the dev
  dependencies with the repository's package manager, npm, pnpm, yarn or bun, and the hook
  runs that copy, so contributors don't need uv. If the package manager fails, the hook isn't
  written, as it would stop every commit. Other repositories' hooks still run `uvx`.

## [0.4.5] - 2026-11-19

gglint now supports the last 7 coding agents from
[#10](https://github.com/corridor/gglint/issues/10), 21 in all, and `gglint secrets` reads
each agent's own settings, rules and personal files. Run over 346 public repositories that
configure the new agents, `gglint mcp --fix` never removed a server and a second run changed
nothing, and `gglint secrets` found 14 secrets written out in 12 of them, with two false
alarms: placeholders left in comments.

### Added

- Five agents, from step 3 of [#10](https://github.com/corridor/gglint/issues/10): Amp
  (`.amp/settings.json`, under `amp.mcpServers`), Crush (`.crush.json` or `crush.json`, under
  `mcp`), Kilo Code (`kilo.json` or `.kilo/kilo.json`, laid out like OpenCode's), VS Code
  (`.vscode/mcp.json`, under `servers`) and Zed (`.zed/settings.json`, under
  `context_servers`). `--fix` writes servers the way each one expects: a `type` on every VS
  Code and Crush server, and `${env:VAR}` in VS Code. It never creates Crush's JSON file, its
  older format, and says when Crush also reads a `crushrc` gglint can't read. Zed fills in no
  environment variables, and Kilo Code skips a repository's whole config file that refers to
  one, though its docs show `{env:VAR}`, so a server that reads one isn't copied to either. A
  Zed server started by an extension is named but not compared. `gglint secrets` reads their
  instruction files too, and Crush's `crushrc`.
- Continue, from step 4 of #10: every YAML and JSON file in `.continue/mcpServers/`. A YAML
  file lists servers under `mcpServers`, with `${{ secrets.NAME }}` read from Continue's `.env`
  files and then the environment; a JSON file is written as Claude Code writes it. `--fix`
  adds each missing server in a YAML file of its own and never changes a file that's there.
  `gglint secrets` reads Continue's models, rules and prompts, and `local-file` covers its
  `.continue/.env`. gglint now bundles `yaml-rust2` to read YAML.
- Mistral Vibe, from step 4 of #10: `.vibe/config.toml`, a list of `[[mcp_servers]]` tables
  with a `name` each. A header read from the environment is written with `api_key_env`, and a
  server reached over SSE isn't copied to it, as it reaches remote servers over streamable
  HTTP only.

- `gglint secrets` reads every supported agent's own files, not only Claude Code's and
  Codex's: settings and hooks that hold commands (`.cursor/cli.json`,
  `.factory/settings.json`, `.github/copilot/settings.json`, `.github/hooks/`, `.kiro/hooks/`)
  and the folders of rules and commands of Cursor, OpenCode, Antigravity, Factory Droid,
  Gemini CLI, Qwen Code and Trae. Each agent's files are listed in its row of the agent table,
  and in the README.
- `local-file` covers the personal files of more agents: `.qoder/settings.local.json`, where
  Qoder saves the commands you allow, `.github/copilot/settings.local.json`,
  `.factory/settings.local.json`, and `.gemini/.env` and `.qwen/.env`, where Gemini CLI's and
  Qwen Code's docs put your API key. GitHub's code search finds 148 committed
  `.qoder/settings.local.json` files, 117 `.gemini/.env` and 33 `.qwen/.env`.

### Changed

- A value with `$(` isn't copied to an agent that fills in `$VAR`: Crush runs the command in
  it.
- The new agents come after Claude Code, Codex, Cursor and OpenCode, in alphabetical order
  with the others.
- A secret in a command an agent saved as allowed names that agent: `/permissions` in Claude
  Code, and the file for another agent. One in an instruction file or MCP config no longer
  mentions Claude Code.
- The `gglint-secrets` pre-commit hook passes these files to gglint too.
- `gglint --help` says gglint keeps secrets out of the agents' files as well as their MCP
  servers in sync.
- The README introduces both checks, with the secrets results of the November 2026 survey,
  and its Quick start runs `gglint secrets` and shows a workflow that checks every pull
  request for secrets.

### Fixed

- `gglint secrets` took `"Authorization": "Bearer $GITHUB_PERSONAL_ACCESS_TOKEN"` for a
  written-out secret, and `--fix` replaced the variable with another one. A reference after
  `Bearer`, `Basic` or `Token` is now read as one, as `$GITHUB_PERSONAL_ACCESS_TOKEN` alone
  was. Gemini CLI, Qwen Code, Qoder and Crush fill in `$VAR`.
- A secret in the MCP config of an agent with no way gglint knows of to read a variable
  there, such as Junie, is no longer advised to be read from one: the advice is to keep the
  server in your own config, or for Kimi Code to name the variable in `bearerTokenEnvVar`.
- On Windows, a file in an agent's folder of rules or commands is named with `/`, as
  `.claude/commands/deploy.md`, like every other file, instead of `.claude/commands\deploy.md`.

## [0.4.0] - 2026-11-18

gglint now keeps secrets out of the files that configure coding agents, as well as keeping
their MCP servers in sync ([#14](https://github.com/corridor/gglint/issues/14)), and runs
before every commit and on every pull request. Run over 441 public repositories, `gglint
secrets` found 7 secrets and 21 committed `.claude/settings.local.json` files, and no false
alarms.

### Added

- `gglint secrets` finds credentials written out in agent config files, settings and
  instruction files: known token formats, values under names such as `token` or `password`,
  passwords in URLs, and secrets in commands, such as those Claude Code saves as allowed.
  With `--fix` it replaces a value with a reference to an environment variable written in
  the agent's own syntax. `--staged` checks the copy of each file that's staged in git, and
  file arguments check only those that are agent files, for pre-commit hooks. A secret in a
  file git ignores is a warning.
- The `local-file` rule fails when an agent file meant for one person's machine is in git:
  `.claude/settings.local.json`, `CLAUDE.local.md`, `.claude/.credentials.json` or
  `.codex/auth.json`. It warns while git doesn't ignore one yet, and `--fix` adds it to
  `.gitignore`.
- `gglint hook` runs the secrets check before every commit: it installs into husky or git's
  `pre-commit` hook, and prints the lines to add for pre-commit, lefthook and lint-staged.
  The repository is also a source of pre-commit hooks (`gglint-secrets`, `gglint-mcp`).
- `--format github`, `sarif` and `markdown` for `mcp` and `secrets`: annotations on the lines
  of a pull request, a SARIF log for Code scanning, and a section for a pull request comment
  that includes a table of servers by agent.
- A GitHub Action (`uses: corridor/gglint@v1`) that annotates a pull request, posts one
  comment that it updates on each push, and optionally uploads to Code scanning. It only
  reports.
- A shared report layer (`report::Diagnostic`) that every check prints through.

## [0.3.0] - 2026-11-16

gglint now supports 14 coding agents instead of 4
([#10](https://github.com/corridor/gglint/issues/10)). Each new agent's format follows its
documentation, and Gemini CLI, Qwen Code, Factory Droid, Qoder and Copilot CLI were also
checked against the agents themselves.

### Added

- Ten agents: Antigravity, GitHub Copilot CLI, Factory Droid, Gemini CLI, Junie, Kimi Code,
  Kiro, Qoder, Qwen Code and Trae. gglint reads each one's MCP config file in the repository,
  and `--fix` writes servers into it the way that agent expects: `httpUrl` for Gemini CLI and
  Qwen Code, `serverUrl` for Antigravity, a `type` for Copilot CLI, Factory Droid and Qoder,
  and `transport` and `bearerTokenEnvVar` for Kimi Code.
- A server turned off with `"disabled": true` (Antigravity, Factory Droid, Kiro, Qoder), with
  `"enabled": false` (Kimi Code), or by Gemini CLI's `mcp.allowed` and `mcp.excluded` lists
  is reported as turned off.
- Qoder reads `.mcp.json` as well as its own file, so a server there counts as Qoder's and
  isn't copied into `.qoder/settings.json`.
- Copilot CLI reads `.mcp.json` instead of `.github/mcp.json` while it exists, and skips a
  `.github/mcp.json` without `mcpServers`. gglint warns about such a file and leaves it
  alone, and `--fix` creates a `.mcp.json` only if Copilot keeps every server it had.
- `gglint --help` lists every agent, with the name `--source` takes and its config file.
  `--source` also accepts other names an agent goes by, such as `gemini`, `copilot` or
  `droid`.

### Changed

- The new agents come after Claude Code, Codex, Cursor and OpenCode, in alphabetical order,
  so `--fix` still copies a server from those four first.
- Antigravity, Copilot CLI and Gemini CLI count only once they have their MCP config file,
  since many repositories have an `.agents`, `.github` or `.gemini` directory for other
  reasons, and `--fix` never creates their config file.
- A server that reads an environment variable isn't copied to an agent whose documentation
  doesn't say how to refer to one: Antigravity, Copilot CLI, Junie and Trae. It's listed
  under `Not added:` instead.
- A reason under `Not added:` quotes the text that can't be written, such as
  `${DB_HOST:-localhost}`, and the warning for a server without a URL names the keys that
  agent reads, such as `serverUrl` for Antigravity.

### Fixed

- A config file that starts with a UTF-8 byte order mark, as some Windows editors save it,
  couldn't be read. It's now read for every agent, and `--fix` keeps the mark.

## [0.2.0] - 2026-11-12

`--fix` is now the only fix flag, and `--source` chooses the agent servers are copied from.
Scripts that pass `--create` or `--overwrite` need to drop them.

### Added

- `--source AGENT` makes one agent the source of truth: each server's setup is taken from
  it wherever it has the server turned on, instead of from the first agent in the order
  Claude Code, Codex, Cursor, OpenCode
  ([#3](https://github.com/corridor/gglint/issues/3)).

### Changed

- `gglint mcp --fix` now does everything `--create` and `--overwrite` did: it also creates
  the config file of an agent that has its own directory, and starts each server with the
  same command and arguments in every agent
  ([#4](https://github.com/corridor/gglint/issues/4)).
- The hints under `missing-config` and `different-server` findings now point to `--fix`.
- The README shows one way to run gglint, `uvx gglint`, and every block of commands is a
  sequence of steps to run in order, never a list of alternatives.
- The README's screenshot is replaced by a terminal graphic. Opened on its own, hovering a
  server, an agent, a cell or a problem shows a card with the details. Below it, a table
  explains its servers, agents and cells on hover, on GitHub and on PyPI.

### Removed

- `--create` and `--overwrite`. `--fix` does what they did, and passing either is now an
  unknown option.

## [0.1.1] - 2026-11-12

### Added

- `gglint mcp --fix --overwrite` makes each server start with the same command and
  arguments in every agent, copied from the first agent where it's turned on. Environment
  variables, headers, the on/off state and keys only one agent has are kept.
- A `different-server` finding for another version or other arguments now says that
  `--overwrite` fixes it.

### Fixed

- The `missing-config` message for a single server read "gets none of the MCP server". It
  now reads "doesn't get the MCP server".

### Changed

- The README leads with `uvx gglint mcp`, which runs gglint without installing it.

## [0.1.0] - 2026-11-11

First release.

### Added

- `gglint mcp` reads the MCP config files of Claude Code (`.mcp.json`), Codex
  (`.codex/config.toml`), Cursor (`.cursor/mcp.json`) and OpenCode (`opencode.json`,
  `opencode.jsonc`), draws each server in each agent as a table, and reports four kinds of
  problem: `missing-server`, `missing-config`, `different-server` and `renamed-server`.
- `gglint mcp --fix` adds the servers missing from existing config files, written in each
  agent's own format, and keeps comments and formatting. It never changes or removes an
  existing entry.
- `gglint mcp --fix --create` also creates the config file of an agent that has its own
  directory but no MCP config file.
- `--format lines` prints one problem per line, and `--servers` lists every configured
  server without values of environment variables or headers.
- Wheels for Linux (x86_64 and aarch64, glibc and musl), macOS (Intel and Apple silicon) and
  Windows (x64 and ARM64).

[0.4.5]: https://github.com/corridor/gglint/releases/tag/v0.4.5
[0.4.0]: https://github.com/corridor/gglint/releases/tag/v0.4.0
[0.3.0]: https://github.com/corridor/gglint/releases/tag/v0.3.0
[0.2.0]: https://github.com/corridor/gglint/releases/tag/v0.2.0
[0.1.1]: https://github.com/corridor/gglint/releases/tag/v0.1.1
[0.1.0]: https://github.com/corridor/gglint/releases/tag/v0.1.0
