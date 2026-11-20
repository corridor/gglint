//! `gglint hook`: runs gglint on every commit, through whichever hook tool a repository uses.
//!
//! husky and plain git hooks are installed by writing a script, never overwriting one that
//! exists: the command is appended once. For pre-commit, lefthook and lint-staged, whose
//! configuration is YAML or JSON that gglint doesn't edit, it prints the lines to add.
//!
//! A JavaScript repository, one with a `package.json`, gets gglint as a dev dependency,
//! added with its own package manager, so every contributor has it after installing the
//! repository's packages, and its hook runs that copy. Any other repository's hook runs
//! `uvx gglint@<version>`.

use std::path::{Path, PathBuf};
use std::process::Command;

use jsonc_parser::JsonValue;
use jsonc_parser::ParseOptions;

/// A tool that runs commands before a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    PreCommit,
    Husky,
    Lefthook,
    LintStaged,
    /// A plain `pre-commit` script in git's hooks directory.
    Git,
}

pub const TOOLS: [Tool; 5] = [
    Tool::PreCommit,
    Tool::Husky,
    Tool::Lefthook,
    Tool::LintStaged,
    Tool::Git,
];

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::PreCommit => "pre-commit",
            Tool::Husky => "husky",
            Tool::Lefthook => "lefthook",
            Tool::LintStaged => "lint-staged",
            Tool::Git => "git",
        }
    }

    pub fn parse(name: &str) -> Option<Tool> {
        let name = name.to_ascii_lowercase();
        TOOLS.into_iter().find(|tool| tool.as_str() == name)
    }
}

/// What `install` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Wrote or extended this file.
    Wrote(PathBuf),
    /// The file already runs gglint.
    AlreadyThere(PathBuf),
    /// gglint doesn't edit this tool's configuration: add `snippet` to `file`.
    Print { file: &'static str, snippet: String },
}

/// gglint's package on npm. npm refuses `gglint` alone, as too close to eslint, oxlint and
/// tslint; its command is still `gglint`.
pub const NPM_PACKAGE: &str = "@gglint/cli";

/// A JavaScript repository's package manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    pub fn as_str(self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun",
        }
    }

    /// The arguments that add this version of gglint to the repository's dev dependencies,
    /// pinned exactly, as a hook should run what the repository chose.
    fn add_args(self) -> Vec<String> {
        let package = format!("{NPM_PACKAGE}@{}", env!("CARGO_PKG_VERSION"));
        let options: &[&str] = match self {
            PackageManager::Npm => &["install", "--save-dev", "--save-exact"],
            PackageManager::Pnpm => &["add", "--save-dev", "--save-exact"],
            PackageManager::Yarn | PackageManager::Bun => &["add", "--dev", "--exact"],
        };
        options
            .iter()
            .map(|option| option.to_string())
            .chain([package])
            .collect()
    }

    /// The command that adds gglint to the repository's dev dependencies, as a person types it.
    pub fn add_command(self) -> String {
        format!("{} {}", self.as_str(), self.add_args().join(" "))
    }

    /// How a hook runs the gglint in the repository's dev dependencies, and nothing else: it
    /// doesn't download a gglint the repository doesn't list.
    fn exec(self) -> &'static str {
        match self {
            PackageManager::Npm => "npx --no -- gglint",
            PackageManager::Pnpm => "pnpm exec gglint",
            PackageManager::Yarn => "yarn gglint",
            PackageManager::Bun => "bun run gglint",
        }
    }
}

/// How a hook starts gglint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    /// `uvx gglint@<version>`, pinned to this version.
    Uvx,
    /// The gglint in a JavaScript repository's dev dependencies.
    Package(PackageManager),
}

fn package_json(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join("package.json")).ok()
}

/// The package manager of the JavaScript repository at `root`, or `None` when it has no
/// `package.json`: the one its `packageManager` field names, else the one its lockfile is
/// from, else npm.
pub fn package_manager(root: &Path) -> Option<PackageManager> {
    let text = package_json(root)?;
    let named = jsonc_parser::parse_to_value(&text, &ParseOptions::default())
        .ok()
        .flatten()
        .and_then(|value| match value {
            JsonValue::Object(object) => object.get_string("packageManager").map(|name| name.to_string()),
            _ => None,
        });
    let from_name = named.and_then(|name| {
        [
            PackageManager::Npm,
            PackageManager::Pnpm,
            PackageManager::Yarn,
            PackageManager::Bun,
        ]
        .into_iter()
        .find(|manager| name.split('@').next() == Some(manager.as_str()))
    });
    let has = |name: &str| root.join(name).is_file();
    Some(from_name.unwrap_or(if has("pnpm-lock.yaml") {
        PackageManager::Pnpm
    } else if has("yarn.lock") {
        PackageManager::Yarn
    } else if has("bun.lock") || has("bun.lockb") {
        PackageManager::Bun
    } else {
        PackageManager::Npm
    }))
}

