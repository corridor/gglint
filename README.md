# gglint

**Keep every coding agent in your repository on the same MCP servers.**

Claude Code, Codex, Cursor and OpenCode each read their MCP servers from their own file, in
their own format. When a team uses more than one, the files drift: someone adds a server for
Claude Code and forgets Cursor, or pins a version in one place and not another. `gglint mcp`
reads every agent's config, shows which agent gets which server, and adds what's missing in
each agent's own format.

```console
$ gglint mcp
MCP servers in acme-app

  ╭────────────┬─────────────┬───────┬────────╮
  │ Server     │ Claude Code │ Codex │ Cursor │
  ├────────────┼─────────────┼───────┼────────┤
  │ github     │      ✓      │   ✗   │   ✗    │
  │ playwright │      ✓      │   ✗   │   ≠    │
  │ linear     │      ✗      │   ✗   │   ✓    │
  ╰────────────┴─────────────┴───────┴────────╯
  ✓ set up   ≠ set up differently   ✗ missing   ○ turned off

5 problems

  github
    ✗ Missing from Codex and Cursor.
      Add it to .codex/config.toml and .cursor/mcp.json, as set up in .mcp.json:3

  playwright
    ✗ Missing from Codex.
      Add it to .codex/config.toml, as set up in .mcp.json:8
    ≠ Runs different versions of the npm package `@playwright/mcp`: 0.0.78 in Claude Code; the newest in Cursor.
      .mcp.json:8, .cursor/mcp.json:3
    ≠ Started with different arguments: `--headless` in Claude Code; no arguments in Cursor.
      .mcp.json:8, .cursor/mcp.json:3

  linear
    ✗ Missing from Claude Code and Codex.
      Add it to .mcp.json and .codex/config.toml, as set up in .cursor/mcp.json:7
```

It never runs your MCP servers, never downloads anything, never prints a secret, and reads a
repository in under a millisecond.

**Why it matters.** In November 2026 we checked the 258 most-starred GitHub repositories set
up for two or more of these agents. 179 of them configure MCP servers, and 143 of those (80%)
gave their agents different servers. Nearly every time, one agent was simply missing a server
the others had.

## Contents

