# Contributing to gglint

Thanks for helping. Bug reports, support for more agents, fixes and documentation are all
welcome.

## Reporting a problem

Open an [issue](https://github.com/corridor/gglint/issues). For a wrong report or a wrong fix,
include the agent config files involved, with any secrets taken out, and the output of
`gglint mcp --format lines` or `gglint secrets --format lines`.

To report a vulnerability, use
[a private security advisory](https://github.com/corridor/gglint/security/advisories/new)
instead of an issue.

## Setting up

gglint is written in Rust. Install a [Rust toolchain](https://rustup.rs) 1.88 or later, then:

```sh
git clone https://github.com/corridor/gglint
cd gglint
cargo run -- mcp path/to/a/repository
```

[ARCHITECTURE.md](ARCHITECTURE.md) explains how the code is laid out, and where to start to
support another agent or add a rule.

## Before each commit

Run these three commands in order; CI runs the same checks on Linux, macOS and Windows:

```sh
cargo fmt
cargo clippy --all-targets -- --deny warnings
cargo test
```

`tests/corpus.rs` runs `--fix` against a directory of real repositories and checks that no
server is removed, that no environment variable, header or on/off state changes, that a
config file is created only in an agent's own directory, that no file becomes unreadable,
that the number of missing servers and differences never grows, and that a second run changes
nothing. Run it for changes to reading, comparing or fixing:

```sh
GGLINT_CORPUS=path/to/clones cargo test --release --test corpus -- --ignored --nocapture
```

## Opening a pull request

`main` only changes through pull requests.

- Keep a pull request to one change, with tests for it.
- Add the change to the `[Unreleased]` section of [CHANGELOG.md](CHANGELOG.md).
- Update the docs: the README, and the page in `docs/` that describes what you changed.
- When it fixes an issue, say `Closes #N` in the description.

## Working on the docs

The documentation site lives in [`docs/`](docs/) and is built with
[Starlight](https://starlight.astro.build). With Node.js 22.23 or later:

```sh
cd docs
npm ci
npm run dev
```

and open the address it prints. Pages are Markdown files in `docs/src/content/docs/`, grouped
as tutorials (`start/`), how-to guides (`guides/`), reference (`reference/`) and explanation
(`concepts/`). The site is published from `main` by `.github/workflows/docs.yml`.

## Releasing

1. Update `version` in `Cargo.toml` and add the release to `CHANGELOG.md`.
2. Merge to `main`.
3. Publish a GitHub release tagged `v<version>`, for example `v0.4.5`.
4. Move the `v1` tag to the release's commit, so `uses: corridor/gglint@v1` runs it:
   `git tag -f v1 v<version> && git push -f origin v1`.

The [release workflow](.github/workflows/pypi_release.yml) builds and tests every wheel,
publishes them to PyPI, and attaches them to the release.

## License

By contributing, you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE).
