//! The `gglint` command.

mod common;

use std::process::Command;

use common::TempDir;

fn gglint(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gglint")).args(args).output().unwrap()
}

#[test]
fn mcp_servers_lists_each_harness_server_without_setting_values() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{
  "mcpServers": {
    "angular-cli": {
      "command": "npx",
      "args": ["-y", "@angular/cli", "mcp"],
      "env": { "NG_TOKEN": "sk-live-not-for-printing" }
    }
  }
}
"#,
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.angular-cli]\ncommand = \"pnpm\"\nargs = [\"dlx\", \"@angular/cli\", \"mcp\", \"--read-only\"]\nenabled = false\n",
    );

    let output = gglint(&["mcp".as_ref(), "--servers".as_ref(), dir.path().as_os_str()]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout}");
    assert!(
        lines[0].ends_with(".mcp.json:3:5  claude-code  angular-cli  npx -y @angular/cli mcp  (env=[NG_TOKEN])"),
        "{stdout}"
    );
    assert!(
        lines[1]
            .ends_with("config.toml:1:14  codex        angular-cli  pnpm dlx @angular/cli mcp --read-only  (disabled)"),
        "{stdout}"
    );
    assert!(!stdout.contains("sk-live"), "{stdout}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Found 2 MCP server(s) in 2 file(s)"), "{stderr}");
}

#[test]
fn mcp_fails_when_there_are_no_config_files() {
    let dir = TempDir::new();

    let output = gglint(&["mcp".as_ref(), dir.path().as_os_str()]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("No MCP config files found")
    );
}

#[test]
fn mcp_format_lines_prints_one_line_per_problem() {
    let dir = TempDir::new();
    dir.write(".mcp.json", "{ \"mcpServers\": {} }\n");
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.angular-cli]\ncommand = \"pnpm\"\nargs = [\"dlx\", \"@angular/cli\", \"mcp\", \"--read-only\"]\n",
    );
    dir.write(
        ".cursor/mcp.json",
        "{\n  \"mcpServers\": {\n    \"angular-cli\": { \"command\": \"npx\", \"args\": [\"-y\", \"@angular/cli\", \"mcp\"] }\n  }\n}\n",
    );

    let output = gglint(&[
        "mcp".as_ref(),
        "--format".as_ref(),
        "lines".as_ref(),
        dir.path().as_os_str(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{stdout}");
    assert!(
        lines[0].ends_with(
            "config.toml:1:14  missing-server   `angular-cli` is set up for Codex and Cursor but not for Claude Code (.mcp.json)"
        ),
        "{stdout}"
    );
    assert!(
        lines[1].starts_with("    also at ") && lines[1].ends_with(".cursor/mcp.json:3:5"),
        "{stdout}"
    );
    assert!(
        lines[2].ends_with(
            "config.toml:1:14  different-server `angular-cli` is started with different arguments: `mcp --read-only` in Codex; `mcp` in Cursor"
        ),
        "{stdout}"
    );
    assert!(
        lines[3].starts_with("    also at ") && lines[3].ends_with(".cursor/mcp.json:3:5"),
        "{stdout}"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Checked 2 MCP server(s) in 3 file(s) for Claude Code, Codex and Cursor: 2 problem(s)"),
        "{stderr}"
    );
}

#[test]
fn mcp_passes_when_every_agent_gets_the_same_servers() {
    let dir = TempDir::new();
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\n",
    );
    dir.write(
        ".cursor/mcp.json",
        "{ \"mcpServers\": { \"docs\": { \"url\": \"https://docs.example.com/mcp\" } } }\n",
    );

    let output = gglint(&["mcp".as_ref(), dir.path().as_os_str()]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("│ docs   │   ✓   │   ✓    │"), "{stdout}");
    assert!(
        stdout.ends_with("No problems: every agent gets the same servers, set up the same way.\n"),
        "{stdout}"
    );
    assert!(String::from_utf8(output.stderr).unwrap().contains(": no problems"));
}

/// GGX's MCP servers: two set up for Codex, one of them also for Cursor, and a `.claude`
/// directory without a `.mcp.json`.
fn ggx_like() -> TempDir {
    let dir = TempDir::new();
    dir.write(".claude/settings.json", "{}\n");
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.code-review-graph]\ncommand = \"uvx\"\nargs = [\"code-review-graph@latest\", \"serve\"]\n\n\
         [mcp_servers.angular-cli]\ncommand = \"pnpm\"\nargs = [\"dlx\", \"@angular/cli\", \"mcp\", \"--read-only\"]\n",
    );
    dir.write(
        ".cursor/mcp.json",
        "{ \"mcpServers\": { \"angular-cli\": { \"command\": \"npx\", \"args\": [\"-y\", \"@angular/cli\", \"mcp\"] } } }\n",
    );
    dir
}

