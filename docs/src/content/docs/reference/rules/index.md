---
title: Rules
description: Every problem gglint reports, which command reports it, and whether --fix fixes it.
---

Each problem gglint reports has a rule name, shown in the `lines`, `github`, `sarif` and
`markdown` [output formats](/gglint/reference/output-formats/).

## MCP servers: `gglint mcp`

| Rule                                                           | Reported when                                                                  | Fixed by `--fix` |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------ | ---------------- |
| [`missing-server`](/gglint/reference/rules/missing-server/)     | A server in one agent's config file is missing from another agent's.           | Yes              |
| [`missing-config`](/gglint/reference/rules/missing-config/)     | An agent has its own directory but no MCP config file, while others have servers. | Yes           |
| [`different-server`](/gglint/reference/rules/different-server/) | The same server is set up differently in two agents.                           | Versions and arguments |
| [`renamed-server`](/gglint/reference/rules/renamed-server/)     | The same server has a different name in some agents.                           | No               |

## Secrets: `gglint secrets`

| Rule                                                             | Reported when                                                              | Fixed by `--fix` |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------- | ---------------- |
| [`inline-secret`](/gglint/reference/rules/inline-secret/)         | Text has the format of a known credential, such as a GitHub token.         | Where the agent can read the environment |
| [`inline-credential`](/gglint/reference/rules/inline-credential/) | A value under a name such as `token` or `password` is written out, a password is in a URL, or a command carries one. | Where the agent can read the environment |
| [`local-file`](/gglint/reference/rules/local-file/)               | An agent file meant for one person's machine is in git, or can be committed. | Adds it to `.gitignore` |

A secret in a file git ignores is a warning, since it isn't committed. Only problems make
gglint exit with status 1.
