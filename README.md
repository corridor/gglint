# gglint

**Give every coding agent in your repository the same MCP servers.**

Claude Code, Codex, Cursor and OpenCode each read their MCP servers from their own file, in
their own format. When a repository keeps config files for more than one of them, the files
drift: someone adds a server to `.mcp.json` and forgets `.cursor/mcp.json`. `gglint mcp`
reads every agent's config file, shows which file has which server, and adds the servers a
file is missing, written the way that agent expects.

It only adds. Nothing already in your files is changed or removed, and no config file is
created.

```console
$ gglint mcp
MCP servers in acme-app

  ╭────────────┬─────────────┬───────┬────────╮
  │ Server     │ Claude Code │ Codex │ Cursor │
  ├────────────┼─────────────┼───────┼────────┤
  │ github     │      ✓      │   ✗   │   ✗    │
  │ playwright │      ✓      │   ✗   │   ✓    │
  │ linear     │      ✗      │   ✗   │   ✓    │
  ╰────────────┴─────────────┴───────┴────────╯
  ✓ set up   ✗ missing   ○ turned off

3 problems

  github
    ✗ Missing from Codex and Cursor.
      Add it to .codex/config.toml and .cursor/mcp.json, as set up in .mcp.json:3

  playwright
    ✗ Missing from Codex.
      Add it to .codex/config.toml, as set up in .mcp.json:8

  linear
    ✗ Missing from Claude Code and Codex.
      Add it to .mcp.json and .codex/config.toml, as set up in .cursor/mcp.json:7
```

It never runs your MCP servers, never downloads anything, never prints a secret, and reads a
repository in under a millisecond.

**Why it matters.** In November 2026 we checked the 258 most-starred GitHub repositories set
up for two or more of these agents. 81 of them keep MCP config files for two or more agents,
and in 25 of those (31%) a server in one agent's file was missing from another's: 48 missing
entries in all.

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

