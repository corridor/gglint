//! `gglint hook` in a JavaScript repository, with a stand-in for its package manager.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::TempDir;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A stand-in for the package manager `name`, in `dir`, that writes the arguments it's given
/// to `dir/args` and exits with `code`.
fn fake_manager(dir: &Path, name: &str, code: i32) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    if cfg!(windows) {
        let script = format!("@echo off\r\necho %*> \"%~dp0args\"\r\nexit /b {code}\r\n");
        std::fs::write(dir.join(format!("{name}.cmd")), script).unwrap();
    } else {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/args\"\nexit {code}\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    dir.join("args")
}

/// Runs gglint with `bin` first on the PATH.
fn gglint(bin: &Path, args: &[&std::ffi::OsStr]) -> Output {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&path))).unwrap();
    Command::new(env!("CARGO_BIN_EXE_gglint"))
        .args(args)
        .env("PATH", path)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_javascript_repository_gets_gglint_as_a_dev_dependency_then_a_hook_that_runs_it() {
    let dir = TempDir::new();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".husky")).unwrap();
    std::fs::write(repo.join("package.json"), "{}\n").unwrap();
    std::fs::write(repo.join("package-lock.json"), "{}\n").unwrap();
    std::fs::write(repo.join(".husky/pre-commit"), "npx lint-staged\n").unwrap();
    let args = fake_manager(&dir.path().join("bin"), "npm", 0);

    let output = gglint(&dir.path().join("bin"), &["hook".as_ref(), repo.as_os_str()]);

    assert!(
        output.status.success(),
        "{}{}",
        text(&output.stdout),
        text(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(args).unwrap().trim(),
        format!("install --save-dev --save-exact gglint@{VERSION}")
    );
    let script = std::fs::read_to_string(repo.join(".husky/pre-commit")).unwrap();
    assert!(script.starts_with("npx lint-staged\n"), "{script}");
    assert!(script.contains("\nnpx --no -- gglint secrets --staged\n"), "{script}");
    assert!(!script.contains("uvx"), "{script}");
    assert!(
        text(&output.stdout).contains(&format!("Added gglint {VERSION} to the dev dependencies with npm")),
        "{}",
        text(&output.stdout)
    );
}

#[test]
fn gglint_already_in_the_dependencies_isnt_added_again() {
    let dir = TempDir::new();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".husky")).unwrap();
    std::fs::write(
        repo.join("package.json"),
        r#"{ "devDependencies": { "gglint": "0.5.0" } }"#,
    )
    .unwrap();
    std::fs::write(repo.join("pnpm-lock.yaml"), "").unwrap();
    let args = fake_manager(&dir.path().join("bin"), "pnpm", 0);

    let output = gglint(&dir.path().join("bin"), &["hook".as_ref(), repo.as_os_str()]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(!args.exists(), "pnpm was run");
    let script = std::fs::read_to_string(repo.join(".husky/pre-commit")).unwrap();
    assert!(script.contains("pnpm exec gglint secrets --staged\n"), "{script}");
}

#[test]
fn a_package_manager_that_fails_leaves_the_hook_alone() {
    // A hook that runs a gglint the repository doesn't have would stop every commit.
    let dir = TempDir::new();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".husky")).unwrap();
    std::fs::write(repo.join("package.json"), "{}\n").unwrap();
    std::fs::write(repo.join(".husky/pre-commit"), "npx lint-staged\n").unwrap();
    fake_manager(&dir.path().join("bin"), "npm", 1);

    let output = gglint(&dir.path().join("bin"), &["hook".as_ref(), repo.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains(&format!("run `npm install --save-dev --save-exact gglint@{VERSION}`")),
        "{}",
        text(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(repo.join(".husky/pre-commit")).unwrap(),
        "npx lint-staged\n"
    );
}

#[test]
fn print_says_to_add_the_dependency_and_runs_nothing() {
    let dir = TempDir::new();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".husky")).unwrap();
    std::fs::write(repo.join("package.json"), "{}\n").unwrap();
    std::fs::write(repo.join("yarn.lock"), "").unwrap();
    let args = fake_manager(&dir.path().join("bin"), "yarn", 0);

    let output = gglint(
        &dir.path().join("bin"),
        &["hook".as_ref(), "print".as_ref(), repo.as_os_str()],
    );

    assert!(output.status.success());
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains(&format!("yarn add --dev --exact gglint@{VERSION}")),
        "{stdout}"
    );
    assert!(stdout.contains("yarn gglint secrets --staged"), "{stdout}");
    assert!(!args.exists(), "yarn was run");
    assert!(!repo.join(".husky/pre-commit").exists());
}

#[test]
fn a_repository_without_a_package_json_still_runs_uvx() {
    let dir = TempDir::new();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".husky")).unwrap();
    let args = fake_manager(&dir.path().join("bin"), "npm", 0);

    let output = gglint(&dir.path().join("bin"), &["hook".as_ref(), repo.as_os_str()]);

    assert!(output.status.success());
    assert!(!args.exists(), "npm was run");
    let script = std::fs::read_to_string(repo.join(".husky/pre-commit")).unwrap();
    assert!(
        script.contains(&format!("uvx gglint@{VERSION} secrets --staged\n")),
        "{script}"
    );
}