/// How `tool`'s hook starts gglint in the repository at `root`. pre-commit runs gglint's own
/// hook definition, which uses uvx, whatever the repository is.
pub fn runner(root: &Path, tool: Tool) -> Runner {
    match package_manager(root) {
        Some(manager) if tool != Tool::PreCommit => Runner::Package(manager),
        _ => Runner::Uvx,
    }
}

/// Whether the repository at `root` already lists gglint among its dependencies.
pub fn has_dependency(root: &Path) -> bool {
    let Some(text) = package_json(root) else {
        return false;
    };
    let Ok(Some(JsonValue::Object(object))) = jsonc_parser::parse_to_value(&text, &ParseOptions::default()) else {
        return false;
    };
    ["devDependencies", "dependencies"].iter().any(|key| {
        object
            .get_object(key)
            .is_some_and(|dependencies| dependencies.get(NPM_PACKAGE).is_some())
    })
}

/// Adds this version of gglint to the dev dependencies of the repository at `root` with its
/// package manager, which prints what it does.
pub fn add_dependency(root: &Path, manager: PackageManager) -> Result<(), String> {
    // A package manager is often a `.cmd` script on Windows, which only `cmd` runs.
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/d", "/c", manager.as_str()]);
        command
    } else {
        Command::new(manager.as_str())
    };
    let retry = format!("run `{}`, then `gglint hook` again", manager.add_command());
    let status = command
        .args(manager.add_args())
        .current_dir(root)
        .status()
        .map_err(|error| format!("couldn't run {}: {error}; {retry}", manager.as_str()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} couldn't add gglint ({status}); {retry}", manager.as_str()))
    }
}

/// The command a hook runs.
fn command(runner: Runner, subcommand: &str) -> String {
    match runner {
        Runner::Uvx => format!("uvx gglint@{} {subcommand}", env!("CARGO_PKG_VERSION")),
        Runner::Package(manager) => format!("{} {subcommand}", manager.exec()),
    }
}

/// The tool a repository already uses: the first of husky, lefthook, pre-commit and
/// lint-staged that's set up, else plain git.
pub fn detect(root: &Path) -> Tool {
    let has = |name: &str| root.join(name).exists();
    if has(".husky") {
        Tool::Husky
    } else if has("lefthook.yml") || has(".lefthook.yml") || has("lefthook.yaml") || has(".lefthook.yaml") {
        Tool::Lefthook
    } else if has(".pre-commit-config.yaml") {
        Tool::PreCommit
    } else if has_lint_staged(root) {
        Tool::LintStaged
    } else {
        Tool::Git
    }
}

fn has_lint_staged(root: &Path) -> bool {
    let rc = std::fs::read_dir(root).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with(".lintstagedrc"))
    });
    rc || std::fs::read_to_string(root.join("package.json")).is_ok_and(|text| text.contains("\"lint-staged\""))
}

/// The lines for `tool`'s own configuration.
fn snippet(tool: Tool, runner: Runner, with_mcp: bool) -> (&'static str, String) {
    let staged_scan = command(runner, "secrets --staged");
    let mcp = command(runner, "mcp --format lines");
    match tool {
        Tool::PreCommit => (
            ".pre-commit-config.yaml",
            format!(
                "repos:\n  - repo: https://github.com/corridor/gglint\n    rev: v{}\n    hooks:\n      - id: gglint-secrets\n{}",
                env!("CARGO_PKG_VERSION"),
                if with_mcp { "      - id: gglint-mcp\n" } else { "" }
            ),
        ),
        Tool::Lefthook => (
            "lefthook.yml",
            format!(
                "pre-commit:\n  commands:\n    gglint-secrets:\n      run: {staged_scan}\n{}",
                if with_mcp {
                    format!("    gglint-mcp:\n      run: {mcp}\n")
                } else {
                    String::new()
                }
            ),
        ),
        Tool::LintStaged => (
            "package.json",
            format!("\"lint-staged\": {{\n  \"*\": \"{}\"\n}}\n", command(runner, "secrets")),
        ),
        Tool::Husky | Tool::Git => (".husky/pre-commit", script_lines(runner, with_mcp)),
    }
}

