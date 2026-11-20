---
title: Contributing
description: How to build gglint, run its tests, and add support for another coding agent.
---

gglint is written in Rust and developed on
[GitHub](https://github.com/corridor/gglint). Bug reports, new agents and fixes are welcome.

- **Report a bug or ask for an agent** in
  [an issue](https://github.com/corridor/gglint/issues). For a wrong report or fix, include
  the config files involved, with secrets taken out.
- **Change the code** by following
  [CONTRIBUTING.md](https://github.com/corridor/gglint/blob/main/CONTRIBUTING.md): how to
  build, test and open a pull request.
- **Find your way around the code** with
  [ARCHITECTURE.md](https://github.com/corridor/gglint/blob/main/ARCHITECTURE.md): what each
  module does, and where to start for a new agent or rule.
- **Improve these docs.** Every page has an *Edit page* link at the bottom. The site lives in
  [`docs/`](https://github.com/corridor/gglint/tree/main/docs).

## Build and test

With a [Rust toolchain](https://rustup.rs) 1.88 or later, run these three commands in order
before each commit; CI runs the same checks:

```sh
cargo fmt
cargo clippy --all-targets -- --deny warnings
cargo test
```
