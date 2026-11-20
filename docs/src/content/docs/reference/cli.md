---
title: Command line
description: Every gglint command, option and exit status.
---

```text
gglint mcp [--fix] [--source AGENT] [--format FORMAT] [PATH]
gglint mcp --servers [PATH]
gglint secrets [--fix] [--staged] [--format FORMAT] [PATH | FILE...]
gglint hook [install|print] [--tool NAME] [--with-mcp] [PATH]
gglint --help
gglint --version
```

`PATH` is the repository's root, and defaults to the current directory.

## `gglint mcp`

Compares the MCP servers of every coding agent in the repository, and fixes the differences.

| Option            | Effect                                                                                                    |
| ----------------- | --------------------------------------------------------------------------------------------------------- |
| *(none)*          | Report every problem and draw each server in each agent.                                                  |
| `--fix`           | Add missing servers and config files, start each server the same way, then report what's left.            |
| `--source AGENT`  | Take each server's setup from `AGENT` where it has the server. `AGENT` is a name from the `--source` column of [Supported agents](/gglint/reference/agents/), such as `claude-code` or `gemini-cli`. |
| `--format FORMAT` | `table` (the default), `lines`, `github`, `sarif` or `markdown`. See [Output formats](/gglint/reference/output-formats/). |
| `--servers`       | List every configured server, file by file, without comparing them.                                       |

| Exit status | Meaning                                                                               |
| ----------- | ------------------------------------------------------------------------------------- |
| `0`         | Every agent gets the same servers, set up the same way.                               |
| `1`         | gglint found a problem, one is left after `--fix`, or there are no MCP config files.  |

It reports the rules [`missing-server`](/gglint/reference/rules/missing-server/),
[`missing-config`](/gglint/reference/rules/missing-config/),
[`different-server`](/gglint/reference/rules/different-server/) and
[`renamed-server`](/gglint/reference/rules/renamed-server/).

## `gglint secrets`

Finds credentials written out in agent config and instruction files, and replaces them with
environment variables.

| Option            | Effect                                                                                     |
| ----------------- | ------------------------------------------------------------------------------------------ |
| *(none)*          | Report every secret in the repository's agent files.                                       |
| `--fix`           | Replace each secret it can with a reference to an environment variable, and add personal agent files to `.gitignore`. |
| `--staged`        | Read only what's staged in git: the copy the commit gets.                                  |
| `--format FORMAT` | `table` (the default), `lines`, `github`, `sarif` or `markdown`.                           |
| `FILE...`         | Check only these files, where they're agent files, as a hook passes them.                  |

| Exit status | Meaning                                                                                  |
| ----------- | ---------------------------------------------------------------------------------------- |
| `0`         | No problem, or only warnings, such as a secret in a file git ignores.                    |
| `1`         | gglint found a problem.                                                                  |

It reports the rules [`inline-secret`](/gglint/reference/rules/inline-secret/),
[`inline-credential`](/gglint/reference/rules/inline-credential/) and
[`local-file`](/gglint/reference/rules/local-file/).

## `gglint hook`

Runs `gglint secrets --staged` before every commit. See
[Check before every commit](/gglint/guides/pre-commit/).

| Option        | Effect                                                                        |
| ------------- | ----------------------------------------------------------------------------- |
| `install`     | The default: add the hook to husky or git, or print the lines for other tools. |
| `print`       | Only print the lines to add.                                                  |
| `--tool NAME` | `husky`, `git`, `pre-commit`, `lefthook` or `lint-staged`, instead of detecting one. |
| `--with-mcp`  | Also run `gglint mcp`.                                                        |

## Output

- Colours are used when standard output is a terminal and `NO_COLOR` isn't set.
- The summary line, with counts and timing, goes to standard error, so standard output holds
  only the report.
