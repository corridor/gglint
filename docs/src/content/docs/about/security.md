---
title: Security
description: What gglint reads, writes and runs, and how to report a vulnerability.
---

You can run gglint on any repository, even one you don't trust. Without `--fix` it only reads
files. It never starts an MCP server, installs a package or expands an environment variable, and
the only other program it calls is git, to see which files are tracked, ignored or staged.

It never prints a secret. Environment variables and headers are shown by name, and a secret is
shown only by its kind and location. `gglint hook` adds to an existing hook rather than
replacing it, and the GitHub Action never changes your files. It only needs write access to post
its comment or send results to Code scanning.

If you find a vulnerability, please report it privately through
[GitHub's security advisories](https://github.com/corridor/gglint/security/advisories/new)
rather than in a public issue.
