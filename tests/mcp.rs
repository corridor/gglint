//! Reading the MCP servers each coding harness is configured with in a repository.

mod common;

use std::path::Path;

use common::TempDir;
use gglint::discovery::Location;
use gglint::harness::{
    Cell, Harness, McpConfig, McpServer, Setting, Transport, check_mcp, compare_mcp, read_mcp, used_harnesses,
};

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

// ---------------------------------------------------------------------------
// Comparing harnesses
// ---------------------------------------------------------------------------

/// Each finding as `path:line  rule  message`.
fn findings(dir: &TempDir) -> Vec<String> {
    let configs = read_mcp(dir.path());
    compare_mcp(&configs, &used_harnesses(dir.path()))
        .iter()
        .map(|finding| {
            format!(
                "{}:{}  {}  {}",
                finding.site.relative_path.display(),
                finding.site.location.line,
                finding.rule.as_str(),
                finding.message
            )
        })
        .collect()
}

fn mcp_servers(servers: &str) -> String {
    format!("{{ \"mcpServers\": {{ {servers} }} }}\n")
}

/// GGX's Codex and Cursor configs, with a `.claude` directory and no `.mcp.json`.
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
        mcp_servers(r#""angular-cli": { "command": "npx", "args": ["-y", "@angular/cli", "mcp"] }"#),
    );
    dir
}

#[test]
fn finds_the_harnesses_a_repository_is_set_up_for() {
    let dir = TempDir::new();
    dir.write("AGENTS.md", "# Shared by several harnesses\n");
    assert!(used_harnesses(dir.path()).is_empty());

    dir.write(".codex/config.toml", "");
    dir.write("CLAUDE.md", "@AGENTS.md\n");
    assert_eq!(used_harnesses(dir.path()), [Harness::ClaudeCode, Harness::Codex]);
}

#[test]
fn reports_servers_missing_from_a_harness_the_repository_uses() {
    let dir = ggx_like();

    assert_eq!(
        findings(&dir),
        [
            ".codex/config.toml:1  missing-server  `code-review-graph` is set up for Codex but not for Claude Code \
             (.mcp.json) or Cursor (.cursor/mcp.json)",
            ".codex/config.toml:5  missing-server  `angular-cli` is set up for Codex and Cursor but not for Claude \
             Code (.mcp.json)",
            ".codex/config.toml:5  different-server  `angular-cli` is started with different arguments: \
             `mcp --read-only` in Codex; `mcp` in Cursor",
        ]
    );
}

#[test]
fn harnesses_the_repository_is_not_set_up_for_are_left_out() {
    let dir = TempDir::new();
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\n",
    );
    dir.write(
        ".cursor/mcp.json",
        mcp_servers(r#""docs": { "url": "https://docs.example.com/mcp" }"#),
    );

    assert!(findings(&dir).is_empty(), "{:?}", findings(&dir));
}

#[test]
fn the_same_package_started_another_way_is_the_same_server() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""angular-cli": { "command": "npx", "args": ["-y", "@angular/cli@latest", "mcp"] }"#),
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.angular-cli]\ncommand = \"pnpm\"\nargs = [\"dlx\", \"@angular/cli\", \"mcp\"]\n",
    );
    dir.write(
        ".cursor/mcp.json",
        mcp_servers(r#""angular-cli": { "command": "bunx", "args": ["@angular/cli", "mcp"] }"#),
    );
    dir.write(
        "opencode.json",
        r#"{ "mcp": { "angular-cli": { "type": "local", "command": ["npx", "--yes", "@angular/cli", "mcp"] } } }"#,
    );

    assert!(findings(&dir).is_empty(), "{:?}", findings(&dir));
}

#[test]
fn reports_different_versions_of_a_package() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""docs": { "command": "npx", "args": ["-y", "docs-mcp@1.2.0"] }"#),
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\ncommand = \"npx\"\nargs = [\"-y\", \"docs-mcp@1.3.0\"]\n",
    );
    dir.write(
        ".cursor/mcp.json",
        mcp_servers(r#""docs": { "command": "npx", "args": ["-y", "docs-mcp"] }"#),
    );

    assert_eq!(
        findings(&dir),
        [
            ".mcp.json:1  different-server  `docs` runs different versions of the npm package `docs-mcp`: 1.2.0 \
          in Claude Code; 1.3.0 in Codex; the newest in Cursor"
        ]
    );
}

