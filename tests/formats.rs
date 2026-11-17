//! The `github`, `sarif` and `markdown` formats, and the script of the GitHub Action.

mod common;

use std::process::Command;

use common::TempDir;

fn gglint(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gglint")).args(args).output().unwrap()
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// A GitHub token's shape without being one.
fn token() -> String {
    format!("ghp_{}", "aB3dE6gH9jK2mN5pQ8sT1vW4yZ7aB3dE6gH9")
}

/// A repository where Cursor has a secret and lacks a server Claude Code has.
fn drifted() -> TempDir {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "linear": { "url": "https://mcp.linear.app/sse" } } }"#,
    );
    dir.write(
        ".cursor/mcp.json",
        format!(
            "{{\n  \"mcpServers\": {{\n    \"github\": {{\n      \"url\": \"https://x\",\n      \"headers\": {{ \"Authorization\": \"Bearer {}\" }}\n    }}\n  }}\n}}\n",
            token()
        ),
    );
    dir
}

#[test]
fn github_and_sarif_paths_start_where_gglint_runs_so_a_subfolder_lands_on_its_files() {
    // The Action runs in the workspace, with its `path` input as the folder to check.
    let workspace = TempDir::new();
    let checked = drifted();
    let sub = workspace.path().join("services/api");
    std::fs::create_dir_all(sub.parent().unwrap()).unwrap();
    std::fs::rename(checked.path(), &sub).unwrap();

    for (format, expected) in [
        ("github", "file=services/api/.cursor/mcp.json,"),
        ("sarif", "\"uri\":\"services/api/.cursor/mcp.json\""),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_gglint"))
            .args(["secrets", "--format", format, "services/api"])
            .current_dir(workspace.path())
            .output()
            .unwrap();
        assert!(stdout(&output).contains(expected), "{format}: {}", stdout(&output));
    }
}

#[test]
fn github_format_annotates_the_file_and_line_without_the_secret() {
    let dir = drifted();

    let scan = gglint(&["secrets".as_ref(), "--format=github".as_ref(), dir.path().as_os_str()]);
    let mcp = gglint(&["mcp".as_ref(), "--format=github".as_ref(), dir.path().as_os_str()]);

    assert_eq!(scan.status.code(), Some(1));
    let annotations = stdout(&scan);
    assert!(
        annotations.starts_with("::error file=.cursor/mcp.json,line=5,col=45,title=gglint inline-secret::"),
        "{annotations}"
    );
    assert!(!annotations.contains(&token()));
    assert!(
        stdout(&mcp).contains("title=gglint missing-server::"),
        "{}",
        stdout(&mcp)
    );
}

#[test]
fn sarif_format_is_a_log_with_the_rule_the_place_and_a_category() {
    let dir = drifted();

    let output = stdout(&gglint(&[
        "secrets".as_ref(),
        "--format=sarif".as_ref(),
        dir.path().as_os_str(),
    ]));

    assert!(output.contains("\"version\":\"2.1.0\""), "{output}");
    assert!(output.contains("\"id\":\"gglint/secrets/\""), "{output}");
    assert!(output.contains("\"ruleId\":\"inline-secret\""), "{output}");
    assert!(output.contains("\"uri\":\".cursor/mcp.json\""), "{output}");
    assert!(!output.contains(&token()));
}

#[test]
fn markdown_format_for_mcp_has_the_problems_and_a_table_of_servers_by_agent() {
    let dir = drifted();

    let output = stdout(&gglint(&[
        "mcp".as_ref(),
        "--format=markdown".as_ref(),
        dir.path().as_os_str(),
    ]));

    assert!(output.starts_with("### ❌ gglint mcp: "), "{output}");
    assert!(output.contains("| Server | Claude Code | Cursor |"), "{output}");
    assert!(output.contains("| `linear` | ✅ | ❌ |"), "{output}");
}

#[test]
fn an_unknown_format_lists_every_format() {
    let output = gglint(&["secrets".as_ref(), "--format=xml".as_ref()]);

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    for format in ["table", "lines", "github", "sarif", "markdown"] {
        assert!(stderr.contains(format), "{stderr}");
    }
}

#[cfg(unix)]
mod action {
    use super::*;

    struct Run {
        status: Option<i32>,
        stdout: String,
        github_output: String,
        comment: Option<String>,
        sarif_files: Vec<String>,
    }

