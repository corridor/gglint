---
title: On every pull request
description: Run gglint on every pull request with its GitHub Action, for annotations on the diff, one comment, and optional Code scanning.
---

To check every pull request, save this as `.github/workflows/gglint.yml`:

```yaml
name: gglint
on: [pull_request]

permissions:
  contents: read
  pull-requests: write   # for the comment

jobs:
  gglint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: corridor/gglint@v1
```

On the next pull request, problems show up as notes on the changed lines, gglint leaves one
comment that it updates on every push, and the check fails until they're fixed. The action only
reports. It never changes your files. Pull requests from forks can't post the comment, but they
still get the notes and the failing check.

`@v1` always runs the latest release. Pin `@v0.5.1` if you'd rather update on your own schedule.

## Options

| Input              | Default               | What it does                                             |
| ------------------ | --------------------- | -------------------------------------------------------- |
| `command`          | `all`                 | Which check to run: `all`, `mcp` or `secrets`            |
| `comment`          | `true`                | Post a comment. Needs `pull-requests: write`.            |
| `fail-on-problems` | `true`                | Set to `false` to report without failing                 |
| `sarif`            | `false`               | Also send results to Code scanning. Needs `security-events: write`. |
| `version`          | the action's release  | Which gglint version to run                              |
| `path`             | `.`                   | Where the repository is checked out                      |
| `token`            | `${{ github.token }}` | The token used to post the comment                       |

For example, to check only for secrets and skip the comment:

```yaml
      - uses: corridor/gglint@v1
        with:
          command: secrets
          comment: "false"
```

On another CI, run gglint directly. `--format github` gives GitHub-style annotations, and
[other formats](/gglint/reference/output-formats/) are available:

```yaml
- run: uvx gglint@0.5.1 mcp --format github
- run: uvx gglint@0.5.1 secrets --format github
```
