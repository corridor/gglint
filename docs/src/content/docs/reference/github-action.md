---
title: GitHub Action
description: Every input of the corridor/gglint GitHub Action, and the permissions each feature needs.
---

```yaml
- uses: corridor/gglint@v1
  with:
    command: all
```

The action runs gglint with [uv](https://docs.astral.sh/uv/), and doesn't start a server or
fetch anything else. It only reports: it never changes a file. For a walkthrough, see
[Check every pull request](/gglint/guides/github-action/).

## Inputs

| Input              | Default                         | Meaning                                                           |
| ------------------ | ------------------------------- | ----------------------------------------------------------------- |
| `command`          | `all`                           | `secrets`, `mcp` or `all`.                                        |
| `path`             | `.`                             | The repository's root, for a repository checked out elsewhere.    |
| `version`          | the one `@v1` was released with | The gglint version to run, from PyPI. Pin it for repeatable runs. |
| `fail-on-problems` | `true`                          | Set to `false` to report without failing the check.               |
| `comment`          | `true`                          | Post the findings as one pull request comment, updated on each push. |
| `sarif`            | `false`                         | Upload the findings to Code scanning.                             |
| `token`            | `${{ github.token }}`           | The token used to post the comment.                               |

## Permissions

| Feature                     | Needs                                                               |
| --------------------------- | ------------------------------------------------------------------- |
| Annotations, failing check  | `contents: read`                                                    |
| The comment                 | `pull-requests: write`. A pull request from a fork can't post it.   |
| Code scanning (`sarif`)     | `security-events: write`, and Code scanning on a private repository |

## Versions

The `v1` tag is moved to each new release of gglint, so `corridor/gglint@v1` runs the latest
release. To stay on one, pin the action to its release tag, such as `@v0.5.1`, or set
`version`.