/// The commands of a hook script, one per line. A hook that fails stops the commit.
fn script_lines(runner: Runner, with_mcp: bool) -> String {
    let mut lines = format!("{}\n", command(runner, "secrets --staged"));
    if with_mcp {
        lines.push_str(&format!("{}\n", command(runner, "mcp --format lines")));
    }
    lines
}

/// Adds the gglint commands to the hook script at `path`, creating it if need be.
fn write_script(path: &Path, runner: Runner, with_mcp: bool, shebang: bool) -> Result<Outcome, String> {
    let existing = std::fs::read_to_string(path).ok();
    if existing.as_deref().is_some_and(|text| text.contains("gglint")) {
        return Ok(Outcome::AlreadyThere(path.to_path_buf()));
    }
    let mut text = match existing {
        Some(text) if text.ends_with('\n') || text.is_empty() => text,
        Some(text) => format!("{text}\n"),
        None if shebang => "#!/bin/sh\n".to_string(),
        None => String::new(),
    };
    text.push_str("# gglint: no secrets in agent config files\n");
    text.push_str(&script_lines(runner, with_mcp));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("can't create {}: {error}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|error| format!("can't write {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .map_err(|error| format!("can't read {}: {error}", path.display()))?
            .permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        std::fs::set_permissions(path, permissions)
            .map_err(|error| format!("can't make {} executable: {error}", path.display()))?;
    }
    Ok(Outcome::Wrote(path.to_path_buf()))
}

/// Git's hooks directory for the repository at `root`, which `core.hooksPath` and worktrees
/// can move.
fn git_hooks_directory(root: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("can't run git: {error}"))?;
    if !output.status.success() {
        return Err("this isn't a git repository".to_string());
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    Ok(if path.is_absolute() { path } else { root.join(path) })
}

/// Sets up `tool` in the repository at `root` to run gglint before each commit, started by
/// `runner`. A JavaScript repository's dependency is added first, with `add_dependency`.
pub fn install(root: &Path, tool: Tool, runner: Runner, with_mcp: bool) -> Result<Outcome, String> {
    match tool {
        Tool::Husky => write_script(&root.join(".husky/pre-commit"), runner, with_mcp, false),
        Tool::Git => write_script(&git_hooks_directory(root)?.join("pre-commit"), runner, with_mcp, true),
        Tool::PreCommit | Tool::Lefthook | Tool::LintStaged => {
            let (file, snippet) = snippet(tool, runner, with_mcp);
            Ok(Outcome::Print { file, snippet })
        }
    }
}

