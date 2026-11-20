---
title: Sync MCP servers across agents
description: Give every coding agent in a repository the same MCP servers, started the same way, with gglint mcp --fix.
---

A repository set up for more than one coding agent has one MCP config file per agent, each in
its own format. This guide brings them back in line.

## See what differs

```sh
uvx gglint mcp
```

The table shows each server in each agent: `✓` set up, `≠` set up differently, `✗` missing,
`○` turned off. Below it, each problem says where it is and what `--fix` does about it:

```text
  playwright
    ≠ Runs different versions of the npm package `@playwright/mcp`: 0.0.78 in Claude Code; the newest in Cursor.
      `gglint mcp --fix` starts it everywhere as in .mcp.json:8

  linear
    ✗ Missing from Claude Code.
      Add it to .mcp.json, as set up in .cursor/mcp.json:11
```

To see every server as it's written, file by file, without comparing them:

```sh
uvx gglint mcp --servers
```

## Fix the differences

```sh
uvx gglint mcp --fix
git diff
```

`--fix` does three things, and reports what's left:

- adds each missing server to the config files that lack it;
- creates the config file of an agent that has its own directory, such as `.codex/`, but no
  config file;
- starts each server with the same command and arguments in every agent.

It never removes a server. A server whose setup can't be written in another agent's format is
listed under `Not added:`, with the reason.

## Choose which agent wins

By default each server is copied from the first config file where it's turned on, in the order
of the [Supported agents](/gglint/reference/agents/) table, so Claude Code's `.mcp.json` comes
first. To make another agent the source of truth, name it:

```sh
uvx gglint mcp --fix --source codex
```

Each server is then taken from Codex wherever Codex has it turned on, and from the first agent
otherwise.

## What `--fix` leaves to you

Some differences can be deliberate, so gglint reports them and changes nothing:

- a server that runs another program or URL under the same name;
- other environment variables or headers, compared by name only;
- a server turned off in one agent;
- a server with [another name](/gglint/reference/rules/renamed-server/) in some agents.

Edit those by hand, then run `uvx gglint mcp` again. It exits with status 0 once every agent
gets the same servers, set up the same way.

## Keep them in sync

- Run the check on every pull request with the
  [GitHub Action](/gglint/guides/github-action/).
- Or run it before each commit: `uvx gglint hook --with-mcp`. See
  [Check before every commit](/gglint/guides/pre-commit/).