#[test]
fn mcp_draws_each_server_in_each_agent_and_every_problem() {
    let dir = ggx_like();

    let output = gglint(&["mcp".as_ref(), dir.path().as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (title, report) = stdout.split_once('\n').unwrap();
    assert!(title.starts_with("MCP servers in gglint-test-"), "{stdout}");
    // No colours: standard output isn't a terminal.
    assert_eq!(
        report,
        "
  ╭───────────────────┬─────────────┬───────┬────────╮
  │ Server            │ Claude Code │ Codex │ Cursor │
  ├───────────────────┼─────────────┼───────┼────────┤
  │ code-review-graph │      ✗      │   ✓   │   ✗    │
  │ angular-cli       │      ✗      │   ✓   │   ≠    │
  ╰───────────────────┴─────────────┴───────┴────────╯
  ✓ set up   ≠ set up differently   ✗ missing   ○ turned off

3 problems

  code-review-graph
    ✗ Missing from Cursor.
      Add it to .cursor/mcp.json, as set up in .codex/config.toml:1

  angular-cli
    ≠ Started with different arguments: `mcp --read-only` in Codex; `mcp` in Cursor.
      `gglint mcp --fix` starts it everywhere as in .codex/config.toml:5

  Missing config files
    ✗ Claude Code has a .claude directory but no .mcp.json.
      `gglint mcp --fix` creates .mcp.json with the 2 MCP servers set up for Codex and Cursor
"
    );
}

#[test]
fn mcp_rejects_an_unknown_format() {
    let dir = ggx_like();

    let output = gglint(&[
        "mcp".as_ref(),
        "--format".as_ref(),
        "xml".as_ref(),
        dir.path().as_os_str(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr).unwrap().contains("unknown format"));
}

#[test]
fn mcp_rejects_an_unknown_option() {
    let dir = ggx_like();

    let output = gglint(&["mcp".as_ref(), "--create".as_ref(), dir.path().as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unknown option \"--create\"")
    );
    assert!(!dir.path().join(".mcp.json").exists());
}

#[test]
fn mcp_fix_adds_servers_creates_config_files_and_aligns_command_lines() {
    let dir = ggx_like();
    let codex = std::fs::read_to_string(dir.path().join(".codex/config.toml")).unwrap();

    let output = gglint(&["mcp".as_ref(), "--fix".as_ref(), dir.path().as_os_str()]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with(
            "Changed 2 files:\n\
             \x20 .mcp.json (new)    added code-review-graph, angular-cli\n\
             \x20 .cursor/mcp.json   added code-review-graph; updated angular-cli\n\
             \n\
             Review the changes with `git diff`.\n"
        ),
        "{stdout}"
    );
    assert!(
        stdout.ends_with("No problems: every agent gets the same servers, set up the same way.\n"),
        "{stdout}"
    );
    let cursor = std::fs::read_to_string(dir.path().join(".cursor/mcp.json")).unwrap();
    assert!(
        cursor.contains(r#""command": "pnpm", "args": ["dlx", "@angular/cli", "mcp", "--read-only"]"#),
        "{cursor}"
    );
    // Codex had every server, set up as the others now are.
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".codex/config.toml")).unwrap(),
        codex
    );
}

#[test]
fn prints_the_version_and_usage() {
    let version = gglint(&["--version".as_ref()]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("gglint {}\n", env!("CARGO_PKG_VERSION"))
    );

    let help = gglint(&["--help".as_ref()]);
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("gglint mcp --fix [PATH]")
    );

    // Without a command there's nothing to do.
    let bare = gglint(&[]);
    assert_eq!(bare.status.code(), Some(1));
    assert!(
        String::from_utf8(bare.stderr)
            .unwrap()
            .starts_with("Keep the MCP servers")
    );
}

#[test]
fn mcp_fix_source_takes_each_server_from_that_agent() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        "{ \"mcpServers\": { \"docs\": { \"command\": \"npx\", \"args\": [\"-y\", \"docs-mcp@1\"] } } }\n",
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\ncommand = \"npx\"\nargs = [\"-y\", \"docs-mcp@2\"]\n",
    );

    let output = gglint(&[
        "mcp".as_ref(),
        "--fix".as_ref(),
        "--source=codex".as_ref(),
        dir.path().as_os_str(),
    ]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("Changed 1 file:\n\x20 .mcp.json   updated docs\n"),
        "{stdout}"
    );
    let claude = std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
    assert!(claude.contains("docs-mcp@2"), "{claude}");
}

#[test]
fn mcp_rejects_an_unknown_source() {
    let dir = ggx_like();

    let output = gglint(&[
        "mcp".as_ref(),
        "--source".as_ref(),
        "vscode".as_ref(),
        dir.path().as_os_str(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unknown agent \"vscode\" for --source; use claude-code, codex, cursor or opencode")
    );
}
