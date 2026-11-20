---
title: Check every pull request
description: Run gglint on every pull request with its GitHub Action, for annotations on the diff, one comment, and optional Code scanning.
---

The gglint action checks every pull request for both problems. It only reports: it never
changes a file.

## Add the workflow

Save this as `.github/workflows/gglint.yml`:

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

On the next pull request you get:

- **Annotations.** Each problem appears on its line in the pull request's *Files changed*.
- **One comment.** The problems, and a table of which agent gets which MCP server, in a single
  comment that's updated on every push, and left alone when a pull request has no problems.
- **A failing check** while a problem is left.

A pull request from a fork can't post the comment, and still gets the annotations and the
failing check. A repository with no MCP config files has nothing to compare, so `mcp` is
skipped with a notice.

## Only check for secrets

```yaml
      - uses: corridor/gglint@v1
        with:
          command: secrets
          comment: "false"
```

With `comment: "false"` the workflow only needs `contents: read`.

## Send findings to Code scanning

With `sarif: true`, the findings also go to *Security*, *Code scanning*, where they can be
tracked and dismissed:

```yaml
permissions:
  contents: read
  pull-requests: write
  security-events: write

# ...
      - uses: corridor/gglint@v1
        with:
          sarif: true
```

On a private repository, Code scanning has to be enabled.

## Report without failing

```yaml
      - uses: corridor/gglint@v1
        with:
          fail-on-problems: "false"
```

## Run gglint without the action

In any CI, run gglint with uv and the `github` format, or another
[output format](/gglint/reference/output-formats/):

```yaml
      - uses: astral-sh/setup-uv@v7
      - run: uvx gglint@0.4.5 mcp --format github
      - run: uvx gglint@0.4.5 secrets --format github
```

Every input of the action is listed in the [GitHub Action reference](/gglint/reference/github-action/).
