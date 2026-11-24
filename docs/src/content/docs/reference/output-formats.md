---
title: Output formats
description: gglint's five output formats, table, lines, github, sarif and markdown, for terminals, scripts and CI.
---

Both checks take `--format`. The default, `table`, is meant for reading in a terminal. The others
are for scripts and CI:

| Format     | What you get                                                                                   |
| ---------- | ---------------------------------------------------------------------------------------------- |
| `table`    | The default, drawn for a terminal                                                              |
| `lines`    | One line per problem, easy to grep                                                             |
| `github`   | [Annotations](https://docs.github.com/actions/reference/workflow-commands-for-github-actions) on the lines of a pull request |
| `sarif`    | A [SARIF](https://sarifweb.azurewebsites.net) 2.1.0 file for GitHub Code scanning              |
| `markdown` | A table of problems to post as a pull request comment                                          |

The summary line with counts and timing goes to standard error, so standard output only ever
holds the report.

`lines` looks like this. Each line has the location, the [rule](/gglint/reference/rules/) and
what's wrong, with any other places the same server appears indented below:

```text
./.mcp.json:3:5  missing-server   `github` is set up for Claude Code but not for Cursor (.cursor/mcp.json)
./.mcp.json:8:5  different-server `playwright` runs different versions of the npm package `@playwright/mcp`: 0.0.78 in Claude Code; the newest in Cursor
    also at ./.cursor/mcp.json:3:5
```

`gglint mcp --servers` skips the comparison and just lists every server as it's written.
Environment variables and headers are shown by name, never by value:

```text
./.mcp.json:3:5  claude-code  github  https://api.githubcopilot.com/mcp/  (headers=[Authorization])
./.mcp.json:8:5  claude-code  playwright  npx -y @playwright/mcp@0.0.78 --headless
./.cursor/mcp.json:3:5  cursor       playwright  npx -y @playwright/mcp@latest
```
