---
title: Secrets
description: gglint secrets finds credentials written into coding-agent files, replaces them with environment variables, and keeps personal agent files out of git.
---

Secrets end up in agent files without anyone meaning to put them there. When you approve a
command in Claude Code, it's saved in `.claude/settings.local.json` exactly as you ran it, token
included. A token pasted into `.mcp.json` to get a server working gets committed with
everything else. `gglint secrets` looks for these in every agent's config, settings, hooks,
rules and instruction files, such as `CLAUDE.md` and `AGENTS.md`.

```sh
uvx gglint secrets
```

```text
.claude/settings.local.json
  !   1:1  git doesn't ignore this file, so it can be committed by accident
  ✗  1:47  credential in "PGPASSWORD" is written out in the file

.cursor/mcp.json
  ✗  5:45  GitHub token in "Authorization" is written out in the file
```

A `✗` fails the check. A `!` is a warning, for example a secret in a file git already ignores.
gglint never prints the secret itself, only what kind it is and where, so the output is safe to
paste into an issue or leave in a CI log.

It finds three kinds of problem: a token in a known format, such as `ghp_…`
([`inline-secret`](/gglint/reference/rules/inline-secret/)); a password or key with no known
format, under a name like `token`, in a URL or in a saved command
([`inline-credential`](/gglint/reference/rules/inline-credential/)); and a personal file such as
`.claude/settings.local.json` that git doesn't ignore
([`local-file`](/gglint/reference/rules/local-file/)).

## Fixing it

```sh
uvx gglint secrets --fix
```

Each secret it can replace becomes a reference to an environment variable, written the way that
agent expects, such as `Bearer ${env:GITHUB_TOKEN}` for Cursor. Personal files are added to
`.gitignore`. After that, set the variables it names in your environment, and rotate any secret
that was ever committed, because it's still in git history. A personal file that's already in
git also needs `git rm --cached`.

A few things it can't fix for you. If an agent saved a command with a secret in it, remove the
command (`/permissions` in Claude Code) and pass the secret from your environment instead. If a
password is inside a URL, read the URL from an environment variable. And if the agent has no way
to read an environment variable in that spot, gglint tells you what to use instead.

## What it skips

To avoid false alarms, it ignores placeholders like `your-token-here`, references like
`${GITHUB_TOKEN}`, passwords shorter than 8 characters, and passwords in commands that have no
digit or symbol. It doesn't scan the rest of your code or your git history; regular secret
scanners already do that. Run over 441 public repositories set up for coding agents, it found 7
secrets and 21 committed `.claude/settings.local.json` files, with no false alarms.
