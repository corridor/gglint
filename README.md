# gglint

**Give every coding agent in your repository the same MCP servers.**

Claude Code, Codex, Cursor and OpenCode each read their MCP servers from their own file, in
their own format. When a team uses more than one, the files drift: someone adds a server to
`.mcp.json` and forgets `.cursor/mcp.json`, pins a version in one file and not another, or
never gives an agent an MCP config file at all. `gglint mcp` reads every agent's config,
shows which agent gets which server, and reports each difference. `--fix` adds the missing
servers, written the way each agent expects, and changes nothing else.

```console
$ gglint mcp
MCP servers in acme-app

  ╭────────────┬─────────────┬───────┬────────┬──────────╮
  │ Server     │ Claude Code │ Codex │ Cursor │ OpenCode │
  ├────────────┼─────────────┼───────┼────────┼──────────┤
  │ github     │      ✓      │   ✗   │   ✗    │    ✗     │
  │ playwright │      ✓      │   ✗   │   ≠    │    ✗     │
  │ linear     │      ✗      │   ✗   │   ✓    │    ✗     │
  ╰────────────┴─────────────┴───────┴────────┴──────────╯
  ✓ set up   ≠ set up differently   ✗ missing   ○ turned off

6 problems

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

  Missing config files
    ✗ OpenCode has a .opencode directory but no opencode.json.
      `gglint mcp --fix --create` creates opencode.json with the 3 MCP servers set up for Claude Code and Cursor
```

It never runs your MCP servers, never downloads anything, never prints a secret, and reads a
repository in under a millisecond.

**Why it matters.** In November 2026 we checked the 258 most-starred GitHub repositories set
up for two or more of these agents. 179 of them configure MCP servers, and 128 of those (72%)
had at least one of these problems: 99 had an agent with its own directory and no MCP config
file, 25 had a server missing from another agent's config file, and 21 set up the same server
differently in two agents.

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
gglint mcp                  # report every problem, and draw each server in each agent
gglint mcp --fix            # add the servers missing from existing config files
gglint mcp --fix --create   # also create the config file an agent is missing
gglint mcp --format lines   # one line per problem, for editors and CI
gglint mcp --servers        # list every server, file by file
gglint mcp path/to/repo
```

| Exit status | Meaning |
| ----------- | ------- |
| 0           | Every agent gets the same servers, set up the same way |
| 1           | A problem was found (or is left after `--fix`), or there are no MCP config files |

Colours are used when standard output is a terminal and `NO_COLOR` isn't set. The last line,
with the counts and the time taken, goes to standard error.

### Fixing

`--fix` adds each missing server to the config file that's missing it, then draws the table
again with what's left:

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

What's left is for you to decide: `playwright` still runs another version, with other
arguments, in Cursor, and OpenCode still has no config file. `--fix --create` creates it:

```console
$ gglint mcp --fix --create
Changed 1 file:
  opencode.json (new)   added github, playwright, linear
```

### Other formats

`--format lines` prints one line per problem: where it is, the rule, and a sentence naming
the server or agent, then where else the server is set up.

```console
$ gglint mcp --format lines
./.mcp.json:3:5  missing-server   `github` is set up for Claude Code but not for Codex (.codex/config.toml) or Cursor (.cursor/mcp.json)
./.mcp.json:8:5  missing-server   `playwright` is set up for Claude Code and Cursor but not for Codex (.codex/config.toml)
    also at ./.cursor/mcp.json:3:5
./.mcp.json:8:5  different-server `playwright` runs different versions of the npm package `@playwright/mcp`: 0.0.78 in Claude Code; the newest in Cursor
    also at ./.cursor/mcp.json:3:5
./.mcp.json:8:5  different-server `playwright` is started with different arguments: `--headless` in Claude Code; no arguments in Cursor
    also at ./.cursor/mcp.json:3:5
./.cursor/mcp.json:7:5  missing-server   `linear` is set up for Cursor but not for Claude Code (.mcp.json) or Codex (.codex/config.toml)
./.mcp.json:3:5  missing-config   OpenCode has a .opencode directory but no opencode.json, so it gets none of the 3 MCP servers set up for Claude Code and Cursor
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

