---
title: Command line
description: Every gglint command, option and exit status.
---

```text
gglint mcp     [--fix] [--source AGENT] [--format FORMAT] [PATH]
gglint mcp     --servers [PATH]
gglint secrets [--fix] [--staged] [--format FORMAT] [PATH | FILE...]
gglint hook    [install | print] [--tool NAME] [--with-mcp] [PATH]
gglint watch   [--port N] [--dir FOLDER] [--no-open] (claude | codex | -- COMMAND) [ARGS...]
gglint watch   --view FOLDER
```

`PATH` is the root of the repository, and defaults to where you are. Every command exits with `0`
when everything is fine and `1` when it found a problem. Colours are turned off when the output
isn't a terminal or `NO_COLOR` is set.

## `gglint mcp`

Runs the [MCP servers](/gglint/checks/mcp-servers/) check. It also exits with `1` if the
repository has no MCP config files at all.

| Option            | What it does                                                             |
| ----------------- | ------------------------------------------------------------------------ |
| `--fix`           | Add missing servers and config files, and make setups match              |
| `--source AGENT`  | Copy servers from this agent first, such as `codex` ([names](/gglint/reference/agents/)) |
| `--servers`       | Just list every server, file by file                                     |
| `--format FORMAT` | `table`, `lines`, `github`, `sarif` or `markdown` ([examples](/gglint/reference/output-formats/)) |

## `gglint secrets`

Runs the [Secrets](/gglint/checks/secrets/) check. Warnings don't change the exit status.

| Option            | What it does                                                          |
| ----------------- | --------------------------------------------------------------------- |
| `--fix`           | Swap secrets for environment variables and gitignore personal files   |
| `--staged`        | Check only what's staged for the next commit                          |
| `FILE...`         | Check only these files                                                |
| `--format FORMAT` | `table`, `lines`, `github`, `sarif` or `markdown`                     |

## `gglint hook`

Sets up a check [before every commit](/gglint/guides/pre-commit/).

| Option        | What it does                                                       |
| ------------- | ------------------------------------------------------------------ |
| `install`     | The default. Sets up husky or git, or prints lines for other tools |
| `print`       | Only prints the lines to add                                       |
| `--tool NAME` | `husky`, `git`, `pre-commit`, `lefthook` or `lint-staged`          |
| `--with-mcp`  | Also checks MCP servers                                            |

## `gglint watch`

Starts an agent with what it sends recorded, and shows it on a local page
([how it works](/gglint/checks/watch/)). Exits with the agent's exit status; in a terminal the
page stays up after the agent exits, until Ctrl-C.

| Option                | What it does                                                    |
| --------------------- | --------------------------------------------------------------- |
| `-- COMMAND...`       | Watch any program, through `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL` and `HTTPS_PROXY` |
| `--view FOLDER`       | Show an earlier run from `~/.gglint/watch/`                     |
| `--port N`            | Listen on this port of `127.0.0.1`                              |
| `--dir FOLDER`        | Record into this folder                                         |
| `--no-open`           | Don't open the page in the browser                              |
| `--upstream NAME=URL` | Send `anthropic`, `openai` or `chatgpt` requests to another URL |
