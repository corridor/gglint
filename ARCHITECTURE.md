# Architecture

This document is a map of gglint's code: what each part does, how a run flows through them,
and where to start for the most common changes. It describes the shape of the code, not every
function; the module comments at the top of each file go further.

## A run, end to end

```text
main.rs ── parses the command line
   │
   ├─ gglint mcp ──────► harness::read_mcp      read every agent's MCP config file
   │                     harness::check_mcp     compare them: missing, different, renamed
   │                     harness::fix_mcp       (--fix) write the missing servers and files
   │
   ├─ gglint secrets ──► secrets::scan_files    read the agent files, find secrets
   │                     secrets::apply_fixes   (--fix) replace them with references
   │
   ├─ gglint hook ─────► hook.rs                install or print the pre-commit hook
   │
   ├─ gglint watch ────► watch::launch          start the agent, its base URL pointed at gglint
   │                     watch::proxy           record each request and pass it on; serve the page
   │
   └─ every check ─────► report.rs              Diagnostics → table, lines, github,
                                                sarif or markdown; the exit status
```

Every check turns what it finds into `report::Diagnostic`s, so the output formats, the colours
and the exit status are the same for all of them.

## Modules

| File                     | What it does                                                                                 |
| ------------------------ | -------------------------------------------------------------------------------------------- |
| `src/main.rs`            | The `gglint` command: parses arguments, runs a check, draws the `table` format.              |
| `src/lib.rs`             | Exposes the modules to the tests.                                                            |
| `src/harness/agents.rs`  | **Everything gglint knows about each agent, in one table** (`AGENTS`): its MCP files, directory, format, environment-variable syntax, how it writes remote servers and turns servers off, and the settings, rules and personal files `gglint secrets` reads. |
| `src/harness/mod.rs`     | The `Harness` enum and `HARNESSES`, in the order agents are reported and fixed.             |
| `src/harness/mcp.rs`     | Reads each agent's MCP config files, JSON, TOML or YAML, into `McpConfig`s and `McpServer`s. Parses only: nothing is started, fetched or expanded. |
| `src/harness/compare.rs` | Matches servers across agents, by name and then by what they run, and finds the `missing-server`, `missing-config`, `different-server` and `renamed-server` problems. |
| `src/harness/fix.rs`     | `--fix`: writes each server in each agent's format, editing JSON with `jsonc-parser` and TOML with `toml_edit` so comments and formatting are kept. |
| `src/secrets.rs`         | `gglint secrets`: which files are agent files, the token formats, the names that hold credentials, commands, URLs, placeholders, and the fixes. Never keeps or prints a secret's value. |
| `src/report.rs`          | `Diagnostic` and `Severity`, the `lines`, `github`, `sarif` and `markdown` formats, colours, and the exit status. |
| `src/watch/mod.rs`       | `gglint watch`: how each agent is started so its requests come to gglint (`ANTHROPIC_BASE_URL`, a Codex model provider, `HTTPS_PROXY`), and where recordings go. |
| `src/watch/proxy.rs`     | The local server: forwards model requests to the real API and records both sides, passes `CONNECT` tunnels through recording only the host and size, and serves the viewer. |
| `src/watch/record.rs`    | The recording, `events.jsonl`: removes credential headers, masks secrets in bodies with `secrets::find_secrets`, and notes email addresses. |
| `src/watch/viewer.html`  | The page, built into the binary: splits each request into system prompt, tools, injected context, history and your prompt, and lists every host. |
| `src/hook.rs`            | `gglint hook`: detects husky, git, pre-commit, lefthook or lint-staged, and installs or prints the hook. In a repository with a `package.json`, adds `@gglint/cli` to the dev dependencies with its package manager, and the hook runs that copy. |
| `npm/`                   | The npm package `@gglint/cli`: `gglint.js` runs the program from the platform's optional dependency, and `build.py` builds the packages from the release's wheels. |
| `pre-commit/`            | `build.py` writes [gglint-pre-commit](https://github.com/corridor/gglint-pre-commit), where pre-commit gets gglint's hooks without uv: `.pre-commit-hooks.yaml` started as `gglint`, and a `pyproject.toml` that installs gglint from PyPI. A workflow there runs it for each release. |
| `action.yml`, `scripts/` | The GitHub Action: a composite action that runs gglint with uv and posts the comment.        |
| `.pre-commit-hooks.yaml` | The hooks gglint provides to the pre-commit framework, with uvx; gglint-pre-commit's are written from them. |
| `docs/`                  | The documentation site, built with Starlight. See `docs/README.md`.                         |

## Rules the code keeps

- **Read-only unless `--fix`.** Only `fix.rs`, `secrets::apply_fixes`,
  `secrets::ignore_local_files` and `hook.rs` write files.
- **Nothing is executed or fetched.** The only program gglint runs is `git`, except in
  `gglint watch`, which starts the agent it's asked to and forwards that agent's requests.
- **No credential is recorded.** `watch::record` drops credential headers and masks secrets
  before anything is written or shown.
- **A secret's value never leaves `secrets.rs`.** A `Diagnostic`'s message names the kind of
  secret, the key it's under and where it is.
- **`--fix` never removes a server**, and running it twice changes nothing. `tests/corpus.rs`
  checks this against real repositories.
- **Fast.** Each agent's files are at fixed paths, so nothing is searched for.
  `tests/latency.rs` holds `gglint mcp` to its time budget.

## Common changes

### Supporting another agent

1. Add it to the `Harness` enum and `HARNESSES` in `src/harness/mod.rs`.
2. Add its row to `AGENTS` in `src/harness/agents.rs`, from the agent's own documentation:
   its config files, directory, format and environment-variable syntax.
3. If its format is new, teach `mcp.rs` to read it and `fix.rs` to write it.
4. Add tests to `tests/agents.rs`: reading its file, and `--fix` writing it.
5. In the same row, list the files `gglint secrets` reads for it: its settings, rules and
   personal files. Test them in `tests/secrets.rs`.
6. Add it to the *Supported agents* and *Files it reads* tables in `README.md`
   (`tests/readme.rs` checks the first) and in `docs/src/content/docs/reference/agents.md`, to
   the pre-commit hooks' `files` patterns in `.pre-commit-hooks.yaml`, and to `CHANGELOG.md`.

### Adding a rule

1. Add the variant to `Rule` in `src/harness/compare.rs` for an MCP rule, or produce a
   `Diagnostic` with the new rule name in `src/secrets.rs`.
2. Test it in `tests/mcp.rs` or `tests/secrets.rs`.
3. Document it: a page in `docs/src/content/docs/reference/rules/`, a row in the rules
   overview, the sidebar in `docs/astro.config.mjs`, and the README.

## Tests

| File                | What it covers                                                                  |
| ------------------- | ------------------------------------------------------------------------------- |
| `tests/mcp.rs`      | Reading, comparing and fixing MCP servers.                                      |
| `tests/agents.rs`   | Every agent beyond the first four: reading and writing each one's format.       |
| `tests/secrets.rs`  | `gglint secrets`, including that no secret's value is printed.                  |
| `tests/cli.rs`      | The command line: arguments, exit status, output.                               |
| `tests/formats.rs`  | The `github`, `sarif` and `markdown` formats, and the GitHub Action's script.   |
| `tests/latency.rs`  | The time budget for `gglint mcp`.                                               |
| `tests/readme.rs`   | The README lists every supported agent with its config file.                    |
| `tests/corpus.rs`   | `--fix` over a directory of real repositories; ignored unless `GGLINT_CORPUS` is set. |
