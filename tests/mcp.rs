//! Reading the MCP servers each coding harness is configured with in a repository.

mod common;

use std::path::Path;

use common::TempDir;
use gglint::discovery::Location;
use gglint::harness::{Harness, McpConfig, McpServer, Setting, Transport, read_mcp};

fn setting(name: &str, value: Option<&str>) -> Setting {
    Setting {
        name: name.to_string(),
        value: value.map(str::to_string),
    }
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

fn names(config: &McpConfig) -> Vec<&str> {
    config.servers.iter().map(|server| server.name.as_str()).collect()
}

fn server<'a>(config: &'a McpConfig, name: &str) -> &'a McpServer {
    config
        .servers
        .iter()
        .find(|server| server.name == name)
        .unwrap_or_else(|| panic!("no server {name:?} in {:?}", names(config)))
}

const CLAUDE_CODE: &str = r#"{
  "mcpServers": {
    "angular-cli": {
      "command": "npx",
      "args": ["-y", "@angular/cli", "mcp"],
      "env": { "NG_TOKEN": "${NG_TOKEN}", "CI": "1" }
    },
    "docs": {
      "type": "http",
      "url": "https://docs.example.com/mcp",
      "headers": { "Authorization": "Bearer ${DOCS_KEY}" }
    }
  }
}
"#;

const CODEX: &str = r#"# Project-scoped Codex MCP configuration.
[mcp_servers."code-review-graph"]
command = "uvx"
args = ["code-review-graph@latest", "serve"]
cwd = "."
env = { GRAPH_DEPTH = "3" }
env_vars = ["GRAPH_HOME"]
enabled_tools = ["search", "explain"]

[mcp_servers.chrome-devtools]
command = "pnpm"
args = ["dlx", "chrome-devtools-mcp@latest"]
enabled = false
disabled_tools = ["evaluate_script"]

[mcp_servers.figma]
url = "https://mcp.figma.com/mcp"
http_headers = { "X-Team" = "design" }
bearer_token_env_var = "FIGMA_TOKEN"
"#;

const CURSOR: &str = r#"{
  "mcpServers": {
    "angular-cli": {
      "command": "npx",
      "args": ["-y", "@angular/cli", "mcp"]
    },
    "linear": { "url": "https://mcp.linear.app/sse" }
  }
}
"#;

const OPENCODE: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  // Servers for everyone working on this repository.
  "mcp": {
    "angular-cli": {
      "type": "local",
      "command": ["npx", "-y", "@angular/cli", "mcp"],
      "environment": { "NG_TOKEN": "{env:NG_TOKEN}" },
    },
    "docs": {
      "type": "remote",
      "url": "https://docs.example.com/mcp",
      "headers": { "Authorization": "Bearer {env:DOCS_KEY}" },
      "enabled": false,
    },
  },
}
"#;

#[test]
fn reads_claude_code_servers() {
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);

    let configs = read_mcp(dir.path());

    assert_eq!(configs.len(), 1);
    let config = &configs[0];
    assert_eq!(config.harness, Harness::ClaudeCode);
    assert_eq!(config.relative_path, Path::new(".mcp.json"));
    assert_eq!(config.path, dir.path().join(".mcp.json"));
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    assert_eq!(names(config), ["angular-cli", "docs"]);

    let angular = server(config, "angular-cli");
    assert_eq!(angular.location, Location { line: 3, column: 5 });
    assert_eq!(angular.transport, command("npx", &["-y", "@angular/cli", "mcp"]));
    assert_eq!(
        angular.env,
        [setting("CI", Some("1")), setting("NG_TOKEN", Some("${NG_TOKEN}"))]
    );
    assert!(angular.headers.is_empty());
    assert!(angular.enabled);

    let docs = server(config, "docs");
    assert_eq!(docs.location, Location { line: 8, column: 5 });
    assert_eq!(docs.transport, remote("https://docs.example.com/mcp"));
    assert_eq!(docs.headers, [setting("Authorization", Some("Bearer ${DOCS_KEY}"))]);
}

#[test]
fn reads_codex_servers() {
    let dir = TempDir::new();
    dir.write(".codex/config.toml", CODEX);

    let configs = read_mcp(dir.path());

    assert_eq!(configs.len(), 1);
    let config = &configs[0];
    assert_eq!(config.harness, Harness::Codex);
    assert_eq!(config.relative_path, Path::new(".codex/config.toml"));
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    // In the order they're written, not alphabetical.
    assert_eq!(names(config), ["code-review-graph", "chrome-devtools", "figma"]);

    let graph = server(config, "code-review-graph");
    assert_eq!(graph.location.line, 2);
    assert_eq!(graph.transport, command("uvx", &["code-review-graph@latest", "serve"]));
    // `env_vars` pass a variable through from the user's environment: no value here.
    assert_eq!(
        graph.env,
        [setting("GRAPH_DEPTH", Some("3")), setting("GRAPH_HOME", None)]
    );
    assert_eq!(
        graph.enabled_tools,
        Some(vec!["search".to_string(), "explain".to_string()])
    );
    assert!(graph.disabled_tools.is_empty());
    assert!(graph.enabled);

    let chrome = server(config, "chrome-devtools");
    assert_eq!(chrome.location.line, 10);
    assert!(!chrome.enabled);
    assert_eq!(chrome.enabled_tools, None);
    assert_eq!(chrome.disabled_tools, ["evaluate_script"]);

    let figma = server(config, "figma");
    assert_eq!(figma.transport, remote("https://mcp.figma.com/mcp"));
    // The bearer token comes from the user's environment.
    assert_eq!(
        figma.headers,
        [setting("Authorization", None), setting("X-Team", Some("design"))]
    );
}

