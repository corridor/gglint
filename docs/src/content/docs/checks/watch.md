---
title: See what an agent sends
description: gglint watch records every request Claude Code or Codex sends, and shows its system prompt, tools, context and your prompt on a local page.
---

A coding agent sends much more than the prompt you type: a system prompt, its tool
definitions, your rules files and skills, the conversation so far, and headers that identify
you and your machine. It also talks to other hosts, for sign-in, telemetry and MCP servers.
`gglint watch` shows all of it.

```sh
gglint watch claude
gglint watch codex
```

gglint starts the agent, opens a page on `127.0.0.1`, and fills it in as the agent works.

## What the page shows

- **Each model request, broken into its parts**, with a bar of how much each takes: the
  hidden system prompt, tool definitions, injected context (rules files, skills, reminders),
  the earlier conversation, tool results, and the prompt you typed.
- **The headers sent**, with the ones that identify you, your session or your machine marked.
- **The reply**, put back together from the stream.
- **Every host the agent connected to**, with how many connections and how much data went
  each way.
- **Flags** for a secret or an email address in what was sent.

## How it works

Each agent lets you change where it sends model requests: Claude Code reads
`ANTHROPIC_BASE_URL`, and Codex takes a model provider with its own `base_url`. gglint points
these at itself, records each request, and sends it on to the real API over HTTPS, so the
agent works as usual. Your sign-in is passed through, never stored.

Everything else the agent connects to goes through gglint as `HTTPS_PROXY`. Those
connections stay encrypted: gglint sees the host and the amount of data, not the content, and
installs no certificate.

## What's kept

Each run is a folder in `~/.gglint/watch/` with an `events.jsonl` file. `Authorization`, API
key and cookie headers are replaced by their length, and a secret found in a request is
replaced by its kind, so the recording holds no credentials. The rest is kept as it was sent,
including your files and prompts, so treat the folder as you would the conversation itself.

```sh
gglint watch --view ~/.gglint/watch/2026-11-25T10-48-08-codex
```

## Options

| Option | What it does |
| --- | --- |
| `-- COMMAND...` | Watch any program, with `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL` and `HTTPS_PROXY` set |
| `--view FOLDER` | Show an earlier run |
| `--port N` | Listen on this port instead of a free one |
| `--dir FOLDER` | Record into this folder |
| `--no-open` | Don't open the page in the browser |
| `--upstream NAME=URL` | Send `anthropic`, `openai` or `chatgpt` requests to another URL, such as a gateway |

Arguments after the agent's name are passed to it: `gglint watch codex exec "fix the test"`.
