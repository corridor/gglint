---
title: Security
description: What gglint reads, writes and runs, and why it's safe to run on an untrusted repository.
---

- **Read-only by default.** Without `--fix`, gglint reads files and writes nothing.
- **Nothing is executed or fetched.** MCP servers aren't started, packages aren't resolved,
  and environment variables aren't expanded. It's safe to run on an untrusted repository.
- **Secrets stay out of the output.** Environment variables and headers are printed by name
  only, and `gglint secrets` shows a secret's kind, the name it's under and where it is, never
  its value.
- **One external program: git.** gglint runs `git check-ignore` to mark files your
  `.gitignore` excludes (in `mcp --fix` and in `secrets`), `git ls-files` to see whether a
  personal agent file is in git, `git diff --cached` and `git show` to read what's staged for
  `secrets --staged`, and `git rev-parse --git-path hooks` for `gglint hook`.
- **Nothing is overwritten by `hook`.** An existing hook script is extended, once.
- **The GitHub Action only reports.** It never changes a file, and needs write permissions
  only for the features that post: the comment and Code scanning.

## Reporting a vulnerability

Please report a vulnerability privately through
[GitHub's security advisories](https://github.com/corridor/gglint/security/advisories/new)
rather than in a public issue.
