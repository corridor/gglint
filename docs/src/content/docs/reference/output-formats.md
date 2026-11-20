---
title: Output formats
description: gglint's five output formats, table, lines, github, sarif and markdown, for terminals, scripts and CI.
---

`--format` takes the same five values for `gglint mcp` and `gglint secrets`:

| Format     | Prints                                                                                         |
| ---------- | ---------------------------------------------------------------------------------------------- |
| `table`    | The default: each command's own drawing, for reading in a terminal.                            |
| `lines`    | One line per problem.                                                                          |
| `github`   | [Workflow commands](https://docs.github.com/actions/reference/workflow-commands-for-github-actions), so GitHub shows each problem as an annotation on its line in a pull request's *Files changed*. |
| `sarif`    | A [SARIF](https://sarifweb.azurewebsites.net) 2.1.0 log, for GitHub's *Security*, *Code scanning* tab. Each command is its own category. |
| `markdown` | A section for a pull request comment: the problems as a table, and for `mcp` a table of servers by agent. |

The summary line, with counts and timing, goes to standard error, so standard output holds only
the report. Colours are used when standard output is a terminal and `NO_COLOR` isn't set.

## `table`

```text
MCP servers in my-repo

  ╭────────────┬─────────────┬───────┬────────┬──────────╮
  │ Server     │ Claude Code │ Codex │ Cursor │ OpenCode │
  ├────────────┼─────────────┼───────┼────────┼──────────┤
  │ github     │      ✓      │   ✗   │   ✗    │    ✗     │
  │ playwright │      ✓      │   ✗   │   ≠    │    ✗     │
  │ linear     │      ✗      │   ✗   │   ✓    │    ✗     │
  ╰────────────┴─────────────┴───────┴────────┴──────────╯
  ✓ set up   ≠ set up differently   ✗ missing   ○ turned off

  playwright
    ≠ Runs different versions of the npm package `@playwright/mcp`: 0.0.78 in Claude Code; the newest in Cursor.
      `gglint mcp --fix` starts it everywhere as in .mcp.json:8
```

## `lines`

One problem per line: where it is, the [rule](/gglint/reference/rules/), and a description, with
the server's other locations indented below it.

```text
./.mcp.json:3:5  missing-server   `github` is set up for Claude Code but not for Cursor (.cursor/mcp.json)
./.mcp.json:8:5  different-server `playwright` runs different versions of the npm package `@playwright/mcp`: 0.0.78 in Claude Code; the newest in Cursor
    also at ./.cursor/mcp.json:3:5
```

## `--servers`

`gglint mcp --servers` lists every server as it's written. Environment variables and headers
are shown by name, never by value.

```text
./.mcp.json:3:5  claude-code  github  https://api.githubcopilot.com/mcp/  (headers=[Authorization])
./.mcp.json:8:5  claude-code  playwright  npx -y @playwright/mcp@0.0.78 --headless
./.cursor/mcp.json:3:5  cursor       playwright  npx -y @playwright/mcp@latest
./.cursor/mcp.json:7:5  cursor       linear  https://mcp.linear.app/sse
```