| Agent       | MCP config file                   | Its directory | Format                                                                 |
| ----------- | --------------------------------- | ------------- | ---------------------------------------------------------------------- |
| Claude Code | `.mcp.json`                       | `.claude/`    | JSON, `mcpServers`                                                     |
| Codex       | `.codex/config.toml`              | `.codex/`     | TOML, `[mcp_servers.<name>]`                                           |
| Cursor      | `.cursor/mcp.json`                | `.cursor/`    | JSON, `mcpServers`                                                     |
| OpenCode    | `opencode.json`, `opencode.jsonc` | `.opencode/`  | JSON with comments, `mcp.servers` (or `mcp` before V2); the command is one list |

The agents compared are those with an MCP config file or their own directory at the
repository's root. A `CLAUDE.md`, `.cursorrules` or `AGENTS.md` alone doesn't count: it says
people use an agent, not that the repository sets it up.

## What gglint reports

| Rule               | Reported when                                                                                   |
| ------------------ | ----------------------------------------------------------------------------------------------- |
| `missing-server`   | A server set up in one agent's MCP config file isn't in another agent's. `--fix` adds it.       |
| `missing-config`   | An agent has its own directory but no MCP config file, while other agents have servers. `--fix --create` creates the file. |
| `different-server` | The same server runs another program, URL or version, with other arguments, environment variables or headers, or is turned off in one agent. Reported only. |
| `renamed-server`   | The same server has another name in some agents. Reported only.                                 |

**Servers are matched by name, and then by what they run**, so a server under another name
in one agent is still the same server (and reported as `renamed-server`, not missing). How a
server is started doesn't count as a difference: `npx -y pkg`, `npx --yes pkg@latest`,
`pnpm dlx pkg`, `yarn dlx pkg`, `bunx pkg` and `bun x pkg` all run the npm package `pkg`,
`uvx pkg`, `uv tool run pkg` and `pipx run pkg` the PyPI package `pkg`, and
`npx mcp-remote <url>` reaches the server at `<url>`. No version and `latest` both mean the
newest version. References to an environment variable mean the same in every agent's
syntax: `${KEY}` in Claude Code and `${env:KEY}` in Cursor are the same argument.

**Environment variables and headers** are compared by name only: their values can be secrets,
and each agent writes references to the environment differently.

**A server turned off** (`enabled = false` in Codex or OpenCode) is reported as a difference
from the agents where it's on, and isn't copied anywhere. A server turned off in every agent
that has it isn't missing anywhere.

## How `--fix` works

`--fix` **adds and never removes or changes** what is already written.

- **A missing server** is added to the agent's config file, copied from the first file where
  it's turned on.
- **A config file is created only with `--create`**, and only for an agent with its own
  directory. It gets every server turned on in the other agents.
- **A server set up differently is never changed.** Those differences are reported, and are
  often deliberate: each agent running its own runtime or port, or setting its own
  environment. Decide which setup is right and edit it yourself.
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

`gglint mcp` exits with status 1 when it finds a problem, so it can fail a build:

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
  read, so a server another agent's file has can be reported missing from Claude Code.
- **Settings outside the repository**, such as each person's `~/.claude.json` or
  `~/.codex/config.toml`, aren't read.
- **Other agents**, such as VS Code and GitHub Copilot (`.vscode/mcp.json`) or Gemini CLI
  (`.gemini/settings.json`), aren't read yet.
- **Monorepos.** Only config files at the repository's root are read.
- **Codex** only reads `.codex/config.toml` in projects the user has trusted.

## Use it as a library

```rust
use std::path::Path;

use gglint::harness::{FixOptions, check_mcp, fix_mcp, read_mcp};

let root = Path::new("path/to/repo");
let configs = read_mcp(root); // each agent's MCP config file, and its servers
let check = check_mcp(root, &configs); // each server in each agent, and every problem
for finding in check.findings() {
    println!("{}: {}", finding.rule.as_str(), finding.message);
}
let report = fix_mcp(root, &configs, &check, &FixOptions::default()); // add what's missing
```

## Development

```sh
cargo test
```

`tests/corpus.rs` checks `--fix` against real repositories. Point it at a directory of clones,
and it fixes a copy of each, with and without `--create`, and fails if anything that was there
is removed or changed, if a config file is created without `--create` or where the agent has
no directory, if a config can't be read afterwards, if there are more problems after than
before, or if a second run changes anything:

```sh
GGLINT_CORPUS=path/to/clones cargo test --release --test corpus -- --ignored --nocapture
```

The code is in [`src/harness/`](src/harness/): `mcp.rs` reads each agent's file, `compare.rs`
finds the problems, and `fix.rs` adds the missing servers.

## License

Apache-2.0. See [LICENSE](LICENSE).