/// The lines `install` would add for `tool`, and the file they go in.
pub fn print(tool: Tool, runner: Runner, with_mcp: bool) -> (&'static str, String) {
    snippet(tool, runner, with_mcp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gglint-hook-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_the_tool_a_repository_uses() {
        let dir = temp("detect");
        assert_eq!(detect(&dir), Tool::Git);
        std::fs::write(dir.join("package.json"), "{\"lint-staged\": {}}").unwrap();
        assert_eq!(detect(&dir), Tool::LintStaged);
        std::fs::write(dir.join(".pre-commit-config.yaml"), "repos: []\n").unwrap();
        assert_eq!(detect(&dir), Tool::PreCommit);
        std::fs::write(dir.join("lefthook.yml"), "").unwrap();
        assert_eq!(detect(&dir), Tool::Lefthook);
        std::fs::create_dir(dir.join(".husky")).unwrap();
        assert_eq!(detect(&dir), Tool::Husky);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn husky_gets_a_script_and_a_second_install_changes_nothing() {
        let dir = temp("husky");
        let first = install(&dir, Tool::Husky, Runner::Uvx, false).unwrap();
        let path = dir.join(".husky/pre-commit");
        assert_eq!(first, Outcome::Wrote(path.clone()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(" secrets --staged\n"), "{text}");
        assert!(!text.contains("mcp"));

        assert_eq!(
            install(&dir, Tool::Husky, Runner::Uvx, true).unwrap(),
            Outcome::AlreadyThere(path.clone())
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_existing_script_is_extended_not_replaced() {
        let dir = temp("extend");
        std::fs::create_dir(dir.join(".husky")).unwrap();
        std::fs::write(dir.join(".husky/pre-commit"), "npx lint-staged").unwrap();

        install(&dir, Tool::Husky, Runner::Uvx, true).unwrap();

        let text = std::fs::read_to_string(dir.join(".husky/pre-commit")).unwrap();
        assert!(text.starts_with("npx lint-staged\n"), "{text}");
        assert!(text.contains("secrets --staged\n") && text.contains("mcp --format lines\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn other_tools_get_lines_to_paste() {
        let dir = temp("print");
        for (tool, file, expected) in [
            (Tool::PreCommit, ".pre-commit-config.yaml", "- id: gglint-secrets"),
            (Tool::Lefthook, "lefthook.yml", "gglint-secrets:"),
            (Tool::LintStaged, "package.json", "\"lint-staged\""),
        ] {
            match install(&dir, tool, Runner::Uvx, false).unwrap() {
                Outcome::Print { file: actual, snippet } => {
                    assert_eq!(actual, file);
                    assert!(snippet.contains(expected), "{snippet}");
                }
                other => panic!("{other:?}"),
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_a_javascript_repositorys_package_manager() {
        let dir = temp("manager");
        assert_eq!(package_manager(&dir), None);
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        assert_eq!(package_manager(&dir), Some(PackageManager::Npm));
        for (lockfile, manager) in [
            ("bun.lock", PackageManager::Bun),
            ("yarn.lock", PackageManager::Yarn),
            ("pnpm-lock.yaml", PackageManager::Pnpm),
        ] {
            std::fs::write(dir.join(lockfile), "").unwrap();
            assert_eq!(package_manager(&dir), Some(manager), "{lockfile}");
        }
        // `packageManager` names it outright, whatever lockfile is there.
        std::fs::write(dir.join("package.json"), r#"{ "packageManager": "yarn@4.5.0" }"#).unwrap();
        assert_eq!(package_manager(&dir), Some(PackageManager::Yarn));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_javascript_repositorys_hook_runs_its_own_gglint() {
        let dir = temp("runner");
        assert_eq!(runner(&dir, Tool::Husky), Runner::Uvx);
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(runner(&dir, Tool::Husky), Runner::Package(PackageManager::Pnpm));
        // pre-commit runs gglint's own hook definition, with uvx.
        assert_eq!(runner(&dir, Tool::PreCommit), Runner::Uvx);

        for (manager, line) in [
            (PackageManager::Npm, "npx --no -- gglint secrets --staged\n"),
            (PackageManager::Pnpm, "pnpm exec gglint secrets --staged\n"),
            (PackageManager::Yarn, "yarn gglint secrets --staged\n"),
            (PackageManager::Bun, "bun run gglint secrets --staged\n"),
        ] {
            assert_eq!(script_lines(Runner::Package(manager), false), line);
        }
        let (file, lint_staged) = print(Tool::LintStaged, Runner::Package(PackageManager::Pnpm), false);
        assert_eq!(file, "package.json");
        assert!(
            lint_staged.contains("\"*\": \"pnpm exec gglint secrets\""),
            "{lint_staged}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn knows_when_gglint_is_already_a_dependency() {
        let dir = temp("dependency");
        assert!(!has_dependency(&dir));
        std::fs::write(
            dir.join("package.json"),
            r#"{ "devDependencies": { "husky": "9.1.7" } }"#,
        )
        .unwrap();
        assert!(!has_dependency(&dir));
        std::fs::write(
            dir.join("package.json"),
            r#"{ "devDependencies": { "@gglint/cli": "0.5.0" } }"#,
        )
        .unwrap();
        assert!(has_dependency(&dir));
        std::fs::write(
            dir.join("package.json"),
            r#"{ "dependencies": { "@gglint/cli": "^0.5.0" } }"#,
        )
        .unwrap();
        assert!(has_dependency(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn adds_gglint_pinned_to_this_version() {
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            PackageManager::Npm.add_command(),
            format!("npm install --save-dev --save-exact @gglint/cli@{version}")
        );
        assert_eq!(
            PackageManager::Pnpm.add_command(),
            format!("pnpm add --save-dev --save-exact @gglint/cli@{version}")
        );
        assert_eq!(
            PackageManager::Yarn.add_command(),
            format!("yarn add --dev --exact @gglint/cli@{version}")
        );
        assert_eq!(
            PackageManager::Bun.add_command(),
            format!("bun add --dev --exact @gglint/cli@{version}")
        );
    }
}
