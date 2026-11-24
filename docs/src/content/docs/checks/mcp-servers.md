---
title: MCP servers
description: gglint mcp checks that every coding agent in a repository gets the same MCP servers, set up the same way, and fixes the differences.
---

Every coding agent keeps its MCP servers in its own file: `.mcp.json` for Claude Code,
`.codex/config.toml` for Codex, `.cursor/mcp.json` for Cursor. If you add a server to one, the
others don't get it. If you bump a version in one, the others keep running the old one.
`gglint mcp` compares all of them and shows you where they differ.

```sh
uvx gglint mcp
```

```text
  ╭────────────┬─────────────┬────────╮
  │ Server     │ Claude Code │ Cursor │
  ├────────────┼─────────────┼────────┤
  │ playwright │      ✓      │   ≠    │
  │ linear     │      ✗      │   ✓    │
  ╰────────────┴─────────────┴────────╯

  playwright
    ≠ Runs different versions of `@playwright/mcp`: 0.0.78 in Claude Code; the newest in Cursor.
  linear
    ✗ Missing from Claude Code. Add it to .mcp.json, as set up in .cursor/mcp.json:11
```

A `✓` means the server is set up, `≠` that it's set up differently, `✗` that it's missing and
`○` that it's turned off. Each kind of problem is a rule:
[`missing-server`](/gglint/reference/rules/missing-server/),
[`missing-config`](/gglint/reference/rules/missing-config/),
[`different-server`](/gglint/reference/rules/different-server/) and
[`renamed-server`](/gglint/reference/rules/renamed-server/).

## Fixing it

```sh
uvx gglint mcp --fix
git diff
```

This copies each missing server into the agents that don't have it, written the way each agent
expects, and makes versions and arguments match. It never removes a server, it keeps your
comments and formatting, and running it again changes nothing.

Servers are copied from the first agent that has them, in the order of
[Supported agents](/gglint/reference/agents/), so Claude Code wins by default. To make another
agent the source, use `--source`, as in `uvx gglint mcp --fix --source codex`.

Some differences might be on purpose, so gglint points them out and leaves them alone: a
different program under the same name, different environment variables or headers, a server
turned off in one agent, or a server with another name. Change those by hand. If a server can't
be written in another agent's format at all, for example because it uses Cursor's
`${workspaceFolder}`, it's listed under `Not added:` with the reason.

## How it decides two servers are the same

It matches servers by name first, then by what they run. How you launch a package doesn't
matter: `npx -y pkg`, `pnpm dlx pkg` and `bunx pkg` are the same server, and no version is the
same as `latest`. Environment variables and headers are compared by name only, never by value.

gglint only reads config files at the root of the repository, so each package in a monorepo is
checked on its own. It doesn't read user-level files such as `~/.claude.json`, or servers that
come from Claude Code plugins.
