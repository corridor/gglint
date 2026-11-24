---
title: Contributing
description: Report a bug, ask for an agent, or change gglint's code and docs.
---

gglint is written in Rust and lives on [GitHub](https://github.com/corridor/gglint). If
something looks wrong or you want support for another agent, open
[an issue](https://github.com/corridor/gglint/issues). For a wrong report, include the config
files involved, with any secrets taken out.

To work on the code, start with
[CONTRIBUTING.md](https://github.com/corridor/gglint/blob/main/CONTRIBUTING.md) and
[ARCHITECTURE.md](https://github.com/corridor/gglint/blob/main/ARCHITECTURE.md). Before you
commit, run the same checks CI does:

```sh
cargo fmt
cargo clippy --all-targets -- --deny warnings
cargo test
```

These docs live in [`docs/`](https://github.com/corridor/gglint/tree/main/docs), and every page
has an *Edit page* link at the bottom.
