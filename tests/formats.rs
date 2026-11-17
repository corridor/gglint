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
fn github_format_annotates_the_file_and_line_without_the_secret() {
    let dir = drifted();

    let secrets = gglint(&["secrets".as_ref(), "--format=github".as_ref(), dir.path().as_os_str()]);
    let mcp = gglint(&["mcp".as_ref(), "--format=github".as_ref(), dir.path().as_os_str()]);

    assert_eq!(secrets.status.code(), Some(1));
    let secrets = stdout(&secrets);
    assert!(
        secrets.starts_with("::error file=.cursor/mcp.json,line=5,col=45,title=gglint inline-secret::"),
        "{secrets}"
    );
    assert!(!secrets.contains(&token()));
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
