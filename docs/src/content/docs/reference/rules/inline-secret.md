---
title: inline-secret
description: Text in a coding agent's config or instruction file has the format of a known credential, such as a GitHub token or an AWS access key ID.
---

**Command:** `gglint secrets` · **Severity:** problem, or a warning in a file git ignores ·
**Fixed by `--fix`:** where the agent can read the environment

## What it finds

Text in the format of a known credential, in any agent file:

- GitHub, GitLab, Anthropic, Slack, Stripe, Google and npm tokens;
- AWS access key IDs;
- API keys starting with `sk-`;
- private key blocks.

## Why it matters

Agent files are committed, so a token pasted into `.mcp.json` ends up in git history, where
anyone with access to the repository, or to a fork or a clone, can read it.

## Example

```json
// .cursor/mcp.json
"github": {
  "url": "https://api.githubcopilot.com/mcp/",
  "headers": { "Authorization": "Bearer ghp_…" }
}
```

```text
./.cursor/mcp.json:5:45  inline-secret    GitHub token in "Authorization" is written out in the file
```

The secret's value is never printed: gglint shows its kind, the name it's under and where it
is.

## How to fix it

Run `uvx gglint secrets --fix`. Where the agent can read an environment variable there, the
value becomes a reference in its syntax:

```diff
-      "headers": { "Authorization": "Bearer ghp_…" }
+      "headers": { "Authorization": "Bearer ${env:GITHUB_TOKEN}" }
```

Then set `GITHUB_TOKEN` in your environment, and **rotate the token**: once committed, it's in
git history.

## When it isn't reported

Placeholders (`your-token-here`, `ghp_xxxx…`) and references (`${GITHUB_TOKEN}`) aren't
reported. In a file git ignores, a secret is a warning, since it isn't committed.
