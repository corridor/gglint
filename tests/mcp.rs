//! Reading the MCP servers each coding harness is configured with in a repository.

mod common;

use std::path::Path;

use common::TempDir;
use gglint::harness::Location;
use gglint::harness::{
    Cell, FixOptions, Harness, McpConfig, McpServer, Part, Setting, Transport, Unfixed, check_mcp, compare_mcp,
    fix_mcp, read_mcp, used_harnesses,
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

fn env(name: &str) -> Part {
    Part::Env(name.to_string())
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
        [setting("CI", &[text("1")]), setting("NG_TOKEN", &[env("NG_TOKEN")])]
    );
    assert!(angular.headers.is_empty());
    assert!(angular.enabled);

    let docs = server(config, "docs");
    assert_eq!(docs.location, Location { line: 8, column: 5 });
    assert_eq!(docs.transport, remote("https://docs.example.com/mcp"));
    assert_eq!(
        docs.headers,
        [setting("Authorization", &[text("Bearer "), env("DOCS_KEY")])]
    );
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
    // `env_vars` pass a variable through from the user's environment, under its own name.
    assert_eq!(
        graph.env,
        [
            setting("GRAPH_DEPTH", &[text("3")]),
            setting("GRAPH_HOME", &[env("GRAPH_HOME")])
        ]
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
        [
            setting("Authorization", &[text("Bearer "), env("FIGMA_TOKEN")]),
            setting("X-Team", &[text("design")])
        ]
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
    assert_eq!(angular.env, [setting("NG_TOKEN", &[env("NG_TOKEN")])]);

    let docs = server(config, "docs");
    assert_eq!(docs.transport, remote("https://docs.example.com/mcp"));
    assert_eq!(
        docs.headers,
        [setting("Authorization", &[text("Bearer "), env("DOCS_KEY")])]
    );
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

// ---------------------------------------------------------------------------
// Fixing
// ---------------------------------------------------------------------------

/// Fixes the MCP configs in `dir` the default way, adding only, then checks them again.
fn fix(dir: &TempDir) -> (gglint::harness::FixReport, Vec<String>) {
    fix_with(dir, FixOptions::default())
}

fn fix_with(dir: &TempDir, options: FixOptions) -> (gglint::harness::FixReport, Vec<String>) {
    let configs = read_mcp(dir.path());
    let checks = check_mcp(&configs, &used_harnesses(dir.path()));
    let report = fix_mcp(dir.path(), &configs, &checks, &options);
    (report, findings(dir))
}

fn read(dir: &TempDir, relative: &str) -> String {
    std::fs::read_to_string(dir.path().join(relative)).unwrap()
}

#[test]
fn fix_adds_a_missing_server_in_each_harness_format() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{
  "mcpServers": {
    "docs": { "command": "uvx", "args": ["docs-mcp"], "env": { "DOCS_KEY": "${DOCS_KEY}", "CI": "1" } },
    "search": {
      "type": "http",
      "url": "https://search.example.com/mcp",
      "headers": { "Authorization": "Bearer ${SEARCH_TOKEN}" }
    }
  }
}
"#,
    );
    dir.write(".codex/config.toml", "model = \"gpt-5\"\n");
    dir.write(".cursor/rules/style.mdc", "Be brief.\n");
    dir.write(
        "opencode.json",
        "{\n  \"$schema\": \"https://opencode.ai/config.json\"\n}\n",
    );

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    let changed: Vec<(String, bool, Vec<String>)> = report
        .changed
        .iter()
        .map(|file| {
            (
                file.relative_path.display().to_string(),
                file.created,
                file.added.clone(),
            )
        })
        .collect();
    let both = vec!["docs".to_string(), "search".to_string()];
    assert_eq!(
        changed,
        [
            (".codex/config.toml".to_string(), false, both.clone()),
            (".cursor/mcp.json".to_string(), true, both.clone()),
            ("opencode.json".to_string(), false, both),
        ]
    );
    assert_eq!(
        read(&dir, ".codex/config.toml"),
        r#"model = "gpt-5"

[mcp_servers.docs]
command = "uvx"
args = ["docs-mcp"]
env = { CI = "1" }
env_vars = ["DOCS_KEY"]

[mcp_servers.search]
url = "https://search.example.com/mcp"
bearer_token_env_var = "SEARCH_TOKEN"
"#
    );
    assert_eq!(
        read(&dir, ".cursor/mcp.json"),
        r#"{
  "mcpServers": {
    "docs": {
      "command": "uvx",
      "args": ["docs-mcp"],
      "env": {
        "CI": "1",
        "DOCS_KEY": "${env:DOCS_KEY}"
      }
    },
    "search": {
      "url": "https://search.example.com/mcp",
      "headers": {
        "Authorization": "Bearer ${env:SEARCH_TOKEN}"
      }
    }
  }
}
"#
    );
    assert_eq!(
        read(&dir, "opencode.json"),
        r#"{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "docs": {
        "type": "local",
        "command": ["uvx", "docs-mcp"],
        "environment": {
          "CI": "1",
          "DOCS_KEY": "{env:DOCS_KEY}"
        }
      },
      "search": {
        "type": "remote",
        "url": "https://search.example.com/mcp",
        "headers": {
          "Authorization": "Bearer {env:SEARCH_TOKEN}"
        }
      }
    }
  }
}
"#
    );
}