    /// Runs the Action's script in `dir` as GitHub would, with this build of gglint.
    fn run(dir: &TempDir, environment: &[(&str, &str)]) -> Run {
        let temp = TempDir::new();
        let github_output = temp.path().join("github_output");
        std::fs::write(&github_output, "").unwrap();
        let output = Command::new("bash")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/action.sh"))
            .current_dir(dir.path())
            .env("GGLINT_BIN", env!("CARGO_BIN_EXE_gglint"))
            .env("RUNNER_TEMP", temp.path())
            .env("GITHUB_OUTPUT", &github_output)
            .env("INPUT_PATH", ".")
            .envs(environment.iter().copied())
            .output()
            .unwrap();
        let gglint_directory = temp.path().join("gglint");
        let mut sarif_files: Vec<String> = std::fs::read_dir(gglint_directory.join("sarif"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        sarif_files.sort();
        Run {
            status: output.status.code(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            github_output: std::fs::read_to_string(&github_output).unwrap(),
            comment: std::fs::read_to_string(gglint_directory.join("comment.md")).ok(),
            sarif_files,
        }
    }

    #[test]
    fn annotates_fails_and_leaves_a_comment_and_sarif_for_the_next_steps() {
        let dir = drifted();

        let run = run(
            &dir,
            &[
                ("INPUT_COMMAND", "all"),
                ("INPUT_COMMENT", "true"),
                ("INPUT_SARIF", "true"),
            ],
        );

        assert_eq!(run.status, Some(1));
        assert!(
            run.stdout.contains("::error file=.cursor/mcp.json,line=5"),
            "{}",
            run.stdout
        );
        assert!(run.stdout.contains("title=gglint missing-server::"), "{}", run.stdout);
        assert_eq!(run.sarif_files, ["gglint-mcp.sarif", "gglint-secrets.sarif"]);
        assert!(run.github_output.contains("sarif-directory="), "{}", run.github_output);
        assert!(run.github_output.contains("comment-file="), "{}", run.github_output);
        let comment = run.comment.unwrap();
        assert!(comment.starts_with("<!-- gglint -->\n"), "{comment}");
        assert!(comment.contains("### ❌ gglint secrets: 1 problem"), "{comment}");
        assert!(comment.contains("### ❌ gglint mcp: "), "{comment}");
        assert!(!comment.contains(&token()));
    }

    #[test]
    fn writes_no_comment_or_sarif_unless_asked_to() {
        let dir = drifted();

        let run = run(&dir, &[("INPUT_COMMAND", "secrets")]);

        assert_eq!(run.status, Some(1));
        assert!(run.comment.is_none());
        assert!(run.sarif_files.is_empty());
        assert!(run.github_output.is_empty(), "{}", run.github_output);
    }

    #[test]
    fn reports_without_failing_when_told_not_to_fail() {
        let dir = drifted();

        let run = run(
            &dir,
            &[("INPUT_COMMAND", "secrets"), ("INPUT_FAIL_ON_PROBLEMS", "false")],
        );

        assert_eq!(run.status, Some(0));
        assert!(run.stdout.contains("::error "), "{}", run.stdout);
    }

    #[test]
    fn a_clean_repository_passes_and_says_so() {
        let dir = TempDir::new();
        dir.write(".mcp.json", r#"{ "mcpServers": {} }"#);

        let run = run(&dir, &[("INPUT_COMMAND", "all"), ("INPUT_COMMENT", "true")]);

        assert_eq!(run.status, Some(0));
        assert!(!run.stdout.contains("::error"), "{}", run.stdout);
        assert!(run.comment.unwrap().contains("### ✅ gglint secrets: no problems"));
    }

    #[test]
    fn a_repository_without_mcp_config_files_isnt_a_failure() {
        let dir = TempDir::new();
        dir.write("README.md", "# Nothing here\n");

        let run = run(&dir, &[("INPUT_COMMAND", "all")]);

        assert_eq!(run.status, Some(0));
        assert!(
            run.stdout.contains("::notice::gglint: no MCP config files found"),
            "{}",
            run.stdout
        );
    }

    #[test]
    fn an_unknown_command_is_an_error() {
        let dir = TempDir::new();

        let run = run(&dir, &[("INPUT_COMMAND", "fix")]);

        assert_eq!(run.status, Some(2));
        assert!(run.stdout.contains("::error::command must be"), "{}", run.stdout);
    }
}
