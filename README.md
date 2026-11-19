# gglint

[![CI](https://github.com/corridor/gglint/actions/workflows/ci.yml/badge.svg)](https://github.com/corridor/gglint/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/gglint.svg)](https://pypi.org/project/gglint/)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/corridor/gglint/blob/main/LICENSE)

Keep every coding agent in your repository in sync, and keep secrets out of their config
files.

Claude Code, Codex, Cursor, Gemini CLI, VS Code and sixteen other coding agents are each set
up by their own files in a repository (see [Supported agents](#supported-agents)). gglint
checks those files for two problems:

- **MCP servers that drift apart.** Each agent reads MCP servers from its own config file, in
  its own format. In a repository set up for more than one of them, a server is added to
  `.mcp.json` but not to `.cursor/mcp.json`, a version is pinned in one file and not the
  other, or an agent has no MCP config at all. `gglint mcp` finds every difference and fixes
  it, written in each agent's own format.
- **Secrets written into agent files.** Approving a command in Claude Code saves it, keys and
  all, in `.claude/settings.local.json`, and tokens get pasted into `.mcp.json`,
  `.codex/config.toml` and `CLAUDE.md`. `gglint secrets` finds them, replaces the ones it can
  with environment variables, and stops the agents' personal files from being committed.

<p align="center">
  <a href="https://raw.githubusercontent.com/corridor/gglint/main/docs/demo.svg"><img src="https://raw.githubusercontent.com/corridor/gglint/main/docs/demo.svg" alt="A terminal running uvx gglint mcp: it draws each MCP server in Claude Code, Codex, Cursor and OpenCode as a table, then lists six problems: missing servers, a server set up differently, and an agent without a config file" width="100%"></a>
</p>

The terminal shows `gglint mcp` on a repository whose agents have drifted apart. Click it to
open it on its own, then hover over a server, an agent, a cell or a problem to see what
gglint found there and what `--fix` does about it. The same details are in this table, on
hover:

<table>
<tr><th>Server</th><th><a href="#what-gglint-checks" title="Claude Code reads .mcp.json">Claude Code</a></th><th><a href="#what-gglint-checks" title="Codex reads .codex/config.toml">Codex</a></th><th><a href="#what-gglint-checks" title="Cursor reads .cursor/mcp.json">Cursor</a></th><th><a href="#what-gglint-checks" title="OpenCode has a .opencode directory but no opencode.json">OpenCode</a></th></tr>
<tr><td><a href="#what-gglint-checks" title="https://api.githubcopilot.com/mcp/, with an Authorization header"><code>github</code></a></td><td align="center"><a href="#what-gglint-checks" title="Set up in .mcp.json:3">✓</a></td><td align="center"><a href="#what-gglint-checks" title="Missing from Codex. --fix adds it to .codex/config.toml, as set up in .mcp.json:3">✗</a></td><td align="center"><a href="#what-gglint-checks" title="Missing from Cursor. --fix adds it to .cursor/mcp.json, as set up in .mcp.json:3">✗</a></td><td align="center"><a href="#what-gglint-checks" title="OpenCode has no opencode.json. --fix creates it with the 3 MCP servers set up for Claude Code and Cursor.">✗</a></td></tr>
<tr><td><a href="#what-gglint-checks" title="npx -y @playwright/mcp@0.0.78 --headless in Claude Code; npx -y @playwright/mcp@latest in Cursor"><code>playwright</code></a></td><td align="center"><a href="#what-gglint-checks" title="Set up in .mcp.json:8: npx -y @playwright/mcp@0.0.78 --headless">✓</a></td><td align="center"><a href="#what-gglint-checks" title="Missing from Codex. --fix adds it to .codex/config.toml, as set up in .mcp.json:8">✗</a></td><td align="center"><a href="#what-gglint-checks" title="Set up differently in .cursor/mcp.json:3: the newest version, without --headless. --fix starts it as in .mcp.json:8">≠</a></td><td align="center"><a href="#what-gglint-checks" title="OpenCode has no opencode.json. --fix creates it with the 3 MCP servers set up for Claude Code and Cursor.">✗</a></td></tr>
<tr><td><a href="#what-gglint-checks" title="https://mcp.linear.app/sse"><code>linear</code></a></td><td align="center"><a href="#what-gglint-checks" title="Missing from Claude Code. --fix adds it to .mcp.json, as set up in .cursor/mcp.json:7">✗</a></td><td align="center"><a href="#what-gglint-checks" title="Missing from Codex. --fix adds it to .codex/config.toml, as set up in .cursor/mcp.json:7">✗</a></td><td align="center"><a href="#what-gglint-checks" title="Set up in .cursor/mcp.json:7">✓</a></td><td align="center"><a href="#what-gglint-checks" title="OpenCode has no opencode.json. --fix creates it with the 3 MCP servers set up for Claude Code and Cursor.">✗</a></td></tr>
</table>

| Command | What it does |
| ------- | ------------ |
| [`gglint mcp`](#usage) | Compares the MCP servers of every coding agent, and fixes the differences. |
| [`gglint secrets`](#finding-secrets) | Finds credentials written out in agent config and instruction files, and replaces them with environment variables. |
| [GitHub Action](#github-action) | Runs both on every pull request: annotations on the diff, a comment, and Code scanning. It only reports. |
| [`gglint hook`](#before-every-commit) | Runs the secrets check before every commit, through husky, git, pre-commit, lefthook or lint-staged. |

- **One view of every agent.** A table shows which agent gets which server, and how each
  one differs.
- **One fix.** `--fix` adds the missing servers and config files and starts each server the
  same way in every agent. It never removes a server, and keeps your comments and formatting.
- **Each agent's own format.** A server is translated between JSON and TOML, between each
  agent's syntax for environment variables, and between local and remote server types.
- **Secrets in the forms they leak in.** Tokens in known formats, passwords in database URLs,
  keys in the commands agents save as allowed, and values under names such as `password` or
  `Authorization`, in every supported agent's config, settings, hooks, rules and instructions.
  A secret's value is never printed.
- **Personal files stay personal.** Committing `.claude/settings.local.json`,
  `CLAUDE.local.md`, another agent's personal settings, or an agent's login or `.env` file
  fails the check, whatever is in it.
- **Fast and read-only by default.** gglint reads a few files at fixed paths and typically
  finishes in under a millisecond. It never starts a server or downloads anything.
- **Built for CI.** It exits with status 1 when it finds a problem, prints annotations for a
  pull request or a SARIF log, and runs as a [GitHub Action](#github-action).

In November 2026 we checked the 258 most-starred GitHub repositories set up for two or more
of Claude Code, Codex, Cursor and OpenCode. Of the 179 that configure MCP servers, 128 (72%)
had at least one of the problems gglint reports. Among the 166 active repositories with 30 or
more stars that set up a newer agent, such as Gemini CLI, Kiro or Junie, 83 of the 162 with
MCP servers (51%) had at least one problem.

The same month we ran `gglint secrets` over 441 public repositories set up for coding agents:
21 had committed `.claude/settings.local.json`, and 5 had 7 secrets that look real in their
agent files, 2 of them inside commands saved in `.claude/settings.json`. Across all of public
GitHub, GitGuardian counted about 31,000 secrets in `.claude/settings.local.json` files
([report](https://blog.gitguardian.com/claude-code-settings-local-json-secrets-docker-hub/)).

## Quick start

gglint runs with `uvx`, which comes with [uv](https://docs.astral.sh/uv/). If `uvx` isn't on
your machine, install uv with its
[installation guide](https://docs.astral.sh/uv/getting-started/installation/) first.

From the root of a repository, run both checks:

```sh
uvx gglint mcp
uvx gglint secrets
```

Each reports what it finds. To fix what it can, run each one again with `--fix`, then
`git diff` to see what changed. `uvx` downloads gglint the first time and reuses it
afterwards; there's nothing else to install. It works on Linux, macOS and Windows.

To check every pull request for secrets, add this workflow as
`.github/workflows/agent-secrets.yml` (see [GitHub Action](#github-action) for the options):

```yaml
name: Agent secrets
on:
  pull_request:
permissions:
  contents: read
jobs:
  gglint:
    name: gglint secrets
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: corridor/gglint@v1
        with:
          command: secrets
          comment: "false"
```

## Usage

```text
uvx gglint mcp [--fix] [--source AGENT] [--format FORMAT] [PATH]
uvx gglint mcp --servers [PATH]
uvx gglint secrets [--fix] [--staged] [--format FORMAT] [PATH | FILE...]
uvx gglint hook [install|print] [--tool NAME] [--with-mcp] [PATH]
```

The options below are for `gglint mcp`; [`secrets`](#finding-secrets) and
[`hook`](#before-every-commit) describe their own.

| Option              | Effect                                                                                   |
| ------------------- | ---------------------------------------------------------------------------------------- |
| *(none)*            | Report every problem and draw each server in each agent.                                 |
| `--fix`             | Add missing servers and config files, start each server the same way, then report what's left. |
| `--source AGENT`    | Take each server's setup from `AGENT` where it has the server. `AGENT` is a name from the `--source` column of [Supported agents](#supported-agents), such as `claude-code` or `gemini-cli`. |
| `--format FORMAT`   | `table` (the default), `lines`, `github`, `sarif` or `markdown`. See [Output formats](#output-formats). |
| `--servers`         | List every configured server, file by file, without comparing them.                      |
| `PATH`              | The repository's root. Defaults to the current directory.                               |

| Exit status | Meaning                                                                       |
| ----------- | ----------------------------------------------------------------------------- |
| `0`         | Every agent gets the same servers, set up the same way.                       |
| `1`         | gglint found a problem, one is left after `--fix`, or there are no MCP config files. |

`gglint secrets` exits with status 1 when it finds a problem, and `0` when it finds none or
only warnings, such as a secret in a file git ignores.

Colours are used when standard output is a terminal and `NO_COLOR` isn't set. The summary
line, with counts and timing, goes to standard error.

## Finding secrets

`gglint secrets` finds credentials written out in the files that configure your coding
agents: for every [supported agent](#supported-agents), its MCP config files, settings, hooks,
rules, commands and instruction files such as `CLAUDE.md` (see
[Files it reads](#files-it-reads)). These files are committed, so a token pasted into
`.mcp.json` ends up in git history. It also stops the agents' personal files, such as
`.claude/settings.local.json`, from being committed at all.

```sh
uvx gglint secrets                 # report every secret
uvx gglint secrets --fix           # replace each with a reference to an environment variable
uvx gglint secrets --staged        # only what's staged in git: the copy the commit gets
uvx gglint secrets FILE...         # only these, where they're agent files, as a hook passes them
```

| Rule                | Reported when                                                                                     |
| ------------------- | ------------------------------------------------------------------------------------------------- |
| `inline-secret`     | Text has the format of a known credential: GitHub, GitLab, Anthropic, Slack, Stripe, Google and npm tokens, AWS access key IDs, API keys starting with `sk-`, and private key blocks. |
| `inline-credential` | A JSON or TOML value under a name such as `token`, `password`, `api_key` or `Authorization` is written out instead of being a reference to an environment variable; a password is written into a URL (`postgresql://admin:secret@db/app`); or a command carries one, as a command Claude Code saves as allowed does: a header (`curl -H "Authorization: Bearer …"`), a variable set for the command (`PGPASSWORD=… psql`) or an option (`--token …`, `sshpass -p …`, `mysql -p…`). |
| `local-file`        | An agent file meant for one person's machine is in git, such as `.claude/settings.local.json`, where Claude Code saves the commands you allow, secrets and all: see the personal files in [Files it reads](#files-it-reads). Whatever it holds, so a secret no pattern recognises is stopped too. It's a warning while the file is on disk and git doesn't ignore it yet. |

Placeholders (`your-token-here`, `ghp_xxxx…`), references (`${GITHUB_TOKEN}`) and values that
name a variable (`bearer_token_env_var = "GITHUB_TOKEN"`) aren't reported. A secret's value is
never printed: gglint shows its kind, the name it's under, and where it is. A secret in a file
git ignores is a warning, since it isn't committed; only errors fail.

`--fix` writes the reference in the agent's own syntax, such as `Bearer ${env:GITHUB_TOKEN}`
in Cursor, and keeps everything else in the file as it was. An agent with no way to refer to
an environment variable, such as Codex in a `http_headers` table, is reported and left alone,
and so is a secret in a command: take it out of the command, or remove the command the agent
saved as allowed, with `/permissions` in Claude Code. `--fix` also adds a personal agent file to `.gitignore`; one
that's already in git needs `git rm --cached` too. A secret that was ever committed is in git
history: rotate it.

### Files it reads

Besides each agent's MCP config file, listed in [Supported agents](#supported-agents),
`gglint secrets` reads these files. In a folder, it reads the Markdown, JSON, TOML and YAML
files at its top. A personal file is reported by `local-file` once it's in git, whatever is in it; the
agent's docs say to keep it out of git, or it's where they say to put your API key.

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

It also reads `.windsurfrules` and `.clinerules`, the instructions of Windsurf and Cline, which
keep their MCP servers outside the repository.

### Before every commit

`gglint hook` runs `gglint secrets --staged` before each commit, so a secret never gets
committed in the first place. It uses the tool your repository already has:

| Tool         | What `uvx gglint hook` does                                              |
| ------------ | ------------------------------------------------------------------------ |
| husky        | Adds the command to `.husky/pre-commit`, keeping what's there.            |
| git          | Adds it to git's `pre-commit` hook, wherever `core.hooksPath` puts it.    |
| pre-commit   | Prints the lines for `.pre-commit-config.yaml`.                           |
| lefthook     | Prints the lines for `lefthook.yml`.                                      |
| lint-staged  | Prints the lines for `package.json`.                                      |

`--tool NAME` picks one instead of detecting it, `--with-mcp` adds the MCP check, and
`gglint hook print` only prints. Running it twice changes nothing. The hooks run
`uvx gglint@<version>`, pinned to the version that wrote them, so uv has to be installed.

With the [pre-commit](https://pre-commit.com) framework, gglint is also a source of hooks:

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/corridor/gglint
    rev: v0.4.5
    hooks:
      - id: gglint-secrets
      - id: gglint-mcp      # optional: the MCP servers of every agent match
```

## What gglint checks

| Rule               | Reported when                                                                                       | Fixed by          |
| ------------------ | --------------------------------------------------------------------------------------------------- | ----------------- |
| `missing-server`   | A server in one agent's config file is missing from another agent's.                                | `--fix`           |
| `missing-config`   | An agent has its own directory but no MCP config file, while other agents have servers.            | `--fix`           |
| `different-server` | The same server runs another version or has other arguments.                                        | `--fix`           |
|                    | The same server runs another program or URL, has other environment variables or headers, or is turned off in one agent. | Reported only |
| `renamed-server`   | The same server has a different name in some agents.                                                | Reported only     |

`--fix` makes the command lines match, since the same server should normally start the same
way everywhere. The other differences can be deliberate, such as each agent setting its own
environment variables, so gglint reports them and leaves them to you.

**Matching servers.** Servers are matched by name, then by what they run, so a server named
differently in one agent is still recognised as the same server. The way a package is
launched doesn't count as a difference:

- `npx -y pkg`, `npx --yes pkg@latest`, `pnpm dlx pkg`, `yarn dlx pkg`, `bunx pkg` and
  `bun x pkg` all run the npm package `pkg`.
- `uvx pkg`, `uv tool run pkg` and `pipx run pkg` all run the PyPI package `pkg`.
- `npx mcp-remote <url>` reaches the remote server at `<url>`.
- No version and `latest` both mean the newest version.
- `${KEY}` in Claude Code, `${env:KEY}` in Cursor and `$KEY` in Gemini CLI refer to the same
  environment variable.

Environment variables and headers are compared by name only, since their values can be
secrets.

## Supported agents

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

gglint compares the agents that have an MCP config file or their own directory at the
repository's root. A `CLAUDE.md`, `.cursorrules` or `AGENTS.md` file on its own doesn't
count, because it doesn't show that the agent is set up in the repository. Antigravity,
Copilot CLI, Gemini CLI, VS Code and Zed count only once they have their config file: many
repositories have an `.agents/`, `.github/`, `.gemini/`, `.vscode/` or `.zed/` directory for
other reasons, such as skills, workflows, Gemini Code Assist's reviews or editor settings.

Crush's JSON files are its older format: gglint reads and fixes `.crush.json` or `crush.json`
but never creates one. Crush also reads servers from `.crushrc` or `crushrc`, shell scripts
gglint doesn't read, and gglint says so when one is there. Crush fills in `$VAR` and runs
`$(command)` in every value, so a value with `$` in it isn't copied to Crush.

Kilo Code reads no environment variable in a repository's config: though its docs show
`{env:VAR}`, it skips a whole `kilo.json` that has one, with every server in it. So a server
that reads a variable isn't copied to Kilo Code, and for a secret in its file, `gglint secrets`
suggests your own Kilo Code config instead.

VS Code can ask for a value once with `${input:id}`, such as a token; a server that uses one
isn't copied to another agent. Zed doesn't fill in environment variables, so a server that
reads one isn't copied to Zed, and a Zed server started by an extension is named but not
compared.

Continue reads every YAML and JSON file in `.continue/mcpServers/`. In a YAML file, it fills
in `${{ secrets.NAME }}` from its `.env` files and then the environment; in a JSON file, written
as Claude Code writes it, only `${NAME}` in `env`. `--fix` adds each missing server in a YAML
file of its own, named after the server, and never changes a file that's there.

Mistral Vibe writes values as they are, and reads a header from the environment with
`api_key_env`, so a server that reads a variable another way isn't copied to it. It reaches
remote servers over streamable HTTP only, so a server reached over SSE isn't copied to it
either.

Qoder reads Claude Code's `.mcp.json` as well as its own file: a server there counts as
Qoder's, and a server missing from both is added to `.mcp.json` only.

Copilot CLI reads `.mcp.json` instead of `.github/mcp.json` whenever `.mcp.json` exists, so it
gets the same servers as Claude Code, and gglint warns that `.github/mcp.json` goes unread.
`--fix` creates a `.mcp.json` only if every server in `.github/mcp.json` is written to it.
Copilot also skips a `.github/mcp.json` that lists no servers under `mcpServers` or at its
top, such as one with VS Code's `servers`: gglint warns about it and leaves it alone.

## How fixing works

Every server is copied from the first config file where it's turned on, in the order of the
table in [Supported agents](#supported-agents). To make one agent the source of truth
instead, name it with `--source`: `uvx gglint mcp --fix --source codex` takes each server from
Codex wherever Codex has it turned on, and from the first agent otherwise. `--fix` never
removes a server.

- A missing server is added to each config file that lacks it.
- An agent with its own directory but no config file gets one, with every server that is
  turned on in the other agents.
- A server that runs another version of the same program, or the same program with other
  arguments, gets the command and arguments of that first config file. Its environment
  variables, headers, on/off state and keys only one agent has, such as Codex's `cwd`, are
  kept. Review the result with `git diff`: a difference such as a runtime chosen per agent,
  or a path that starts with Cursor's `${workspaceFolder}`, may have been deliberate.
- JSON is edited with [jsonc-parser](https://github.com/dprint/jsonc-parser) and TOML with
  [toml_edit](https://github.com/toml-rs/toml), so comments and formatting are kept.
- Running `--fix` a second time changes nothing.
- A file excluded by `.gitignore` is still updated, and marked `gitignored` in the output,
  because the change won't be committed.

References to environment variables are rewritten in each agent's syntax, in values and in
command lines:

| Agent                                        | Environment variable `KEY`                                                                      |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Claude Code                                  | `${KEY}`                                                                                        |
| Cursor                                       | `${env:KEY}`                                                                                    |
| OpenCode                                     | `{env:KEY}`                                                                                     |
| Gemini CLI, Qoder, Qwen Code                 | `$KEY` or `${KEY}`, written `${KEY}`                                                            |
| Kiro, Factory Droid                          | `${KEY}`, in environment variables and headers only                                             |
| Codex                                        | `env_vars = ["KEY"]`, `env_http_headers`, or `bearer_token_env_var` for `Authorization: Bearer` |
| Kimi Code                                    | `bearerTokenEnvVar` for `Authorization: Bearer`                                                 |
| Antigravity, Copilot CLI, Junie, Trae        | None documented, so a server that reads the environment isn't copied to them                    |

Each server is written the way its agent expects:

- Remote servers get `"type": "http"`, or `"sse"` for a URL ending in `/sse`, in Claude Code,
  Copilot CLI, Factory Droid and Qoder.
- Gemini CLI and Qwen Code get the URL as `httpUrl`, or as `url` for SSE.
- Kimi Code gets `"transport": "sse"` for SSE, and a bearer token read from the environment
  as `bearerTokenEnvVar`.
- Antigravity gets the URL as `serverUrl`. It reaches remote servers over streamable HTTP
  only, so a server reached over SSE isn't added.
- Copilot CLI's local servers get `"type": "local"`.
- OpenCode servers get `"type": "local"` or `"remote"`, with the command and its arguments in
  one list.

A server is skipped, and listed under `Not added:`, when its setup can't be expressed in the
other agent's format: for example Claude Code's `${KEY:-default}`, Cursor's
`${workspaceFolder}`, an environment variable inside a command line, which Codex, Kiro and
Factory Droid don't expand, or any environment variable for an agent whose docs don't say how
to refer to one.

### Example

```console
$ uvx gglint mcp --fix
Changed 4 files:
  .mcp.json             added linear
  .codex/config.toml    added github, playwright, linear
  .cursor/mcp.json      added github; updated playwright
  opencode.json (new)   added github, playwright, linear

Review the changes with `git diff`.
```

The same `github` server, written for Codex and Cursor:

```diff
# .codex/config.toml
+[mcp_servers.github]
+url = "https://api.githubcopilot.com/mcp/"
+bearer_token_env_var = "GITHUB_TOKEN"

# .cursor/mcp.json
+    "github": {
+      "url": "https://api.githubcopilot.com/mcp/",
+      "headers": {
+        "Authorization": "Bearer ${env:GITHUB_TOKEN}"
+      }
+    }
```

Cursor's `playwright` now starts as in Claude Code:

```diff
# .cursor/mcp.json
-      "args": ["-y", "@playwright/mcp@latest"]
+      "args": ["-y", "@playwright/mcp@0.0.78", "--headless"]
```

## Output formats

`--format lines` prints one problem per line: where it is, the rule, and a description, with
the server's other locations indented below it.

```console
$ uvx gglint mcp --format lines
./.mcp.json:3:5  missing-server   `github` is set up for Claude Code but not for Codex (.codex/config.toml) or Cursor (.cursor/mcp.json)
./.mcp.json:8:5  different-server `playwright` runs different versions of the npm package `@playwright/mcp`: 0.0.78 in Claude Code; the newest in Cursor
    also at ./.cursor/mcp.json:3:5
./.mcp.json:3:5  missing-config   OpenCode has a .opencode directory but no opencode.json, so it gets none of the 3 MCP servers set up for Claude Code and Cursor
```

`--servers` lists every server as it's written. Environment variables and headers are shown
by name, never by value.

```console
$ uvx gglint mcp --servers
./.mcp.json:3:5  claude-code  github  https://api.githubcopilot.com/mcp/  (headers=[Authorization])
./.mcp.json:8:5  claude-code  playwright  npx -y @playwright/mcp@0.0.78 --headless
./.cursor/mcp.json:3:5  cursor       playwright  npx -y @playwright/mcp@latest
./.cursor/mcp.json:7:5  cursor       linear  https://mcp.linear.app/sse
```

### Formats for CI

`--format` takes the same five values for `mcp` and `secrets`:

| Format     | Prints                                                                                         |
| ---------- | ---------------------------------------------------------------------------------------------- |
| `table`    | The default: each command's own drawing, for reading in a terminal.                            |
| `lines`    | One line per problem.                                                                          |
| `github`   | [Workflow commands](https://docs.github.com/actions/reference/workflow-commands-for-github-actions), so GitHub shows each problem as an annotation on its line in a pull request's *Files changed*. |
| `sarif`    | A [SARIF](https://sarifweb.azurewebsites.net) 2.1.0 log, for GitHub's *Security*, *Code scanning* tab. Each command is its own category. |
| `markdown` | A section for a pull request comment: the problems as a table, and for `mcp` a table of servers by agent. |

The summary line still goes to standard error, so standard output holds only the report.

## GitHub Action

The gglint action checks every pull request, and only reports: it never changes a file.

```yaml
# .github/workflows/gglint.yml
name: gglint
on: [pull_request]

permissions:
  contents: read
  pull-requests: write   # for the comment

jobs:
  gglint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: corridor/gglint@v1
```

It does three things with what gglint finds:

- **Annotations.** Each problem appears on its line in the pull request's *Files changed*.
- **One comment.** The problems, and a table of which agent gets which MCP server, in a single
  comment that's updated on every push, and left alone when a pull request has no problems.
  Posting needs `pull-requests: write`; a pull request from a fork can't post, and still gets
  the annotations and the failing check.
- **Code scanning (optional).** With `sarif: true`, the findings also go to *Security*, *Code
  scanning*, where they can be tracked and dismissed. That needs `security-events: write`, and
  Code scanning enabled on a private repository.

| Input              | Default   | Meaning                                                           |
| ------------------ | --------- | ----------------------------------------------------------------- |
| `command`          | `all`     | `secrets`, `mcp` or `all`.                                        |
| `path`             | `.`       | The repository's root, for a repository checked out elsewhere.    |
| `version`          | the one `@v1` was released with | The gglint version to run, from PyPI. Pin it for repeatable runs. |
| `fail-on-problems` | `true`    | Set to `false` to report without failing the check.               |
| `comment`          | `true`    | Post the pull request comment.                                    |
| `sarif`            | `false`   | Upload to Code scanning.                                          |

A repository with no MCP config files has nothing to compare, so `mcp` is skipped with a
notice. The action runs gglint with [uv](https://docs.astral.sh/uv/) and doesn't start a
server or fetch anything else.

To run gglint without the action:

```yaml
      - uses: astral-sh/setup-uv@v7
      - run: uvx gglint@0.4.5 mcp --format github
      - run: uvx gglint@0.4.5 secrets --format github
```

## Security

- **Read-only by default.** Without `--fix`, gglint reads files and writes nothing.
- **Nothing is executed or fetched.** MCP servers aren't started, packages aren't resolved,
  and environment variables aren't expanded. It's safe to run on an untrusted repository.
- **Secrets stay out of the output.** Environment variables and headers are printed by name
  only, and `gglint secrets` shows a secret's kind, the name it's under and where it is, never
  its value.
- **One external program: git.** gglint runs `git check-ignore` to mark files your
  `.gitignore` excludes (in `mcp --fix` and in `secrets`), `git ls-files` to see whether a
  personal agent file is in git, `git diff --cached` and `git show` to read what's staged for
  `secrets --staged`, and `git rev-parse --git-path hooks` for `gglint hook`.
- **Nothing is overwritten by `hook`.** An existing hook script is extended, once.

## Limitations

- Servers added by Claude Code plugins, enabled in `.claude/settings.json`, aren't read. A
  server provided by a plugin can be reported as missing from Claude Code.
- User-level settings such as `~/.claude.json` or `~/.codex/config.toml` aren't read.
- Only config files at the repository's root are read, so each package of a monorepo is
  checked separately.
- Gemini CLI loads only the servers listed in `mcp.allowed`, when its settings have that
  list, and never those in `mcp.excluded`; gglint reports the servers these lists leave out
  as turned off. `--fix` doesn't edit the lists, so a server it adds while `mcp.allowed` is
  set stays off until you add it there.
- `gglint secrets` reads a file line by line. It finds a value under a name such as `token`
  in JSON and TOML files, and in every agent file the credential formats it knows, passwords
  in URLs and secrets in commands. It misses a password made of one kind of letters
  (`"hunter"`) or shorter than 8 characters, and in a command one with no digit or symbol
  (`app-reader`), as commands and the text around them are full of words and names. It doesn't
  read `.env` files or other source, and doesn't search git history: a secret that was ever
  committed has to be rotated. An agent's `.env` file is reported once it's in git, but not
  read.
- `gglint secrets --fix` can't change a value for an agent that has no way to refer to an
  environment variable, such as Codex in `http_headers`; it reports it and says what to use
  instead.
- Windsurf, Cline and Goose read MCP servers only from the user's home directory, so there's
  nothing in a repository to compare.
- Amp, Codex, Copilot CLI, Gemini CLI, Mistral Vibe, Qoder, Qwen Code and Zed load a
  repository's MCP servers only once you trust the project or approve its servers. Trae reads `.trae/mcp.json` once project
  MCP is turned on in its settings.

## Development

gglint is written in Rust. With a [Rust toolchain](https://rustup.rs) 1.88 or later, run these
three commands in order before each commit; CI runs the same checks:

```sh
cargo fmt
cargo clippy --all-targets -- --deny warnings
cargo test
```

`tests/corpus.rs` runs `--fix` against a directory of real repositories and checks that no
server is removed, that no environment variable, header or on/off state changes, that a
config file is created only in an agent's own directory, that no file becomes unreadable,
that the number of missing servers and differences never grows, and that a second run changes
nothing:

```sh
GGLINT_CORPUS=path/to/clones cargo test --release --test corpus -- --ignored --nocapture
```

The code lives in [`src/`](https://github.com/corridor/gglint/tree/main/src).
[`harness/`](https://github.com/corridor/gglint/tree/main/src/harness) does the MCP check:
`agents.rs` describes each agent in one table, `mcp.rs` reads each agent's config,
`compare.rs` finds the problems, and `fix.rs` writes the fixes. Supporting another agent
starts with a row in `agents.rs`. `report.rs` is what every check prints through,
`secrets.rs` finds secrets and replaces them, and `hook.rs` sets up the pre-commit hooks.

### Releasing

1. Update `version` in `Cargo.toml` and add the release to `CHANGELOG.md`.
2. Merge to `main`.
3. Publish a GitHub release tagged `v<version>`, for example `v0.4.5`.
4. Move the `v1` tag to the release's commit, so `uses: corridor/gglint@v1` runs it:
   `git tag -f v1 v<version> && git push -f origin v1`.

The [release workflow](https://github.com/corridor/gglint/blob/main/.github/workflows/pypi_release.yml)
builds and tests every wheel, publishes them to PyPI, and attaches them to the release.

## License

gglint is licensed under the [Apache License 2.0](https://github.com/corridor/gglint/blob/main/LICENSE).
The licenses of the libraries compiled into it are listed in
[THIRD_PARTY_LICENSES](https://github.com/corridor/gglint/blob/main/THIRD_PARTY_LICENSES).
