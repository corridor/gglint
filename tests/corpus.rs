//! `gglint mcp --fix` over real repositories: what it must never do, whatever it finds.
//!
//! Each directory under `GGLINT_CORPUS` is a repository (a clone, or just its agent files).
//! Its MCP config files and the agents' directories are copied to a scratch directory, so
//! the repositories themselves are never changed. Ignored by default; run with:
//!
//! ```sh
//! GGLINT_CORPUS=path/to/repos cargo test --release --test corpus -- --ignored --nocapture
//! ```

mod common;

use std::path::Path;

use common::TempDir;
use gglint::harness::{McpConfig, check_mcp, fix_mcp, read_mcp};

const FILES: &[&str] = &[
    ".mcp.json",
    ".codex/config.toml",
    ".cursor/mcp.json",
    "opencode.json",
    "opencode.jsonc",
];
/// The agents' own directories: `--fix` creates a config file only where one is.
const DIRECTORIES: &[&str] = &[".claude", ".codex", ".cursor", ".opencode"];

/// A scratch copy of what `gglint mcp` reads in `repo`: its MCP files and, empty, the
/// agents' directories.
fn copy(repo: &Path) -> TempDir {
    let dir = TempDir::new();
    for directory in DIRECTORIES {
        if repo.join(directory).is_dir() {
            std::fs::create_dir_all(dir.path().join(directory)).unwrap();
        }
    }
    for file in FILES {
        if let Ok(text) = std::fs::read(repo.join(file)) {
            dir.write(file, text);
        }
    }
    dir
}

fn warnings(configs: &[McpConfig]) -> Vec<String> {
    let mut all: Vec<String> = configs
        .iter()
        .flat_map(|config| {
            config
                .warnings
                .iter()
                .map(|warning| format!("{}: {warning}", config.relative_path.display()))
        })
        .collect();
    all.sort();
    all
}

fn problems(root: &Path, configs: &[McpConfig]) -> usize {
    check_mcp(root, configs).findings().count()
}

#[test]
#[ignore = "needs GGLINT_CORPUS"]
fn fix_keeps_its_promises_on_real_repositories() {
    let Some(corpus) = std::env::var_os("GGLINT_CORPUS") else {
        eprintln!("skipped: set GGLINT_CORPUS");
        return;
    };
    let mut repos: Vec<_> = std::fs::read_dir(corpus)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    repos.sort();
    let mut broken = Vec::new();
    let (mut before_total, mut after_total, mut created, mut edited, mut added, mut updated, mut left) =
        (0, 0, 0, 0, 0, 0, 0);
    for repo in &repos {
        let name = repo.file_name().unwrap().to_string_lossy().to_string();
        let dir = copy(repo);
        let root = dir.path();
        let before = read_mcp(root);
        let report = fix_mcp(root, &before, &check_mcp(root, &before));
        let after = read_mcp(root);
        let mut fail = |what: String| broken.push(format!("{name}: {what}"));

        let new_warnings: Vec<_> = warnings(&after)
            .into_iter()
            .filter(|w| !warnings(&before).contains(w))
            .collect();
        if !new_warnings.is_empty() {
            fail(format!("left a config it can't read: {new_warnings:?}"));
        }
        for config in after
            .iter()
            .filter(|config| !before.iter().any(|old| old.relative_path == config.relative_path))
        {
            if !root.join(config.harness.directory()).is_dir() {
                fail(format!(
                    "created {} without a {} directory",
                    config.relative_path.display(),
                    config.harness.directory()
                ));
            }
        }
        for config in &before {
            let Some(now) = after.iter().find(|other| other.relative_path == config.relative_path) else {
                fail(format!("{} disappeared", config.relative_path.display()));
                continue;
            };
            for server in &config.servers {
                match now.servers.iter().find(|other| other.name == server.name) {
                    None => fail(format!(
                        "removed `{}` from {}",
                        server.name,
                        config.relative_path.display()
                    )),
                    // A command line may change; nothing else may.
                    Some(kept)
                        if kept.env != server.env
                            || kept.headers != server.headers
                            || kept.enabled != server.enabled =>
                    {
                        fail(format!(
                            "changed `{}` in {}",
                            server.name,
                            config.relative_path.display()
                        ))
                    }
                    Some(_) => {}
                }
            }
        }
        let (problems_before, problems_after) = (problems(root, &before), problems(root, &after));
        if problems_after > problems_before {
            fail(format!("{problems_before} problems before, {problems_after} after"));
        }
        let again = fix_mcp(root, &after, &check_mcp(root, &after));
        if !again.changed.is_empty() {
            fail("a second run changed files again".to_string());
        }
        before_total += problems_before;
        after_total += problems_after;
        created += report.changed.iter().filter(|file| file.created).count();
        edited += report.changed.iter().filter(|file| !file.created).count();
        added += report.changed.iter().map(|file| file.added.len()).sum::<usize>();
        updated += report.changed.iter().map(|file| file.updated.len()).sum::<usize>();
        left += report.unfixed.len();
    }

    eprintln!(
        "{} repositories; {before_total} problems before --fix, {after_total} after; {added} servers added, \
         {updated} command lines updated, {edited} files edited, {created} created; {left} not fixed",
        repos.len()
    );
    assert!(
        broken.is_empty(),
        "{} broken promises:\n{}",
        broken.len(),
        broken.join("\n")
    );
}