#[test]
fn fix_creates_a_claude_code_config_with_a_type_for_remote_servers() {
    let dir = TempDir::new();
    dir.write(".claude/settings.json", "{}\n");
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.linear]\nurl = \"https://mcp.linear.app/sse\"\n",
    );

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.changed[0].created);
    assert_eq!(
        read(&dir, ".mcp.json"),
        r#"{
  "mcpServers": {
    "linear": {
      "type": "sse",
      "url": "https://mcp.linear.app/sse"
    }
  }
}
"#
    );
}

#[test]
fn fix_overwrite_makes_a_server_match_the_first_harness_and_keeps_the_rest() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "angular-cli": { "command": "npx", "args": ["-y", "@angular/cli", "mcp", "--read-only"] } } }
"#,
    );
    dir.write(
        ".codex/config.toml",
        "# Codex only.\n[mcp_servers.angular-cli]\ncommand = \"pnpm\"\nargs = [\"dlx\", \"@angular/cli\", \"mcp\"]\n\
         cwd = \".\"\nenabled_tools = [\"list_projects\"]\n",
    );
    dir.write(
        "opencode.jsonc",
        r#"{
  // Shared with the team.
  "mcp": {
    "angular-cli": { "type": "local", "command": ["npx", "-y", "@angular/cli", "mcp"], "enabled": true },
  },
}
"#,
    );

    let (report, left) = fix_with(&dir, FixOptions { overwrite: true });

    assert!(left.is_empty(), "{left:?}");
    let updated: Vec<(String, Vec<String>)> = report
        .changed
        .iter()
        .map(|file| (file.relative_path.display().to_string(), file.updated.clone()))
        .collect();
    assert_eq!(
        updated,
        [
            (".codex/config.toml".to_string(), vec!["angular-cli".to_string()]),
            ("opencode.jsonc".to_string(), vec!["angular-cli".to_string()]),
        ]
    );
    // Keys only Codex has, and comments, are kept.
    assert_eq!(
        read(&dir, ".codex/config.toml"),
        "# Codex only.\n[mcp_servers.angular-cli]\ncommand = \"npx\"\nargs = [\"-y\", \"@angular/cli\", \"mcp\", \"--read-only\"]\n\
         cwd = \".\"\nenabled_tools = [\"list_projects\"]\n"
    );
    assert_eq!(
        read(&dir, "opencode.jsonc"),
        r#"{
  // Shared with the team.
  "mcp": {
    "angular-cli": { "type": "local", "command": ["npx", "-y", "@angular/cli", "mcp", "--read-only"], "enabled": true },
  },
}
"#
    );
}

#[test]
fn fix_leaves_what_it_cannot_decide() {
    let dir = TempDir::new();
    let claude = r#"{
  "mcpServers": {
    "docs": { "type": "http", "url": "https://docs.example.com/mcp" },
    "search": { "command": "uvx", "args": ["search-mcp"], "env": { "SEARCH_KEY": "${SEARCH_KEY:-dev}" } },
    "lint": { "command": "npx", "args": ["lint-mcp"] }
  }
}
"#;
    let codex = "[mcp_servers.docs]\ncommand = \"uvx\"\nargs = [\"docs-mcp\"]\n\n\
                 [mcp_servers.lint]\ncommand = \"npx\"\nargs = [\"lint-mcp\"]\nenabled = false\n";
    dir.write(".mcp.json", claude);
    dir.write(".codex/config.toml", codex);

    let (report, left) = fix(&dir);

    assert!(report.changed.is_empty());
    assert_eq!(read(&dir, ".mcp.json"), claude);
    assert_eq!(read(&dir, ".codex/config.toml"), codex);
    assert_eq!(
        report.unfixed,
        [
            Unfixed {
                harness: Harness::Codex,
                server: "docs".to_string(),
                reason: "it's a different server than in Claude Code; decide which one is right".to_string(),
            },
            Unfixed {
                harness: Harness::Codex,
                server: "search".to_string(),
                reason: "the value of `SEARCH_KEY` can't be written for Codex".to_string(),
            },
        ]
    );
    // A server turned off on purpose is left alone, and still reported.
    assert_eq!(left.len(), 3, "{left:?}");
}

