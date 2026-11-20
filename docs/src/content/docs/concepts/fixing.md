---
title: How fixing works
description: What gglint mcp --fix changes, where it takes each server from, how it writes each agent's format, and what it leaves alone.
---

`gglint mcp --fix` gives every agent the same servers, started the same way. It's designed to
be safe to run on any repository: it never removes a server, it keeps comments and formatting,
and running it a second time changes nothing.

## Where each server comes from

Every server is copied from the first config file where it's turned on, in the order of the
[Supported agents](/gglint/reference/agents/) table. To make one agent the source of truth
instead, name it with `--source`: `uvx gglint mcp --fix --source codex` takes each server from
Codex wherever Codex has it turned on, and from the first agent otherwise.

## What it changes

- A missing server is added to each config file that lacks it.
- An agent with its own directory but no config file gets one, with every server that is
  turned on in the other agents.
- A server that runs another version of the same program, or the same program with other
  arguments, gets the command and arguments of that first config file. Its environment
  variables, headers, on/off state and keys only one agent has, such as Codex's `cwd`, are
  kept.
- A file excluded by `.gitignore` is still updated, and marked `gitignored` in the output,
  because the change won't be committed.

Review the result with `git diff`: a difference such as a runtime chosen per agent, or a path
that starts with Cursor's `${workspaceFolder}`, may have been deliberate.

## What it leaves alone

Differences that can be deliberate are reported and left to you: another program under the
same name, other environment variables or headers, a server turned off in one agent, and a
server with another name.

## Keeping your files intact

JSON is edited with [jsonc-parser](https://github.com/dprint/jsonc-parser) and TOML with
[toml_edit](https://github.com/toml-rs/toml), so comments and formatting are kept. A server is
added to the file; nothing else in it is rewritten.

## Writing each agent's format

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

References to environment variables are rewritten in each agent's syntax, listed in
[Supported agents](/gglint/reference/agents/#environment-variables). Each server is also
written the way its agent expects:

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

## When a server can't be copied

A server is skipped, and listed under `Not added:`, when its setup can't be expressed in the
other agent's format: for example Claude Code's `${KEY:-default}`, Cursor's
`${workspaceFolder}`, an environment variable inside a command line, which Codex, Kiro and
Factory Droid don't expand, or any environment variable for an agent whose docs don't say how
to refer to one.

## How it's tested

Besides its unit tests, gglint runs `--fix` against a directory of real repositories and checks
that no server is removed, that no environment variable, header or on/off state changes, that
a config file is created only in an agent's own directory, that no file becomes unreadable,
that the number of problems never grows, and that a second run changes nothing.
