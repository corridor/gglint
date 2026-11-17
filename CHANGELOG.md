# Changelog

All notable changes to gglint are recorded in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and gglint follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `gglint secrets` finds credentials written out in agent config files, settings and
  instruction files, and with `--fix` replaces them with a reference to an environment
  variable written in the agent's own syntax. `--staged` checks only what is staged in git,
  and file arguments check only those files, for pre-commit hooks.
- `gglint hook` runs the secrets check before every commit: it installs into husky or git's
  `pre-commit` hook, and prints the lines to add for pre-commit, lefthook and lint-staged.
  The repository is also a source of pre-commit hooks (`gglint-secrets`, `gglint-mcp`).
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

[0.3.0]: https://github.com/corridor/gglint/releases/tag/v0.3.0
[0.2.0]: https://github.com/corridor/gglint/releases/tag/v0.2.0
[0.1.1]: https://github.com/corridor/gglint/releases/tag/v0.1.1
[0.1.0]: https://github.com/corridor/gglint/releases/tag/v0.1.0
