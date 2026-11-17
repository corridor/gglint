//! `gglint secrets`.

mod common;

use std::process::Command;

use common::TempDir;

fn gglint(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gglint")).args(args).output().unwrap()
}

/// A GitHub token's shape without being one.
fn token() -> String {
    format!("ghp_{}", "aB3dE6gH9jK2mN5pQ8sT1vW4yZ7aB3dE6gH9")
}

fn cursor_config(token: &str) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"github\": {{\n      \"url\": \"https://api.githubcopilot.com/mcp/\",\n      \"headers\": {{ \"Authorization\": \"Bearer {token}\" }}\n    }}\n  }}\n}}\n"
    )
}

#[test]
fn reports_a_secret_by_place_and_never_prints_its_value() {
    let dir = TempDir::new();
    let token = token();
    dir.write(".cursor/mcp.json", cursor_config(&token));

    let output = gglint(&[
        "secrets".as_ref(),
        "--format".as_ref(),
        "lines".as_ref(),
        dir.path().as_os_str(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(".cursor/mcp.json:5:"), "{stdout}");
    assert!(stdout.contains("inline-secret"), "{stdout}");
    assert!(!stdout.contains(&token) && !stderr.contains(&token));
}

#[test]
fn a_repository_without_secrets_passes() {
    let dir = TempDir::new();
    dir.write(".cursor/mcp.json", cursor_config("${env:GITHUB_TOKEN}"));
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.github]\nurl = \"https://x\"\nbearer_token_env_var = \"GITHUB_TOKEN\"\n",
    );
    dir.write(
        "CLAUDE.md",
        "Use ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx as an example.\n",
    );

    let output = gglint(&["secrets".as_ref(), dir.path().as_os_str()]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn fix_replaces_the_secret_with_the_agents_own_reference_and_is_idempotent() {
    let dir = TempDir::new();
    let path = dir.write(".cursor/mcp.json", cursor_config(&token()));

    let output = gglint(&["secrets".as_ref(), "--fix".as_ref(), dir.path().as_os_str()]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        cursor_config("${env:GITHUB_TOKEN}")
    );
    let again = gglint(&["secrets".as_ref(), "--fix".as_ref(), dir.path().as_os_str()]);
    assert!(again.status.success());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        cursor_config("${env:GITHUB_TOKEN}")
    );
}

#[test]
fn fix_leaves_a_secret_in_a_file_whose_agent_has_no_reference_syntax() {
    let dir = TempDir::new();
    let path = dir.write(
        ".codex/config.toml",
        format!(
            "[mcp_servers.github]\nurl = \"https://x\"\nhttp_headers = {{ Authorization = \"Bearer {}\" }}\n",
            token()
        ),
    );
    let before = std::fs::read_to_string(&path).unwrap();

    let output = gglint(&["secrets".as_ref(), "--fix".as_ref(), dir.path().as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn files_named_on_the_command_line_are_checked_as_given() {
    let dir = TempDir::new();
    let clean = dir.write("clean.json", "{}\n");
    let dirty = dir.write("notes.md", format!("key: {}\n", token()));

    let ok = gglint(&["secrets".as_ref(), clean.as_os_str()]);
    let bad = gglint(&[
        "secrets".as_ref(),
        "--format".as_ref(),
        "lines".as_ref(),
        dirty.as_os_str(),
    ]);

    assert!(ok.status.success());
    assert_eq!(bad.status.code(), Some(1));
    assert!(String::from_utf8(bad.stdout).unwrap().contains("notes.md:1:"));
}

#[test]
fn staged_checks_only_what_is_staged() {
    let dir = TempDir::new();
    let git = |args: &[&str]| {
        let status = Command::new("git").args(args).current_dir(dir.path()).status().unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    dir.write(".cursor/mcp.json", cursor_config(&token()));

    let unstaged = gglint(&["secrets".as_ref(), "--staged".as_ref(), dir.path().as_os_str()]);
    git(&["add", ".cursor/mcp.json"]);
    let staged = gglint(&["secrets".as_ref(), "--staged".as_ref(), dir.path().as_os_str()]);

    assert!(unstaged.status.success());
    assert_eq!(staged.status.code(), Some(1));
}

/// Runs git in `dir`, and fails the test if git does.
fn git(dir: &TempDir, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn staged_checks_the_copy_in_the_commit_not_the_one_on_disk() {
    let dir = TempDir::new();
    git(&dir, &["init", "-q"]);
    let clean = cursor_config("${env:GITHUB_TOKEN}");

    // Staged with a secret, then fixed on disk only: the commit would still have it.
    dir.write(".cursor/mcp.json", cursor_config(&token()));
    git(&dir, &["add", ".cursor/mcp.json"]);
    dir.write(".cursor/mcp.json", &clean);
    let staged_secret = gglint(&["secrets".as_ref(), "--staged".as_ref(), dir.path().as_os_str()]);

    // Staged clean, with a secret only on disk: the commit is fine.
    dir.write(".cursor/mcp.json", &clean);
    git(&dir, &["add", ".cursor/mcp.json"]);
    dir.write(".cursor/mcp.json", cursor_config(&token()));
    let disk_secret = gglint(&["secrets".as_ref(), "--staged".as_ref(), dir.path().as_os_str()]);

    assert_eq!(staged_secret.status.code(), Some(1));
    assert!(disk_secret.status.success());
}

#[test]
fn staged_finds_files_under_a_root_below_the_top_of_the_repository() {
    let dir = TempDir::new();
    git(&dir, &["init", "-q"]);
    dir.write("app/.cursor/mcp.json", cursor_config(&token()));
    git(&dir, &["add", "."]);

    let output = gglint(&[
        "secrets".as_ref(),
        "--staged".as_ref(),
        "--format".as_ref(),
        "lines".as_ref(),
        dir.path().join("app").as_os_str(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains(".cursor/mcp.json:5:")
    );
}

#[test]
fn staged_and_fix_cannot_be_combined() {
    let dir = TempDir::new();
    git(&dir, &["init", "-q"]);

    let output = gglint(&[
        "secrets".as_ref(),
        "--staged".as_ref(),
        "--fix".as_ref(),
        dir.path().as_os_str(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr).unwrap().contains("git add"));
}

#[test]
fn staged_and_a_file_list_cannot_be_combined() {
    let dir = TempDir::new();
    let file = dir.write("a.json", "{}\n");

    let output = gglint(&["secrets".as_ref(), "--staged".as_ref(), file.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr).unwrap().contains("use one"));
}

#[test]
fn hook_installs_into_husky_once_and_prints_lines_for_lefthook() {
    let dir = TempDir::new();
    dir.write(".husky/pre-commit", "npx lint-staged\n");

    let first = gglint(&["hook".as_ref(), dir.path().as_os_str()]);
    let second = gglint(&["hook".as_ref(), dir.path().as_os_str()]);
    let print = gglint(&["hook".as_ref(), "print".as_ref(), "--tool=lefthook".as_ref()]);

    assert!(first.status.success() && second.status.success());
    let script = std::fs::read_to_string(dir.path().join(".husky/pre-commit")).unwrap();
    assert!(script.starts_with("npx lint-staged\n"));
    assert_eq!(script.matches("secrets --staged").count(), 1, "{script}");
    let stdout = String::from_utf8(print.stdout).unwrap();
    assert!(
        stdout.contains("gglint-secrets:") && stdout.contains("secrets --staged"),
        "{stdout}"
    );
}

#[test]
fn a_secret_in_a_gitignored_file_says_so_and_codex_gets_its_own_advice() {
    let dir = TempDir::new();
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    dir.write(".gitignore", ".codex\n");
    dir.write(
        ".codex/config.toml",
        format!(
            "[mcp_servers.refero]\nurl = \"https://x\"\n[mcp_servers.refero.http_headers]\nAuthorization = \"Bearer {}\"\n",
            "aB3dE6gH9jK2mN5pQ8sT1vW4yZ7"
        ),
    );
    dir.write(".cursor/mcp.json", cursor_config(&token()));

    let output = gglint(&[
        "secrets".as_ref(),
        "--format".as_ref(),
        "lines".as_ref(),
        dir.path().as_os_str(),
    ]);

    let stdout = String::from_utf8(output.stdout).unwrap();
    let codex = stdout.lines().find(|line| line.contains(".codex/config.toml")).unwrap();
    let cursor = stdout.lines().find(|line| line.contains(".cursor/mcp.json")).unwrap();
    assert!(codex.contains("gitignored"), "{stdout}");
    assert!(!cursor.contains("gitignored"), "{stdout}");

    let table = gglint(&["secrets".as_ref(), dir.path().as_os_str()]);
    let table = String::from_utf8(table.stdout).unwrap();
    assert!(
        table.contains("bearer_token_env_var") && table.contains("env_http_headers"),
        "{table}"
    );
}

/// Text with the mix of a credential's characters, `length` long, that is nothing else.
fn filler(length: usize) -> String {
    let alphabet = b"aB3dE6gH9jK2mN5pQ8sT1vW4yZ7";
    (0..length).map(|i| alphabet[i % alphabet.len()] as char).collect()
}

#[test]
fn no_planted_value_reaches_any_output_in_any_format_or_after_fix() {
    let planted = [
        format!("ghp_{}", filler(36)),
        format!("sk-ant-{}", filler(40)),
        format!("AKIA{}", "ABCDEFGH23456789"),
        format!("xoxb-{}", filler(24)),
        "Zx9Qw3Er7Ty1Uo5P".to_string(),
        format!("sk-{}", filler(40)),
    ];
    let dir = TempDir::new();
    dir.write(
        ".cursor/mcp.json",
        format!(
            "{{\n  \"mcpServers\": {{\n    \"one\": {{\n      \"url\": \"https://x\",\n      \"env\": {{\n        \"GITHUB_TOKEN\": \"{}\",\n        \"ANTHROPIC_API_KEY\": \"{}\",\n        \"AWS_ACCESS_KEY_ID\": \"{}\",\n        \"SLACK_TOKEN\": \"{}\",\n        \"password\": \"{}\"\n      }}\n    }}\n  }}\n}}\n",
            planted[0], planted[1], planted[2], planted[3], planted[4]
        ),
    );
    dir.write(
        "CLAUDE.md",
        format!("Never commit {} or\n-----BEGIN PRIVATE KEY-----\n", planted[5]),
    );

    let mut runs: Vec<Vec<&str>> = ["table", "lines", "github", "sarif", "markdown"]
        .iter()
        .map(|format| vec!["secrets", "--format", format])
        .collect();
    runs.push(vec!["secrets", "--fix"]);
    runs.push(vec!["secrets", "--fix", "--format", "github"]);
    for args in runs {
        let mut command: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
        command.push(dir.path().as_os_str());
        let output = gglint(&command);
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!printed.is_empty(), "{args:?} printed nothing");
        for value in &planted {
            assert!(!printed.contains(value), "{args:?} printed a planted value");
            assert!(
                !printed.contains(&value[value.len() - 10..]),
                "{args:?} printed the end of a planted value"
            );
        }
    }
}
