---
title: How servers are matched
description: How gglint decides that two entries in different agents' config files are the same MCP server, and which differences it ignores.
---

To compare agents, gglint first decides which entries in their config files are the same
server. Servers are matched **by name**, then **by what they run**, so a server named
differently in one agent is still recognised as the same server, and reported as
[`renamed-server`](/gglint/reference/rules/renamed-server/) instead of as two missing ones.

## Differences that aren't differences

The way a package is launched doesn't count:

- `npx -y pkg`, `npx --yes pkg@latest`, `pnpm dlx pkg`, `yarn dlx pkg`, `bunx pkg` and
  `bun x pkg` all run the npm package `pkg`.
- `uvx pkg`, `uv tool run pkg` and `pipx run pkg` all run the PyPI package `pkg`.
- `npx mcp-remote <url>` reaches the remote server at `<url>`.
- No version and `latest` both mean the newest version.
- `${KEY}` in Claude Code, `${env:KEY}` in Cursor and `$KEY` in Gemini CLI refer to the same
  environment variable.

## What is compared

Once matched, two setups of a server are compared on:

- the program or URL it runs;
- the package version;
- the arguments;
- the names of its environment variables and headers;
- whether it's turned on.

Environment variables and headers are compared **by name only**, since their values can be
secrets. Each difference is a separate
[`different-server`](/gglint/reference/rules/different-server/) problem.
