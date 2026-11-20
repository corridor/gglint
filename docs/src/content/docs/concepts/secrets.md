---
title: How secrets are found
description: Which files gglint secrets reads, what it looks for, how it avoids false alarms, and what it can miss.
---

`gglint secrets` reads the files that configure coding agents: for every
[supported agent](/gglint/reference/agents/), its MCP config files, settings, hooks, rules,
commands and instruction files such as `CLAUDE.md`, `AGENTS.md` and `.cursorrules`. Each agent's
files are listed in [Files `gglint secrets` reads](/gglint/reference/agents/#files-gglint-secrets-reads).
It doesn't read the rest of the repository; general secret scanners already do that.

## Three ways a secret is found

1. **By its format** ([`inline-secret`](/gglint/reference/rules/inline-secret/)). Tokens with a
   known shape, such as `ghp_…` for GitHub or `AKIA…` for AWS, are found anywhere in an agent
   file.
2. **By where it's written** ([`inline-credential`](/gglint/reference/rules/inline-credential/)).
   A value under a name such as `token` or `password`, a password in a URL, or a credential in
   a command, as agents save the commands you allow.
3. **By the file it's in** ([`local-file`](/gglint/reference/rules/local-file/)). A personal
   file such as `.claude/settings.local.json` shouldn't be in git at all, whatever it holds, so
   a secret no pattern recognises is stopped too.

## Avoiding false alarms

- Placeholders (`your-token-here`, `ghp_xxxx…`) aren't reported.
- References (`${GITHUB_TOKEN}`) and values that name a variable
  (`bearer_token_env_var = "GITHUB_TOKEN"`) aren't reported.
- A secret in a file git ignores is a warning, since it isn't committed. Only problems fail.

Run over 441 public repositories set up for coding agents, `gglint secrets` found 7 secrets
and 21 committed `.claude/settings.local.json` files, and no false alarms.

## The value is never shown

A secret's value is never printed, stored or sent. Findings carry its kind, the name it's under
and where it is, so the output is safe to put in a CI log or a pull request comment.

## What it can miss

gglint reads a file line by line. It misses:

- a password made of one kind of letters (`"hunter"`) or shorter than 8 characters;
- in a command, a password with no digit or symbol (`app-reader`), as commands and the text
  around them are full of words and names;
- secrets in `.env` files or other source, which it doesn't read. An agent's own `.env`
  file, such as `.gemini/.env`, is reported once it's in git, but not read;
- secrets in git history, which it doesn't search. A secret that was ever committed has to be
  rotated.