- [Install](#install)
- [Usage](#usage)
- [Supported agents](#supported-agents)
- [What gglint reports](#what-gglint-reports)
- [How `--fix` works](#how---fix-works)
- [Use in CI](#use-in-ci)
- [Safety](#safety)
- [Limitations](#limitations)
- [Use it as a library](#use-it-as-a-library)
- [Development](#development)
- [License](#license)

## Install

gglint is a single binary, built with Rust. With a [Rust toolchain](https://rustup.rs)
installed:

```sh
cargo install --git https://github.com/corridor/gglint --branch release --locked
```

`--locked` builds with the exact dependency versions gglint is tested with.

## Usage

Run it from the root of a repository, or give it the path:

```sh
gglint mcp                     # check, and draw each server in each agent
gglint mcp --fix               # add what's missing; never change what's there
gglint mcp --fix --overwrite   # also make servers set up differently match
gglint mcp --format lines      # one line per problem, for editors and CI
gglint mcp --servers           # list every server, without comparing
gglint mcp path/to/repo
```

| Exit status | Meaning |
| ----------- | ------- |
| 0           | Every agent gets the same servers, set up the same way |
| 1           | A problem was found (or is left after `--fix`), or there are no MCP config files |

Colours are used when standard output is a terminal and `NO_COLOR` isn't set. The last line,
with the counts and the time taken, goes to standard error.

### Fixing

`--fix` adds what's missing, in each agent's own format, and draws the table again:

```console
$ gglint mcp --fix
Changed 3 files:
  .mcp.json            added linear
  .codex/config.toml   added github, playwright, linear
  .cursor/mcp.json     added github

Left as they are:
  playwright in Cursor: set up differently from Claude Code; left as it is, and `--overwrite` copies Claude Code's setup

Review the changes with `git diff`.
```

The servers it adds are written the way each agent expects. The same `github` server became:

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

### Other formats

`--format lines` prints one line per problem: where the server is set up, the rule, and a
sentence naming the server, then where the other agents set it up.

```console
$ gglint mcp --format lines
./.mcp.json:3:5  missing-server   `github` is set up for Claude Code but not for Codex (.codex/config.toml) or Cursor (.cursor/mcp.json)
./.mcp.json:8:5  missing-server   `playwright` is set up for Claude Code and Cursor but not for Codex (.codex/config.toml)
    also at ./.cursor/mcp.json:3:5
...
```

`--servers` lists every server as it's written, with the names of its environment variables
and headers (never their values):

```console
$ gglint mcp --servers
./.mcp.json:3:5  claude-code  github  https://api.githubcopilot.com/mcp/  (headers=[Authorization])
./.mcp.json:8:5  claude-code  playwright  npx -y @playwright/mcp@0.0.78 --headless
./.cursor/mcp.json:3:5  cursor       playwright  npx -y @playwright/mcp@latest
./.cursor/mcp.json:7:5  cursor       linear  https://mcp.linear.app/sse
```

## Supported agents

| Agent       | MCP config file                   | Format                                                                 |
| ----------- | --------------------------------- | ---------------------------------------------------------------------- |
| Claude Code | `.mcp.json`                       | JSON, `mcpServers`                                                     |
| Codex       | `.codex/config.toml`              | TOML, `[mcp_servers.<name>]`                                           |
| Cursor      | `.cursor/mcp.json`                | JSON, `mcpServers`                                                     |
| OpenCode    | `opencode.json`, `opencode.jsonc` | JSON with comments, `mcp.servers` (or `mcp` before V2); the command is one list |

Only the agents a repository is set up for are compared: those with their own files in it.

| Agent       | Its files                                       |
| ----------- | ----------------------------------------------- |
| Claude Code | `.claude/`, `CLAUDE.md`, `.mcp.json`            |
| Codex       | `.codex/`                                       |
| Cursor      | `.cursor/`, `.cursorrules`                      |
| OpenCode    | `.opencode/`, `opencode.json`, `opencode.jsonc` |

A name ending in `/` must be a directory. `AGENTS.md` is read by several agents, so it
doesn't count for any of them.

## What gglint reports

Servers are matched by name, and then by what they run, so a server renamed in one agent is
still the same server.

| Rule               | Reported when                                                                                                |
| ------------------ | ------------------------------------------------------------------------------------------------------------ |
| `missing-server`   | An agent the repository is set up for doesn't get a server the others do. The message names the file to add it to. |
| `different-server` | The same server runs another program, URL or version, with other arguments, environment variables or headers, or is turned off. |
| `renamed-server`   | The same server has another name in some agents.                                                             |

**What doesn't count as a difference.** How a package is started: `npx -y pkg`,
`npx --yes pkg@latest`, `pnpm dlx pkg`, `yarn dlx pkg`, `bunx pkg` and `bun x pkg` all run
the npm package `pkg`, and `uvx pkg`, `uv tool run pkg` and `pipx run pkg` the PyPI package
`pkg`. No version and `latest` both mean the newest version. References to an environment
variable mean the same in every agent's syntax: `${KEY}` in Claude Code and `${env:KEY}` in
Cursor are the same argument.

**Environment variables and headers** are compared by name only: their values can be secrets,
and each agent writes references to the environment differently.

## How `--fix` works

`--fix` **adds and never removes or changes** what is already written.

- **A server missing from an agent** is added to that agent's config file, copied from the
  first agent that has it.
- **A config file is created** only when the agent has its own directory (`.claude/`,
  `.codex/`, `.cursor/`, `.opencode/`). A `CLAUDE.md` or `.cursorrules` alone shows that people
  use the agent, not that they want MCP servers in it.
- **A server set up differently is left as it is.** Most such differences are deliberate: each
  agent running its own runtime or port, or setting its own environment variables. When you've
  decided which setup is right, `--fix --overwrite` copies the first agent's setup over the
  others. It writes only the keys that say how a server is started and what it's given, so keys
  one agent alone has, such as Codex's `cwd` or `enabled_tools`, are kept.
- **Comments and formatting are kept**: JSON is edited with
  [jsonc-parser](https://github.com/dprint/jsonc-parser)'s CST and TOML with
  [toml_edit](https://github.com/toml-rs/toml). Running `--fix` again changes nothing.
- **A file your `.gitignore` leaves out** is still written, and marked `gitignored`: the change
  stays on your machine.

Each agent's syntax for a reference to the user's environment is translated, in values and in
command lines:

| Agent       | Environment variable `KEY`                                                       |
| ----------- | -------------------------------------------------------------------------------- |
| Claude Code | `${KEY}`                                                                         |
| Cursor      | `${env:KEY}`                                                                     |
| OpenCode    | `{env:KEY}`                                                                      |
| Codex       | `env_vars = ["KEY"]`, `env_http_headers`, or `bearer_token_env_var` for `Authorization: Bearer` |

Remote servers get `"type": "http"` in Claude Code, or `"sse"` for a URL ending in `/sse`;
OpenCode servers get `"type": "local"` or `"remote"`, with the command and its arguments as one
list.

**Left for you to decide**, under `Left as they are`:

- a different program or URL under the same name;
- a value another agent can't express, such as Claude Code's `${KEY:-default}`, Cursor's
  `${workspaceFolder}`, or a reference in a command line, which Codex can't fill in;
- a server turned off in one agent, which may be on purpose;
- a server with another name, since permission rules refer to servers by name.

## Use in CI

`gglint mcp` exits with status 1 when the agents' servers differ, so it can fail a build:

```yaml
# .github/workflows/mcp.yml
name: MCP servers
on: [pull_request]
jobs:
  gglint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo install --git https://github.com/corridor/gglint --branch release --locked
      - run: gglint mcp --format lines
```

## Safety

- **Nothing is run or fetched.** MCP servers aren't started, packages aren't looked up, and
  references to the environment aren't expanded. gglint only reads files at fixed paths under
  the repository's root, so it's safe to point at an untrusted repository.
- **Secrets stay out of the output.** Environment variables and headers are printed by name,
  never by value.
- **The one program gglint runs** is `git check-ignore`, during `--fix`, to mark files your
  `.gitignore` leaves out.
- **It's fast.** Reading and comparing four agents' configs with 83 servers between them takes
  under 0.6 ms; the time is spent starting the program.

## Limitations

- **Claude Code plugins.** Servers a plugin adds, turned on in `.claude/settings.json`, aren't
  read, so they're reported as missing from Claude Code.
- **Settings outside the repository**, such as each person's `~/.claude.json` or
  `~/.codex/config.toml`, aren't read.
- **Other agents**, such as VS Code and GitHub Copilot (`.vscode/mcp.json`) or Gemini CLI
  (`.gemini/settings.json`), aren't read yet.
- **Monorepos.** Only config files at the repository's root are read.
- **Codex** only reads `.codex/config.toml` in projects the user has trusted.

## Use it as a library

```rust
use std::path::Path;

use gglint::harness::{FixOptions, check_mcp, fix_mcp, read_mcp, used_harnesses};

let root = Path::new("path/to/repo");
let configs = read_mcp(root); // each agent's MCP config file, and its servers
let checks = check_mcp(&configs, &used_harnesses(root)); // each server in each agent
for finding in checks.iter().flat_map(|check| &check.findings) {
    println!("{}: {}", finding.rule.as_str(), finding.message);
}
let report = fix_mcp(root, &configs, &checks, &FixOptions::default()); // add what's missing
```

## Development

```sh
cargo test
```

`tests/corpus.rs` checks `--fix` against real repositories. Point it at a directory of clones,
and it fixes a copy of each, with and without `--overwrite`, and fails if anything that was
there is removed or (without `--overwrite`) changed, if a config can't be read afterwards, if
there are more problems after than before, or if a second run changes anything:

```sh
GGLINT_CORPUS=path/to/clones cargo test --release --test corpus -- --ignored --nocapture
```

The code is in [`src/harness/`](src/harness/): `mcp.rs` reads each agent's file, `compare.rs`
compares them, and `fix.rs` writes the fixes.

## License

Apache-2.0. See [LICENSE](LICENSE).
