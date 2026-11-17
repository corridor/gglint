//! `gglint hook`: runs gglint on every commit, through whichever hook tool a repository uses.
//!
//! husky and plain git hooks are installed by writing a script, never overwriting one that
//! exists: the command is appended once. For pre-commit, lefthook and lint-staged, whose
//! configuration is YAML or JSON that gglint doesn't edit, it prints the lines to add.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// The command a hook runs, pinned to this version of gglint.
fn command(subcommand: &str) -> String {
    format!("uvx gglint@{} {subcommand}", env!("CARGO_PKG_VERSION"))
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
fn snippet(tool: Tool, with_mcp: bool) -> (&'static str, String) {
    let secrets = command("secrets --staged");
    let mcp = command("mcp --format lines");
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
                "pre-commit:\n  commands:\n    gglint-secrets:\n      run: {secrets}\n{}",
                if with_mcp {
                    format!("    gglint-mcp:\n      run: {mcp}\n")
                } else {
                    String::new()
                }
            ),
        ),
        Tool::LintStaged => (
            "package.json",
            format!("\"lint-staged\": {{\n  \"*\": \"{}\"\n}}\n", command("secrets")),
        ),
        Tool::Husky | Tool::Git => (".husky/pre-commit", script_lines(with_mcp)),
    }
}

/// The commands of a hook script, one per line. A hook that fails stops the commit.
fn script_lines(with_mcp: bool) -> String {
    let mut lines = format!("{}\n", command("secrets --staged"));
    if with_mcp {
        lines.push_str(&format!("{}\n", command("mcp --format lines")));
    }
    lines
}

/// Adds the gglint commands to the hook script at `path`, creating it if need be.
fn write_script(path: &Path, with_mcp: bool, shebang: bool) -> Result<Outcome, String> {
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
    text.push_str(&script_lines(with_mcp));
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

/// Sets up `tool` in the repository at `root` to run gglint before each commit.
pub fn install(root: &Path, tool: Tool, with_mcp: bool) -> Result<Outcome, String> {
    match tool {
        Tool::Husky => write_script(&root.join(".husky/pre-commit"), with_mcp, false),
        Tool::Git => write_script(&git_hooks_directory(root)?.join("pre-commit"), with_mcp, true),
        Tool::PreCommit | Tool::Lefthook | Tool::LintStaged => {
            let (file, snippet) = snippet(tool, with_mcp);
            Ok(Outcome::Print { file, snippet })
        }
    }
}

/// The lines `install` would add for `tool`, and the file they go in.
pub fn print(tool: Tool, with_mcp: bool) -> (&'static str, String) {
    snippet(tool, with_mcp)
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
        let first = install(&dir, Tool::Husky, false).unwrap();
        let path = dir.join(".husky/pre-commit");
        assert_eq!(first, Outcome::Wrote(path.clone()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(" secrets --staged\n"), "{text}");
        assert!(!text.contains("mcp"));

        assert_eq!(
            install(&dir, Tool::Husky, true).unwrap(),
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

        install(&dir, Tool::Husky, true).unwrap();

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
            match install(&dir, tool, false).unwrap() {
                Outcome::Print { file: actual, snippet } => {
                    assert_eq!(actual, file);
                    assert!(snippet.contains(expected), "{snippet}");
                }
                other => panic!("{other:?}"),
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
