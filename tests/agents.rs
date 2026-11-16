//! The agents gglint supports beyond Claude Code, Codex, Cursor and OpenCode: reading each
//! one's MCP config file, and writing servers into it the way that agent does.

mod common;

use std::path::Path;

use common::TempDir;
use gglint::harness::{
    FixReport, Harness, McpConfig, McpServer, Part, Setting, Transport, Unfixed, check_mcp, compare_mcp, fix_mcp,
    read_mcp,
};

fn setting(name: &str, value: &[Part]) -> Setting {
    Setting {
        name: name.to_string(),
        value: value.to_vec(),
    }
}

fn text(text: &str) -> Part {
    Part::Text(text.to_string())
}

fn command(command: &str, args: &[&str]) -> Transport {
    Transport::Command {
        command: command.to_string(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
    }
}

fn remote(url: &str) -> Transport {
    Transport::Remote { url: url.to_string() }
}

/// The config `harness` has under `dir`, which must be its only one.
fn config_of(dir: &TempDir, harness: Harness) -> McpConfig {
    let mut configs: Vec<McpConfig> = read_mcp(dir.path())
        .into_iter()
        .filter(|config| config.harness == harness)
        .collect();
    assert_eq!(configs.len(), 1, "{harness:?} has {} config files", configs.len());
    let config = configs.remove(0);
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    config
}

fn server<'a>(config: &'a McpConfig, name: &str) -> &'a McpServer {
    config
        .servers
        .iter()
        .find(|server| server.name == name)
        .unwrap_or_else(|| panic!("no server {name:?} in {}", config.relative_path.display()))
}

fn findings(dir: &TempDir) -> Vec<String> {
    compare_mcp(dir.path(), &read_mcp(dir.path()))
        .iter()
        .map(|finding| format!("{}  {}", finding.rule.as_str(), finding.message))
        .collect()
}

/// Runs `--fix` under `dir`, and returns what it did and the problems left.
fn fix(dir: &TempDir) -> (FixReport, Vec<String>) {
    let configs = read_mcp(dir.path());
    let check = check_mcp(dir.path(), &configs);
    let report = fix_mcp(dir.path(), &configs, &check);
    (report, findings(dir))
}

fn read(dir: &TempDir, relative: &str) -> String {
    std::fs::read_to_string(dir.path().join(relative)).unwrap()
}

/// Claude Code's servers: one started as a command, one reached over HTTP.
const CLAUDE_CODE: &str = r#"{
  "mcpServers": {
    "playwright": { "command": "npx", "args": ["-y", "@playwright/mcp@0.0.78"], "env": { "CI": "1" } },
    "docs": { "type": "http", "url": "https://docs.example.com/mcp", "headers": { "X-Team": "web" } }
  }
}
"#;

/// `CLAUDE_CODE`'s servers, written in the `mcpServers` format of an agent that gives a
/// remote server a `url` and nothing else.
const CLAUDE_CODE_AS_URL: &str = r#"{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": ["-y", "@playwright/mcp@0.0.78"],
      "env": {
        "CI": "1"
      }
    },
    "docs": {
      "url": "https://docs.example.com/mcp",
      "headers": {
        "X-Team": "web"
      }
    }
  }
}
"#;

/// Fixes a repository with `CLAUDE_CODE` and the agent's directory, and returns the config
/// file `--fix` created at `file`.
fn created_from_claude_code(directory_file: &str, file: &str) -> String {
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(directory_file, "Be brief.\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    let changed: Vec<(String, bool)> = report
        .changed
        .iter()
        .map(|changed| (changed.relative_path.display().to_string(), changed.created))
        .collect();
    assert_eq!(changed, [(file.to_string(), true)]);
    read(&dir, file)
}

/// What `--fix` leaves undone when Claude Code's server reads `TOKEN` from the environment
/// and the agent with its directory at `directory_file` can't.
fn unfixed_env_reference(directory_file: &str) -> Vec<Unfixed> {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "search": { "command": "uvx", "args": ["search-mcp"], "env": { "TOKEN": "${TOKEN}" } } } }"#,
    );
    dir.write(directory_file, "Be brief.\n");
    fix(&dir).0.unfixed
}

// ---------------------------------------------------------------------------
// Junie
// ---------------------------------------------------------------------------

#[test]
fn reads_junie_servers() {
    let dir = TempDir::new();
    dir.write(
        ".junie/mcp/mcp.json",
        r#"{
  "mcpServers": {
    "context7": { "command": "npx", "args": ["-y", "@upstash/context7-mcp"], "env": { "MODE": "fast" } },
    "remote": { "url": "https://mcp.example.com/v1", "headers": { "Authorization": "Bearer token" } }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::Junie);

    assert_eq!(config.relative_path, Path::new(".junie/mcp/mcp.json"));
    let context7 = server(&config, "context7");
    assert_eq!(context7.transport, command("npx", &["-y", "@upstash/context7-mcp"]));
    assert_eq!(context7.env, [setting("MODE", &[text("fast")])]);
    let remote_server = server(&config, "remote");
    assert_eq!(remote_server.transport, remote("https://mcp.example.com/v1"));
    assert_eq!(
        remote_server.headers,
        [setting("Authorization", &[text("Bearer token")])]
    );
}

#[test]
fn fix_creates_a_junie_config() {
    assert_eq!(
        created_from_claude_code(".junie/guidelines.md", ".junie/mcp/mcp.json"),
        CLAUDE_CODE_AS_URL
    );
}

#[test]
fn junie_gets_no_server_that_reads_the_environment() {
    // Junie's docs don't say how a value refers to an environment variable.
    let unfixed = unfixed_env_reference(".junie/guidelines.md");

    assert_eq!(
        unfixed,
        [Unfixed {
            harness: Harness::Junie,
            server: "search".to_string(),
            reason: "the value of `TOKEN` can't be written for Junie".to_string(),
        }]
    );
}

// ---------------------------------------------------------------------------
// Trae
// ---------------------------------------------------------------------------

#[test]
fn reads_trae_servers() {
    let dir = TempDir::new();
    dir.write(
        ".trae/mcp.json",
        r#"{
  "mcpServers": {
    "github": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"] },
    "docs": { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer xxxx" } }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::Trae);

    assert_eq!(config.relative_path, Path::new(".trae/mcp.json"));
    assert_eq!(
        server(&config, "github").transport,
        command("npx", &["-y", "@modelcontextprotocol/server-github"])
    );
    assert_eq!(server(&config, "docs").transport, remote("https://example.com/mcp"));
}

#[test]
fn fix_creates_a_trae_config() {
    assert_eq!(
        created_from_claude_code(".trae/rules/project_rules.md", ".trae/mcp.json"),
        CLAUDE_CODE_AS_URL
    );
}

#[test]
fn trae_gets_no_server_that_reads_the_environment() {
    let unfixed = unfixed_env_reference(".trae/rules/project_rules.md");

    assert_eq!(unfixed.len(), 1, "{unfixed:?}");
    assert_eq!(unfixed[0].harness, Harness::Trae);
}

#[test]
fn trae_cn_is_trae() {
    assert_eq!(Harness::parse("trae-cn"), Some(Harness::Trae));
}
