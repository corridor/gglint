"""Builds gglint-pre-commit, the repository pre-commit installs gglint's hooks from.

    python pre-commit/build.py --version 0.5.0 --out DIR

pre-commit installs a Python hook by pip-installing the repository it comes from. This one
would make pip build gglint from source, with Rust, so the hooks are published from
corridor/gglint-pre-commit instead, which holds only the files this writes: a
`pyproject.toml` that depends on gglint==VERSION, which pip installs as a wheel from PyPI,
and the hooks of this repository's `.pre-commit-hooks.yaml`, started as `gglint` rather
than `uvx gglint@VERSION`. pre-commit is a Python program, so a machine that runs it needs
nothing else.

gglint-pre-commit publishes each release itself: a workflow there looks for gglint's newest
version on PyPI every half hour, runs this script from that version's tag, commits what it
writes and tags the commit vVERSION, the `rev` a `.pre-commit-config.yaml` names.
"""

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPOSITORY = "https://github.com/corridor/gglint-pre-commit"

HOOKS_HEADER = """\
# gglint's hooks for pre-commit (https://pre-commit.com), from
# https://github.com/corridor/gglint/blob/v{version}/.pre-commit-hooks.yaml
# pre-commit installs gglint {version} from PyPI to run them. Written by pre-commit/build.py.

"""

PYPROJECT = """\
# pre-commit pip-installs this repository to run its hooks, which installs gglint.
[project]
name = "gglint-pre-commit"
version = "{version}"
description = "gglint's hooks for pre-commit"
dependencies = ["gglint=={version}"]

[build-system]
requires = ["setuptools>=61"]
build-backend = "setuptools.build_meta"

# Nothing of its own to install.
[tool.setuptools]
packages = []
py-modules = []
"""

README = """\
# gglint-pre-commit

[gglint](https://github.com/corridor/gglint)'s hooks for [pre-commit](https://pre-commit.com)
and [prek](https://github.com/j178/prek). They stop a commit that writes a secret into the
config or instruction files of a coding agent, such as `.mcp.json`, `.cursor/mcp.json` or
`CLAUDE.md`, and can check that every agent in the repository gets the same MCP servers.

```yaml
# .pre-commit-config.yaml
repos:
  - repo: {repository}
    rev: v{version}
    hooks:
      - id: gglint-secrets
      # Also check that every agent gets the same MCP servers:
      # - id: gglint-mcp
```

pre-commit installs gglint from PyPI, as a program built for the machine, so nothing else is
needed: not uv, Node or Rust. It runs on Linux, macOS and Windows.

| Hook             | Runs                        | When a commit changes                       |
| ---------------- | --------------------------- | ------------------------------------------- |
| `gglint-secrets` | `gglint secrets FILE...`    | an agent's config or instruction file       |
| `gglint-mcp`     | `gglint mcp --format lines` | an agent's MCP config file                  |

Each gglint release is published here within the hour, tagged with its version, and
`pre-commit autoupdate` moves to the newest. The files are written from gglint's own
repository: send issues and changes to [corridor/gglint](https://github.com/corridor/gglint),
and see [the documentation](https://corridor.github.io/gglint/guides/pre-commit/).
"""


def hooks(version: str) -> str:
    """This repository's hooks, started by the gglint pre-commit installs."""
    text = (ROOT / ".pre-commit-hooks.yaml").read_text()
    # The header says how to use this repository's hooks, with uvx; the mirror has its own.
    body = text[text.index("\n- id: ") + 1 :]
    pinned = f"uvx gglint@{version} "
    lines = []
    for line in body.splitlines():
        key, _, value = line.strip().partition(": ")
        if key == "entry":
            if not value.startswith(pinned):
                sys.exit(f"error: .pre-commit-hooks.yaml runs {value!r}, not {pinned}...")
            line = line.replace(pinned, "gglint ", 1)
        elif key == "language":
            if value != "system":
                sys.exit(f"error: a hook in .pre-commit-hooks.yaml has language {value!r}, not system")
            line = line.replace("system", "python", 1)
        lines.append(line)
    return HOOKS_HEADER.format(version=version) + "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    files = {
        ".pre-commit-hooks.yaml": hooks(args.version),
        "pyproject.toml": PYPROJECT.format(version=args.version),
        "README.md": README.format(version=args.version, repository=REPOSITORY),
    }
    args.out.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (args.out / name).write_text(text, newline="\n")
        print(args.out / name)


if __name__ == "__main__":
    main()
