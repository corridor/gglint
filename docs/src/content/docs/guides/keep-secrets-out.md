---
title: Keep secrets out of agent files
description: Find credentials written out in coding-agent config and instruction files, replace them with environment variables, and keep personal files out of git.
---

The files that configure coding agents are committed, so a token pasted into `.mcp.json` ends
up in git history. `gglint secrets` finds them in every supported agent's MCP config files,
settings, hooks, rules and instruction files such as `CLAUDE.md`.

## Find them

```sh
uvx gglint secrets
```

```text
.claude/settings.local.json
  !   1:1  git doesn't ignore this file, so it can be committed by accident; it's Claude Code's
           personal settings, where it saves the commands you allow, secrets and all
  ✗  1:47  credential in "PGPASSWORD" is written out in the file

.cursor/mcp.json
  ✗  5:45  GitHub token in "Authorization" is written out in the file
  ✗  9:83  password in a URL is written out in the file
```

Each finding says what to do about it. `✗` is a problem and fails the check; `!` is a warning,
such as a secret in a file git ignores, which isn't committed.

To check other files, or only some, name them:

```sh
uvx gglint secrets --staged        # only what's staged in git: the copy the commit gets
uvx gglint secrets FILE...         # only these, where they're agent files
```

## Replace them

```sh
uvx gglint secrets --fix
```

```text
Replaced 1 secret(s) with references to environment variables:
  ./.cursor/mcp.json  (1)

Set those variables in your environment. Anything already committed stays in git history: rotate it.

Added to ./.gitignore:
  .claude/settings.local.json
```

`--fix` writes the reference in the agent's own syntax, such as `Bearer ${env:GITHUB_TOKEN}` in
Cursor, and keeps everything else in the file as it was. Then:

1. **Set the variables** it names in your environment, or in your agent's own settings.
2. **Rotate every secret that was committed.** It's still in git history.
3. **Untrack personal files** already in git: `git rm --cached .claude/settings.local.json`.

## What you fix by hand

Some secrets can't be replaced automatically. gglint reports them and says what to use instead:

- **A secret in a command**, such as one Claude Code saved as allowed
  (`Bash(PGPASSWORD=… psql)`). Take it out of the command and pass it from your environment,
  or remove the command the agent saved, with `/permissions` in Claude Code.
- **A password in a URL**, such as `postgresql://admin:…@db/app`. Read the URL, or the
  password, from an environment variable.
- **An agent with no way to refer to an environment variable**, such as Codex in an
  `http_headers` table. Use `env_http_headers` or `bearer_token_env_var` instead.

## Stop the next one

- [Check before every commit](/gglint/guides/pre-commit/) with `uvx gglint hook`.
- [Check every pull request](/gglint/guides/github-action/) with the GitHub Action.

[How secrets are found](/gglint/concepts/secrets/) explains what gglint looks for and what it
can miss.