#[test]
fn reports_a_different_server_under_the_same_name() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""docs": { "type": "http", "url": "https://docs.example.com/mcp" }"#),
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\ncommand = \"uvx\"\nargs = [\"docs-mcp==2.0\"]\n",
    );

    assert_eq!(
        findings(&dir),
        [
            ".mcp.json:1  different-server  `docs` is a different server in each harness: \
          https://docs.example.com/mcp in Claude Code; the PyPI package `docs-mcp` in Codex"
        ]
    );
}

#[test]
fn reports_different_environment_variables_and_headers() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(
            r#""angular-cli": { "command": "npx", "args": ["@angular/cli", "mcp"], "env": { "NG_TOKEN": "${NG_TOKEN}" } },
               "docs": { "url": "https://docs.example.com/mcp", "headers": { "Authorization": "Bearer ${KEY}" } }"#,
        ),
    );
    dir.write(
        ".cursor/mcp.json",
        mcp_servers(
            r#""angular-cli": { "command": "npx", "args": ["@angular/cli", "mcp"] },
               "docs": { "url": "https://docs.example.com/mcp" }"#,
        ),
    );

    assert_eq!(
        findings(&dir),
        [
            ".mcp.json:1  different-server  `angular-cli` is given different environment variables: NG_TOKEN in \
             Claude Code; none in Cursor",
            ".mcp.json:2  different-server  `docs` is sent different HTTP headers: Authorization in Claude Code; \
             none in Cursor",
        ]
    );
}

#[test]
fn reports_a_server_turned_off_in_one_harness() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""docs": { "url": "https://docs.example.com/mcp" }"#),
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\nenabled = false\n",
    );

    assert_eq!(
        findings(&dir),
        [".codex/config.toml:1  different-server  `docs` is turned off in Codex but not in Claude Code"]
    );
}

#[test]
fn matches_a_renamed_server_by_what_it_runs() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""angular": { "command": "npx", "args": ["-y", "@angular/cli", "mcp"] }"#),
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.angular-cli]\ncommand = \"pnpm\"\nargs = [\"dlx\", \"@angular/cli\", \"mcp\"]\n",
    );

    assert_eq!(
        findings(&dir),
        [
            ".mcp.json:1  renamed-server  One server has different names: `angular` in Claude Code; `angular-cli` \
          in Codex"
        ]
    );
}

// ---------------------------------------------------------------------------
// Each server in each harness
// ---------------------------------------------------------------------------

/// Each server's name and its cell in each harness the repository is set up for.
fn table(dir: &TempDir) -> Vec<(String, Vec<(Harness, Cell)>)> {
    check_mcp(&read_mcp(dir.path()), &used_harnesses(dir.path()))
        .into_iter()
        .map(|check| (check.name, check.cells))
        .collect()
}

#[test]
fn gives_each_server_a_cell_in_each_harness() {
    use Harness::{ClaudeCode, Codex, Cursor};

    let dir = ggx_like();

    assert_eq!(
        table(&dir),
        [
            (
                "code-review-graph".to_string(),
                vec![
                    (ClaudeCode, Cell::Missing),
                    (Codex, Cell::Same),
                    (Cursor, Cell::Missing)
                ]
            ),
            // Compared with the first harness that has it: Codex.
            (
                "angular-cli".to_string(),
                vec![
                    (ClaudeCode, Cell::Missing),
                    (Codex, Cell::Same),
                    (Cursor, Cell::Different)
                ]
            ),
        ]
    );
}

#[test]
fn a_server_turned_off_is_off_in_that_harness() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""docs": { "url": "https://docs.example.com/mcp" }"#),
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\nenabled = false\n",
    );

    assert_eq!(
        table(&dir),
        [(
            "docs".to_string(),
            vec![(Harness::ClaudeCode, Cell::Same), (Harness::Codex, Cell::Off)]
        )]
    );
}

#[test]
fn describes_each_finding_and_how_to_fix_a_missing_server() {
    let dir = ggx_like();

    let checks = check_mcp(&read_mcp(dir.path()), &used_harnesses(dir.path()));

    let described: Vec<(&str, Option<&str>)> = checks
        .iter()
        .flat_map(|check| &check.findings)
        .map(|finding| (finding.detail.as_str(), finding.fix.as_deref()))
        .collect();
    assert_eq!(
        described,
        [
            (
                "Missing from Claude Code and Cursor.",
                Some("Add it to .mcp.json and .cursor/mcp.json, as set up in .codex/config.toml:1")
            ),
            (
                "Missing from Claude Code.",
                Some("Add it to .mcp.json, as set up in .codex/config.toml:5")
            ),
            (
                "Started with different arguments: `mcp --read-only` in Codex; `mcp` in Cursor.",
                None
            ),
        ]
    );
}
