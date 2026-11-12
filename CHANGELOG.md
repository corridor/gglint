# Changelog

All notable changes to gglint are recorded in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and gglint follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- `gglint mcp --fix` now does everything `--create` and `--overwrite` did: it also creates
  the config file of an agent that has its own directory, and starts each server with the
  same command and arguments in every agent
  ([#4](https://github.com/corridor/gglint/issues/4)).
- `--create` and `--overwrite` are no longer needed. They're still accepted, print a note,
  and do what `--fix` does.

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

[0.1.1]: https://github.com/corridor/gglint/releases/tag/v0.1.1
[0.1.0]: https://github.com/corridor/gglint/releases/tag/v0.1.0