gglint is a single program, built with Rust. With a [Rust toolchain](https://rustup.rs)
installed:

```sh
cargo install --git https://github.com/corridor/gglint --locked
```

`--locked` builds with the exact dependency versions gglint is tested with.

## Usage

Run it from the root of a repository, or give it the path:

```sh
gglint mcp                  # find the servers each config file is missing
gglint mcp --fix            # add them, changing nothing else
gglint mcp --format lines   # one line per problem, for editors and CI
gglint mcp --servers        # list every server, file by file
gglint mcp path/to/repo
```

| Exit status | Meaning |
| ----------- | ------- |
| 0           | Every MCP config file has every server |
| 1           | A server is missing from a config file (or still is after `--fix`), or there are no MCP config files |

Colours are used when standard output is a terminal and `NO_COLOR` isn't set. The last line,
with the counts and the time taken, goes to standard error.

### Fixing

`--fix` adds each missing server to the config file that's missing it, then draws the table
again:

```console
$ gglint mcp --fix
Changed 3 files:
  .mcp.json            added linear
  .codex/config.toml   added github, playwright, linear
  .cursor/mcp.json     added github

Review the changes with `git diff`.
```

Each server is written the way its agent expects. The same `github` server became:

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

`playwright` runs `@playwright/mcp@0.0.78 --headless` in Claude Code and the newest version
in Cursor. gglint doesn't report or change that: see [What gglint reports](#what-gglint-reports).

### Other formats

`--format lines` prints one line per problem: where the server is set up, the rule, and a
sentence naming the server, then where else it's set up.

```console
$ gglint mcp --format lines
./.mcp.json:3:5  missing-server   `github` is set up for Claude Code but not for Codex (.codex/config.toml) or Cursor (.cursor/mcp.json)
./.mcp.json:8:5  missing-server   `playwright` is set up for Claude Code and Cursor but not for Codex (.codex/config.toml)
    also at ./.cursor/mcp.json:3:5
./.cursor/mcp.json:7:5  missing-server   `linear` is set up for Cursor but not for Claude Code (.mcp.json) or Codex (.codex/config.toml)
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

Only the agents with an MCP config file at the repository's root are compared. An agent your
team uses without one, such as a `.cursor/` directory with no `mcp.json`, is left alone: its
users may not want MCP servers, and gglint doesn't create config files. A config file with no
servers in it, such as a `.codex/config.toml` that only sets the model, counts.

## What gglint reports

One rule:

| Rule             | Reported when                                                                                   |
| ---------------- | ----------------------------------------------------------------------------------------------- |
| `missing-server` | A server set up in one agent's MCP config file isn't in another agent's. The message names the file to add it to. |

**Servers are matched by name, and then by what they run**, so a server under another name
in one file is still there. How a server is started doesn't matter: `npx -y pkg`,
`npx --yes pkg@latest`, `pnpm dlx pkg`, `yarn dlx pkg`, `bunx pkg` and `bun x pkg` all run
the npm package `pkg`, `uvx pkg`, `uv tool run pkg` and `pipx run pkg` the PyPI package `pkg`,
and `npx mcp-remote <url>` reaches the server at `<url>`. References to an environment
variable mean the same in every agent's syntax: `${KEY}` in Claude Code and `${env:KEY}` in
Cursor are the same argument.

**How each file sets a server up isn't compared.** Another version, other arguments, other
environment variables or headers, or another program under the same name: in real
repositories these are usually deliberate, such as each agent running its own runtime or
port. gglint leaves them to you.

**A server turned off** (`enabled = false` in Codex or OpenCode) still counts as set up in
that file. A server turned off in every file that has it isn't missing anywhere.

## How `--fix` works

`--fix` **adds and never removes or changes** what is already written.

- **A missing server** is added to the agent's config file, copied from the first file where
  it's turned on.
- **No config file is created.** Only files that are already there are written.
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

**Not added**, and listed under `Not added:`, is a server whose setup the other agent can't
express: a value such as Claude Code's `${KEY:-default}` or Cursor's `${workspaceFolder}`, or
a reference to an environment variable in a command line, which Codex can't fill in.

## Use in CI

`gglint mcp` exits with status 1 when a config file is missing a server, so it can fail a
build:

```yaml
# .github/workflows/mcp.yml
name: MCP servers
on: [pull_request]
jobs:
  gglint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo install --git https://github.com/corridor/gglint --locked
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
- **It's fast.** Reading and comparing a repository's config files takes well under a
  millisecond; most of the time is spent starting the program.

## Limitations

- **Claude Code plugins.** Servers a plugin adds, turned on in `.claude/settings.json`, aren't
  read, so a server another agent's file has can be reported missing from `.mcp.json`.
- **Settings outside the repository**, such as each person's `~/.claude.json` or
  `~/.codex/config.toml`, aren't read.
- **Other agents**, such as VS Code and GitHub Copilot (`.vscode/mcp.json`) or Gemini CLI
  (`.gemini/settings.json`), aren't read yet.
- **Monorepos.** Only config files at the repository's root are read.
- **Codex** only reads `.codex/config.toml` in projects the user has trusted.

## Use it as a library

```rust
use std::path::Path;

use gglint::harness::{check_mcp, fix_mcp, read_mcp};

let root = Path::new("path/to/repo");
let configs = read_mcp(root); // each agent's MCP config file, and its servers
let checks = check_mcp(&configs); // each server in each config file
for finding in checks.iter().flat_map(|check| &check.findings) {
    println!("{}: {}", finding.rule.as_str(), finding.message);
}
let report = fix_mcp(root, &configs, &checks); // add what's missing
```

## Development

```sh
cargo test
```

`tests/corpus.rs` checks `--fix` against real repositories. Point it at a directory of clones,
and it fixes a copy of each and fails if anything that was there is removed or changed, if a
config file is created or can't be read afterwards, if there are more problems after than
before, or if a second run changes anything:

```sh
GGLINT_CORPUS=path/to/clones cargo test --release --test corpus -- --ignored --nocapture
```

The code is in [`src/harness/`](src/harness/): `mcp.rs` reads each agent's file, `compare.rs`
finds the servers each file is missing, and `fix.rs` adds them.

## License

Apache-2.0. See [LICENSE](LICENSE).