#[test]
fn reads_codex_servers_written_as_dotted_keys() {
    let dir = TempDir::new();
    dir.write(
        ".codex/config.toml",
        "model = \"gpt-5\"\n\n[mcp_servers]\ndocs.url = \"https://docs.example.com/mcp\"\n",
    );

    let configs = read_mcp(dir.path());

    let docs = server(&configs[0], "docs");
    assert_eq!(docs.location.line, 4);
    assert_eq!(docs.transport, remote("https://docs.example.com/mcp"));
}

#[test]
fn reads_cursor_servers() {
    let dir = TempDir::new();
    dir.write(".cursor/mcp.json", CURSOR);

    let configs = read_mcp(dir.path());

    assert_eq!(configs.len(), 1);
    let config = &configs[0];
    assert_eq!(config.harness, Harness::Cursor);
    assert_eq!(names(config), ["angular-cli", "linear"]);
    assert_eq!(
        server(config, "angular-cli").transport,
        command("npx", &["-y", "@angular/cli", "mcp"])
    );
    // A server with a `url` and no `type` is remote.
    assert_eq!(server(config, "linear").transport, remote("https://mcp.linear.app/sse"));
}

#[test]
fn reads_opencode_servers_with_comments_and_trailing_commas() {
    let dir = TempDir::new();
    dir.write("opencode.jsonc", OPENCODE);

    let configs = read_mcp(dir.path());

    assert_eq!(configs.len(), 1);
    let config = &configs[0];
    assert_eq!(config.harness, Harness::OpenCode);
    assert_eq!(config.relative_path, Path::new("opencode.jsonc"));
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    assert_eq!(names(config), ["angular-cli", "docs"]);

    let angular = server(config, "angular-cli");
    assert_eq!(angular.location, Location { line: 5, column: 5 });
    // OpenCode writes the command and its arguments as one list.
    assert_eq!(angular.transport, command("npx", &["-y", "@angular/cli", "mcp"]));
    assert_eq!(angular.env, [setting("NG_TOKEN", Some("{env:NG_TOKEN}"))]);

    let docs = server(config, "docs");
    assert_eq!(docs.transport, remote("https://docs.example.com/mcp"));
    assert_eq!(docs.headers, [setting("Authorization", Some("Bearer {env:DOCS_KEY}"))]);
    assert!(!docs.enabled);
}

#[test]
fn reads_every_harness_in_a_fixed_order() {
    let dir = TempDir::new();
    dir.write("opencode.json", OPENCODE);
    dir.write(".cursor/mcp.json", CURSOR);
    dir.write(".codex/config.toml", CODEX);
    dir.write(".mcp.json", CLAUDE_CODE);

    let configs = read_mcp(dir.path());

    let harnesses: Vec<_> = configs.iter().map(|config| config.harness).collect();
    assert_eq!(
        harnesses,
        [Harness::ClaudeCode, Harness::Codex, Harness::Cursor, Harness::OpenCode]
    );
}

#[test]
fn a_repository_without_config_files_has_no_configs() {
    let dir = TempDir::new();
    dir.write("README.md", "# Nothing to see\n");

    assert!(read_mcp(dir.path()).is_empty());
}

#[test]
fn a_file_without_mcp_servers_is_still_listed() {
    let dir = TempDir::new();
    dir.write(".codex/config.toml", "model = \"gpt-5\"\n");

    let configs = read_mcp(dir.path());

    assert_eq!(configs.len(), 1);
    assert!(configs[0].servers.is_empty());
    assert!(configs[0].warnings.is_empty());
}

#[test]
fn a_file_that_does_not_parse_gives_a_warning() {
    let dir = TempDir::new();
    dir.write(".mcp.json", "{ \"mcpServers\": { \"docs\": ");
    dir.write(".codex/config.toml", "[mcp_servers.docs\nurl = 1\n");

    let configs = read_mcp(dir.path());

    assert_eq!(configs.len(), 2);
    for config in &configs {
        assert!(config.servers.is_empty());
        assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
        assert!(
            config.warnings[0].starts_with("couldn't parse"),
            "{:?}",
            config.warnings
        );
    }
}

#[test]
fn a_server_without_a_command_or_url_gives_a_warning() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "broken": { "args": ["x"] }, "docs": { "url": "https://docs.example.com/mcp" } } }"#,
    );

    let configs = read_mcp(dir.path());

    assert_eq!(names(&configs[0]), ["docs"]);
    assert_eq!(configs[0].warnings, ["`broken` has neither a `command` nor a `url`"]);
}
