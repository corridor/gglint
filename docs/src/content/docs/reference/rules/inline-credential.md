---
title: inline-credential
description: A password, token or key is written out under a name such as token or password, in a URL, or in a command saved by a coding agent.
---

**Command:** `gglint secrets` · **Severity:** problem, or a warning in a file git ignores ·
**Fixed by `--fix`:** where the agent can read the environment

## What it finds

A credential that has no known format, found by where it's written:

- **Under a name.** A JSON or TOML value under a name such as `token`, `password`, `api_key`
  or `Authorization`, written out instead of being a reference to an environment variable.
- **In a URL.** A password in a URL, such as `postgresql://admin:…@db/app`.
- **In a command**, as agents save the commands you allow: a header
  (`curl -H "Authorization: Bearer …"`), a variable set for the command (`PGPASSWORD=… psql`)
  or an option (`--token …`, `sshpass -p …`, `mysql -p…`).

## Why it matters

Approving a command in Claude Code saves it, keys and all, in `.claude/settings.json` or
`.claude/settings.local.json`, and other agents keep commands in their settings and hooks too. These secrets have no fixed format, so a scanner that looks
only for token formats misses them.

## Example

```json
// .claude/settings.local.json
{ "permissions": { "allow": ["Bash(PGPASSWORD=… psql -h db -U app)"] } }
```

```text
./.claude/settings.local.json:1:47  inline-credential credential in "PGPASSWORD" is written out in the file
```

## How to fix it

- **Under a name:** `uvx gglint secrets --fix` replaces it with a reference to an environment
  variable, where the agent has a way to refer to one.
- **In a URL:** read the URL, or the password, from an environment variable.
- **In a command:** take it out of the command and pass it from your environment. If an agent
  saved the command as allowed, remove it there, with `/permissions` in Claude Code.

Rotate every credential that was committed.

## When it isn't reported

References (`${DB_PASSWORD}`) and values that name a variable
(`bearer_token_env_var = "GITHUB_TOKEN"`) aren't reported. To keep false alarms down, a
password made of one kind of letters (`"hunter"`) or shorter than 8 characters isn't reported,
nor, in a command, one with no digit or symbol. See
[How secrets are found](/gglint/concepts/secrets/).