// ---------------------------------------------------------------------------
// Found by running gglint on popular repositories
// ---------------------------------------------------------------------------

#[test]
fn a_file_where_a_harness_directory_belongs_does_not_count() {
    let dir = TempDir::new();
    // In Yeachan-Heo/oh-my-claudecode, `.codex` is an empty file.
    dir.write(".codex", "");
    dir.write(
        ".mcp.json",
        mcp_servers(r#""docs": { "url": "https://docs.example.com/mcp" }"#),
    );

    assert_eq!(used_harnesses(dir.path()), [Harness::ClaudeCode]);
    let configs = read_mcp(dir.path());
    assert_eq!(configs.len(), 1);
    assert!(configs[0].warnings.is_empty(), "{:?}", configs[0].warnings);
    assert!(findings(&dir).is_empty(), "{:?}", findings(&dir));
}

#[test]
fn the_same_reference_in_two_harness_syntaxes_is_the_same_argument() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""local": { "command": "node", "args": ["${ROOT}/server.js"] }"#),
    );
    dir.write(
        ".cursor/mcp.json",
        mcp_servers(r#""local": { "command": "node", "args": ["${env:ROOT}/server.js"] }"#),
    );

    assert!(findings(&dir).is_empty(), "{:?}", findings(&dir));
}

#[test]
fn fix_writes_references_in_the_command_line_in_each_harness_syntax() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(
            r#""local": { "command": "node", "args": ["${ROOT}/server.js", "--port", "8080"] },
               "plain": { "command": "npx", "args": ["-y", "plain-mcp"] }"#,
        ),
    );
    dir.write(".cursor/rules/style.mdc", "Be brief.\n");
    dir.write("opencode.json", "{}\n");
    dir.write(".codex/config.toml", "");

    let (report, _) = fix(&dir);

    let cursor = read(&dir, ".cursor/mcp.json");
    assert!(
        cursor.contains(r#""args": ["${env:ROOT}/server.js", "--port", "8080"]"#),
        "{cursor}"
    );
    let opencode = read(&dir, "opencode.json");
    assert!(
        opencode.contains(r#""command": ["node", "{env:ROOT}/server.js", "--port", "8080"]"#),
        "{opencode}"
    );
    // Codex doesn't fill in references to the environment in a command line.
    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::Codex,
            server: "local".to_string(),
            reason: "the command line uses the environment variable `ROOT`, which Codex can't fill in".to_string(),
        }]
    );
    let codex = read(&dir, ".codex/config.toml");
    assert!(
        codex.contains("[mcp_servers.plain]") && !codex.contains("local"),
        "{codex}"
    );
}

#[test]
fn reads_opencode_servers_in_its_current_layout() {
    let dir = TempDir::new();
    // OpenCode's V2 layout, as in ryan4yin/nix-config and dyoshikawa/rulesync.
    dir.write(
        "opencode.jsonc",
        r#"{
  "mcp": {
    "servers": {
      "grafana": { "type": "local", "command": ["mcp-grafana", "--disable-write"] },
      "deepwiki": { "type": "remote", "url": "https://mcp.deepwiki.com/mcp" },
    },
  },
}
"#,
    );

    let configs = read_mcp(dir.path());

    assert!(configs[0].warnings.is_empty(), "{:?}", configs[0].warnings);
    assert_eq!(names(&configs[0]), ["grafana", "deepwiki"]);
    assert_eq!(server(&configs[0], "grafana").location.line, 4);
}

#[test]
fn a_server_named_servers_in_the_older_layout_is_still_a_server() {
    let dir = TempDir::new();
    dir.write(
        "opencode.json",
        r#"{ "mcp": { "servers": { "type": "remote", "url": "https://servers.example.com/mcp" } } }"#,
    );

    assert_eq!(names(&read_mcp(dir.path())[0]), ["servers"]);
}

#[test]
fn fix_writes_into_the_layout_an_opencode_file_already_uses() {
    let dir = TempDir::new();
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\n",
    );
    dir.write(
        "opencode.jsonc",
        "{\n  \"mcp\": {\n    \"servers\": {\n      \"grafana\": { \"type\": \"local\", \"command\": [\"mcp-grafana\"] },\n    },\n  },\n}\n",
    );

    let (_, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    let opencode = read(&dir, "opencode.jsonc");
    assert!(
        opencode.contains("\"servers\": {\n      \"grafana\"")
            && opencode.contains("\"docs\": {\n        \"type\": \"remote\""),
        "{opencode}"
    );
}

