---
title: renamed-server
description: The same MCP server has a different name in some coding agents.
---

**Command:** `gglint mcp` · **Severity:** problem · **Fixed by `--fix`:** no

## What it finds

One server, recognised by what it runs, that has a different name in some agents.

## Why it matters

Agents name a server's tools after the server, so the same tool is `docs` in one agent and
`fetch` in another. Instructions, prompts and permissions that name it work in one agent only.

## Example

```json
// .mcp.json (Claude Code)
"docs": { "command": "uvx", "args": ["mcp-server-fetch"] }

// .cursor/mcp.json
"fetch": { "command": "uvx", "args": ["mcp-server-fetch"] }
```

```text
./.mcp.json:12:5  renamed-server   One server has different names: `docs` in Claude Code; `fetch` in Cursor
    also at ./.cursor/mcp.json:7:5
```

## How to fix it

Pick one name and rename the server in the other files by hand. gglint doesn't rename
servers, because something else may refer to the name.

`--fix` still treats the two as one server: it doesn't add a second copy under the other name.