#[test]
fn a_missing_server_names_the_file_the_harness_already_has() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""docs": { "url": "https://docs.example.com/mcp" }"#),
    );
    dir.write("opencode.jsonc", "{ \"mcp\": {} }\n");

    assert_eq!(
        findings(&dir),
        [".mcp.json:1  missing-server  `docs` is set up for Claude Code but not for OpenCode (opencode.jsonc)"]
    );
}

// ---------------------------------------------------------------------------
// Fixing safely: add, never overwrite
// ---------------------------------------------------------------------------

#[test]
fn fix_leaves_a_server_set_up_differently_as_it_is() {
    let dir = TempDir::new();
    // As in Q00/ouroboros: each harness runs the server with its own runtime, on purpose.
    let claude = mcp_servers(
        r#""ouroboros": { "command": "ouroboros", "args": ["mcp", "serve", "--runtime", "claude-cli"], "env": { "A": "1" } }"#,
    );
    let codex = "[mcp_servers.ouroboros]\ncommand = \"ouroboros\"\nargs = [\"mcp\", \"serve\", \"--runtime\", \"codex\"]\n\
                 env = { A = \"1\", B = \"2\" }\n";
    dir.write(".mcp.json", &claude);
    dir.write(".codex/config.toml", codex);

    let (report, left) = fix(&dir);

    assert!(report.changed.is_empty());
    assert_eq!(read(&dir, ".mcp.json"), claude);
    assert_eq!(read(&dir, ".codex/config.toml"), codex);
    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::Codex,
            server: "ouroboros".to_string(),
            reason: "set up differently from Claude Code; left as it is, and `--overwrite` copies Claude Code's setup"
                .to_string(),
        }]
    );
    assert_eq!(left.len(), 2, "{left:?}");
}

#[test]
fn fix_creates_a_config_file_only_for_a_harness_with_its_own_directory() {
    let dir = TempDir::new();
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\n",
    );
    // Claude Code is known only from CLAUDE.md, Cursor from its own directory.
    dir.write("CLAUDE.md", "@AGENTS.md\n");
    dir.write(".cursor/rules/style.mdc", "Be brief.\n");

    let (report, left) = fix(&dir);

    assert!(!dir.path().join(".mcp.json").exists());
    assert!(dir.path().join(".cursor/mcp.json").exists());
    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::ClaudeCode,
            server: "docs".to_string(),
            reason:
                "Claude Code has no .mcp.json and no .claude directory; create .mcp.json if its users want MCP servers"
                    .to_string(),
        }]
    );
    assert_eq!(left.len(), 1, "{left:?}");
}

#[test]
fn fix_says_when_it_writes_a_file_the_repository_ignores() {
    let dir = TempDir::new();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    dir.write(".gitignore", ".mcp.json\n");
    dir.write(".claude/settings.json", "{}\n");
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\n",
    );
    dir.write(".cursor/rules/style.mdc", "Be brief.\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    let ignored: Vec<(String, bool)> = report
        .changed
        .iter()
        .map(|file| (file.relative_path.display().to_string(), file.ignored))
        .collect();
    assert_eq!(
        ignored,
        [(".mcp.json".to_string(), true), (".cursor/mcp.json".to_string(), false)]
    );
}

#[test]
fn fix_never_removes_or_changes_what_is_there() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        mcp_servers(r#""docs": { "url": "https://docs.example.com/mcp" }, "lint": { "command": "npx", "args": ["lint-mcp@1"] }"#),
    );
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.lint]\ncommand = \"npx\"\nargs = [\"lint-mcp@2\"]\ncwd = \".\"\n",
    );
    let before = read_mcp(dir.path());

    fix(&dir);

    let after = read_mcp(dir.path());
    for config in &before {
        let now = after
            .iter()
            .find(|other| other.relative_path == config.relative_path)
            .unwrap();
        for server in &config.servers {
            let kept = now.servers.iter().find(|other| other.name == server.name).unwrap();
            assert_eq!(
                (&kept.transport, &kept.env, &kept.headers),
                (&server.transport, &server.env, &server.headers)
            );
        }
    }
    // `docs` was added to Codex; `lint`, at another version there, was left alone.
    assert!(read(&dir, ".codex/config.toml").contains("[mcp_servers.docs]"));
    assert!(read(&dir, ".codex/config.toml").contains("lint-mcp@2"));
}
