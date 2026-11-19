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
    created_from(".mcp.json", CLAUDE_CODE, directory_file, file)
}

/// `CLAUDE_CODE`'s servers, as Cursor writes them.
const CURSOR: &str = r#"{
  "mcpServers": {
    "playwright": { "command": "npx", "args": ["-y", "@playwright/mcp@0.0.78"], "env": { "CI": "1" } },
    "docs": { "url": "https://docs.example.com/mcp", "headers": { "X-Team": "web" } }
  }
}
"#;

/// Fixes a repository with `CLAUDE_CODE`'s servers in Cursor's config, and the agent's
/// directory, and returns the config file `--fix` created at `file`.
fn created_from_cursor(directory_file: &str, file: &str) -> String {
    created_from(".cursor/mcp.json", CURSOR, directory_file, file)
}

/// Fixes a repository with `source_text` at `source_file`, and the agent's directory, and
/// returns the config file `--fix` created at `file`.
fn created_from(source_file: &str, source_text: &str, directory_file: &str, file: &str) -> String {
    let dir = TempDir::new();
    dir.write(source_file, source_text);
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

// ---------------------------------------------------------------------------
// Kiro
// ---------------------------------------------------------------------------

const KIRO: &str = r#"{
  "mcpServers": {
    "web-search": {
      "command": "uvx",
      "args": ["web-search-mcp"],
      "env": { "API_KEY": "${YOUR_API_KEY}", "DEBUG": "true" },
      "autoApprove": ["search"]
    },
    "api-server": {
      "url": "https://api.example.com/mcp",
      "headers": { "Authorization": "Bearer ${API_TOKEN}" },
      "disabled": true
    }
  }
}
"#;

#[test]
fn reads_kiro_servers() {
    let dir = TempDir::new();
    dir.write(".kiro/settings/mcp.json", KIRO);

    let config = config_of(&dir, Harness::Kiro);

    assert_eq!(config.relative_path, Path::new(".kiro/settings/mcp.json"));
    let search = server(&config, "web-search");
    assert_eq!(search.transport, command("uvx", &["web-search-mcp"]));
    assert_eq!(
        search.env,
        [
            setting("API_KEY", &[env("YOUR_API_KEY")]),
            setting("DEBUG", &[text("true")])
        ]
    );
    assert!(search.enabled);
    let api = server(&config, "api-server");
    assert_eq!(api.transport, remote("https://api.example.com/mcp"));
    assert_eq!(
        api.headers,
        [setting("Authorization", &[text("Bearer "), env("API_TOKEN")])]
    );
    assert!(!api.enabled, "`disabled: true` turns it off");
}

#[test]
fn fix_creates_a_kiro_config() {
    assert_eq!(
        created_from_claude_code(".kiro/steering/product.md", ".kiro/settings/mcp.json"),
        CLAUDE_CODE_AS_URL
    );
}

#[test]
fn kiro_gets_values_that_read_the_environment() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "search": { "command": "uvx", "args": ["search-mcp"], "env": { "TOKEN": "${TOKEN}" } } } }"#,
    );
    dir.write(".kiro/steering/product.md", "Be brief.\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    assert!(
        read(&dir, ".kiro/settings/mcp.json").contains(r#""TOKEN": "${TOKEN}""#),
        "{}",
        read(&dir, ".kiro/settings/mcp.json")
    );
}

#[test]
fn kiro_gets_no_command_line_that_reads_the_environment() {
    // Kiro's docs show `${VAR}` in `env` and `headers` only.
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "files": { "command": "npx", "args": ["files-mcp", "${ROOT}"] } } }"#,
    );
    dir.write(".kiro/steering/product.md", "Be brief.\n");

    let (report, _) = fix(&dir);

    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::Kiro,
            server: "files".to_string(),
            reason: "the command line uses the environment variable `ROOT`, which Kiro can't fill in".to_string(),
        }]
    );
}

#[test]
fn a_server_disabled_in_kiro_is_turned_off() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "api-server": { "type": "http", "url": "https://api.example.com/mcp", "headers": { "Authorization": "Bearer ${API_TOKEN}" } } } }"#,
    );
    dir.write(".kiro/settings/mcp.json", KIRO);

    let found = findings(&dir);

    assert!(
        found.contains(&"different-server  `api-server` is turned off in Kiro but not in Claude Code".to_string()),
        "{found:?}"
    );
}

/// `CLAUDE_CODE`'s servers, written in the `mcpServers` format of an agent that gives a
/// remote server a `type`, as Claude Code does.
const CLAUDE_CODE_AS_TYPED_URL: &str = r#"{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": ["-y", "@playwright/mcp@0.0.78"],
      "env": {
        "CI": "1"
      }
    },
    "docs": {
      "type": "http",
      "url": "https://docs.example.com/mcp",
      "headers": {
        "X-Team": "web"
      }
    }
  }
}
"#;

// ---------------------------------------------------------------------------
// Factory Droid
// ---------------------------------------------------------------------------

#[test]
fn reads_factory_droid_servers() {
    let dir = TempDir::new();
    dir.write(
        ".factory/mcp.json",
        r#"{
  "mcpServers": {
    "playwright": { "type": "stdio", "command": "npx", "args": ["-y", "@playwright/mcp@latest"], "env": { "TOKEN": "${PW_TOKEN}" } },
    "context7": {
      "type": "http",
      "url": "https://mcp.context7.com/mcp",
      "headers": { "CONTEXT7_API_KEY": "${CONTEXT7_API_KEY}" },
      "disabled": true
    }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::FactoryDroid);

    assert_eq!(config.relative_path, Path::new(".factory/mcp.json"));
    let playwright = server(&config, "playwright");
    assert_eq!(playwright.transport, command("npx", &["-y", "@playwright/mcp@latest"]));
    assert_eq!(playwright.env, [setting("TOKEN", &[env("PW_TOKEN")])]);
    let context7 = server(&config, "context7");
    assert_eq!(context7.transport, remote("https://mcp.context7.com/mcp"));
    assert_eq!(
        context7.headers,
        [setting("CONTEXT7_API_KEY", &[env("CONTEXT7_API_KEY")])]
    );
    assert!(!context7.enabled);
}

#[test]
fn fix_creates_a_factory_droid_config_with_typed_remote_servers() {
    assert_eq!(
        created_from_claude_code(".factory/droids/reviewer.md", ".factory/mcp.json"),
        CLAUDE_CODE_AS_TYPED_URL
    );
}

#[test]
fn factory_droid_gets_values_but_not_command_lines_that_read_the_environment() {
    // Its docs: `${NAME}` is filled in in `env` and `headers`, not `command`, `args` or `url`.
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": {
  "search": { "command": "uvx", "args": ["search-mcp"], "env": { "TOKEN": "${TOKEN}" } },
  "files": { "command": "npx", "args": ["files-mcp", "${ROOT}"] }
} }"#,
    );
    dir.write(".factory/droids/reviewer.md", "Be brief.\n");

    let (report, _) = fix(&dir);

    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::FactoryDroid,
            server: "files".to_string(),
            reason: "the command line uses the environment variable `ROOT`, which Factory Droid can't fill in"
                .to_string(),
        }]
    );
    assert!(read(&dir, ".factory/mcp.json").contains(r#""TOKEN": "${TOKEN}""#));
}

#[test]
fn droid_and_factory_are_factory_droid() {
    assert_eq!(Harness::parse("droid"), Some(Harness::FactoryDroid));
    assert_eq!(Harness::parse("factory"), Some(Harness::FactoryDroid));
}

// ---------------------------------------------------------------------------
// Qoder
// ---------------------------------------------------------------------------

#[test]
fn reads_qoder_servers() {
    let dir = TempDir::new();
    dir.write(
        ".qoder/settings.json",
        r#"{
  "model": "auto",
  "mcpServers": {
    "my-server": { "command": "node", "args": ["./mcp-server.js"], "env": { "API_KEY": "..." }, "cwd": "tools" },
    "remote": { "type": "http", "url": "https://mcp.example.com/mcp", "disabled": true }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::Qoder);

    assert_eq!(config.relative_path, Path::new(".qoder/settings.json"));
    assert_eq!(
        server(&config, "my-server").transport,
        command("node", &["./mcp-server.js"])
    );
    let remote_server = server(&config, "remote");
    assert_eq!(remote_server.transport, remote("https://mcp.example.com/mcp"));
    assert!(!remote_server.enabled);
}

#[test]
fn fix_creates_a_qoder_config_with_typed_remote_servers() {
    assert_eq!(
        created_from_cursor(".qoder/rules/style.md", ".qoder/settings.json"),
        CLAUDE_CODE_AS_TYPED_URL
    );
}

#[test]
fn fix_keeps_the_other_settings_in_a_qoder_settings_file() {
    let dir = TempDir::new();
    dir.write(".cursor/mcp.json", CURSOR);
    dir.write(".qoder/settings.json", "{\n  \"model\": \"auto\"\n}\n");

    let (_, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    let settings = read(&dir, ".qoder/settings.json");
    assert!(
        settings.starts_with("{\n  \"model\": \"auto\",\n  \"mcpServers\": {"),
        "{settings}"
    );
}

// ---------------------------------------------------------------------------
// Kimi Code
// ---------------------------------------------------------------------------

#[test]
fn reads_kimi_code_servers() {
    let dir = TempDir::new();
    dir.write(
        ".kimi-code/mcp.json",
        r#"{
  "mcpServers": {
    "context7": { "command": "npx", "args": ["-y", "@upstash/context7-mcp"] },
    "linear": { "url": "https://mcp.linear.app/mcp" },
    "legacy-events": { "transport": "sse", "url": "https://mcp.example.com/sse", "enabled": false }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::KimiCode);

    assert_eq!(config.relative_path, Path::new(".kimi-code/mcp.json"));
    assert_eq!(
        server(&config, "context7").transport,
        command("npx", &["-y", "@upstash/context7-mcp"])
    );
    assert_eq!(
        server(&config, "linear").transport,
        remote("https://mcp.linear.app/mcp")
    );
    let events = server(&config, "legacy-events");
    assert_eq!(events.transport, remote("https://mcp.example.com/sse"));
    assert!(!events.enabled, "`enabled: false` turns it off");
}

#[test]
fn fix_creates_a_kimi_code_config() {
    assert_eq!(
        created_from_claude_code(".kimi-code/skills/review/SKILL.md", ".kimi-code/mcp.json"),
        CLAUDE_CODE_AS_URL
    );
}

#[test]
fn fix_marks_an_sse_server_for_kimi_code() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "linear": { "type": "sse", "url": "https://mcp.linear.app/sse" } } }"#,
    );
    dir.write(".kimi-code/mcp.json", "{}\n");

    let (_, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert_eq!(
        read(&dir, ".kimi-code/mcp.json"),
        "{\n  \"mcpServers\": {\n    \"linear\": {\n      \"transport\": \"sse\",\n      \
         \"url\": \"https://mcp.linear.app/sse\"\n    }\n  }\n}\n"
    );
}

#[test]
fn reads_the_bearer_token_kimi_code_takes_from_the_environment() {
    let dir = TempDir::new();
    dir.write(
        ".kimi-code/mcp.json",
        r#"{ "mcpServers": { "linear": { "url": "https://mcp.linear.app/mcp", "bearerTokenEnvVar": "LINEAR_TOKEN" } } }"#,
    );

    let config = config_of(&dir, Harness::KimiCode);

    assert_eq!(
        server(&config, "linear").headers,
        [setting("Authorization", &[text("Bearer "), env("LINEAR_TOKEN")])]
    );
}

#[test]
fn fix_gives_kimi_code_a_bearer_token_from_the_environment() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "linear": { "type": "http", "url": "https://mcp.linear.app/mcp", "headers": { "Authorization": "Bearer ${LINEAR_TOKEN}" } } } }"#,
    );
    dir.write(".kimi-code/mcp.json", "{}\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    assert_eq!(
        read(&dir, ".kimi-code/mcp.json"),
        "{\n  \"mcpServers\": {\n    \"linear\": {\n      \"url\": \"https://mcp.linear.app/mcp\",\n      \
         \"bearerTokenEnvVar\": \"LINEAR_TOKEN\"\n    }\n  }\n}\n"
    );
}

// ---------------------------------------------------------------------------
// Gemini CLI
// ---------------------------------------------------------------------------

#[test]
fn reads_gemini_cli_servers() {
    let dir = TempDir::new();
    dir.write(
        ".gemini/settings.json",
        r#"{
  "theme": "GitHub",
  "mcpServers": {
    "pythonTools": {
      "command": "python",
      "args": ["-m", "my_mcp_server", "--root", "${PWD}"],
      "env": { "DATABASE_URL": "$DB_CONNECTION_STRING", "API_KEY": "${EXTERNAL_API_KEY}" },
      "timeout": 15000
    },
    "httpServer": { "httpUrl": "http://localhost:3000/mcp", "headers": { "Authorization": "Bearer $TOKEN" } },
    "sseServer": { "url": "http://localhost:8080/sse" }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::GeminiCli);

    assert_eq!(config.relative_path, Path::new(".gemini/settings.json"));
    let python = server(&config, "pythonTools");
    assert_eq!(
        python.transport,
        command("python", &["-m", "my_mcp_server", "--root", "${PWD}"])
    );
    assert_eq!(
        python.env,
        [
            setting("API_KEY", &[env("EXTERNAL_API_KEY")]),
            setting("DATABASE_URL", &[env("DB_CONNECTION_STRING")]),
        ]
    );
    let http = server(&config, "httpServer");
    assert_eq!(http.transport, remote("http://localhost:3000/mcp"));
    assert_eq!(
        http.headers,
        [setting("Authorization", &[text("Bearer "), env("TOKEN")])]
    );
    assert_eq!(
        server(&config, "sseServer").transport,
        remote("http://localhost:8080/sse")
    );
}

#[test]
fn a_gemini_directory_alone_is_not_gemini_cli() {
    // Gemini Code Assist keeps its review settings in `.gemini` too.
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".gemini/styleguide.md", "Be brief.\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.changed.is_empty(), "{:?}", report.changed);
}

#[test]
fn fix_writes_gemini_cli_servers_next_to_its_settings() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{
  "mcpServers": {
    "docs": { "type": "http", "url": "https://docs.example.com/mcp", "headers": { "Authorization": "Bearer ${DOCS_KEY}" } },
    "linear": { "type": "sse", "url": "https://mcp.linear.app/sse" },
    "files": { "command": "npx", "args": ["-y", "files-mcp", "${ROOT}"], "env": { "CI": "1" } }
  }
}
"#,
    );
    dir.write(".gemini/settings.json", "{\n  \"theme\": \"GitHub\"\n}\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    assert_eq!(
        read(&dir, ".gemini/settings.json"),
        r#"{
  "theme": "GitHub",
  "mcpServers": {
    "docs": {
      "httpUrl": "https://docs.example.com/mcp",
      "headers": {
        "Authorization": "Bearer ${DOCS_KEY}"
      }
    },
    "linear": {
      "url": "https://mcp.linear.app/sse"
    },
    "files": {
      "command": "npx",
      "args": ["-y", "files-mcp", "${ROOT}"],
      "env": {
        "CI": "1"
      }
    }
  }
}
"#
    );
}

#[test]
fn gemini_cli_gets_no_text_it_would_read_as_an_environment_variable() {
    // Claude Code leaves `$HOME` as it is; Gemini CLI would fill it in.
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "files": { "command": "npx", "args": ["files-mcp", "$HOME"] } } }"#,
    );
    dir.write(".gemini/settings.json", "{}\n");

    let (report, _) = fix(&dir);

    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::GeminiCli,
            server: "files".to_string(),
            reason: "the command line has `$HOME`, which can't be written for Gemini CLI".to_string(),
        }]
    );
}

// ---------------------------------------------------------------------------
// Qwen Code
// ---------------------------------------------------------------------------

#[test]
fn reads_qwen_code_servers() {
    let dir = TempDir::new();
    dir.write(
        ".qwen/settings.json",
        r#"{
  "mcpServers": {
    "myDatabaseServer": { "command": "/path/to/db-server", "args": ["--port", "5432"], "env": { "DATABASE_URL": "$DB_CONNECTION_STRING" } },
    "httpServer": { "httpUrl": "http://localhost:3000/mcp" }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::QwenCode);

    assert_eq!(config.relative_path, Path::new(".qwen/settings.json"));
    let database = server(&config, "myDatabaseServer");
    assert_eq!(database.transport, command("/path/to/db-server", &["--port", "5432"]));
    assert_eq!(database.env, [setting("DATABASE_URL", &[env("DB_CONNECTION_STRING")])]);
    assert_eq!(
        server(&config, "httpServer").transport,
        remote("http://localhost:3000/mcp")
    );
}

#[test]
fn fix_creates_a_qwen_code_config() {
    assert_eq!(
        created_from_claude_code(".qwen/commands/review.md", ".qwen/settings.json"),
        r#"{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": ["-y", "@playwright/mcp@0.0.78"],
      "env": {
        "CI": "1"
      }
    },
    "docs": {
      "httpUrl": "https://docs.example.com/mcp",
      "headers": {
        "X-Team": "web"
      }
    }
  }
}
"#
    );
}

// ---------------------------------------------------------------------------
// Antigravity
// ---------------------------------------------------------------------------

#[test]
fn reads_antigravity_servers() {
    let dir = TempDir::new();
    dir.write(
        ".agents/mcp_config.json",
        r#"{
  "mcpServers": {
    "sqlite": { "command": "uvx", "args": ["mcp-server-sqlite"], "env": { "SQLITE_DB_PATH": "/var/data/app.db" } },
    "my-remote-server": {
      "serverUrl": "https://api.example.com/mcp/",
      "headers": { "Authorization": "Bearer YOUR_API_TOKEN" },
      "timeoutSeconds": 600
    },
    "older": { "url": "https://older.example.com/mcp", "disabled": true }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::Antigravity);

    assert_eq!(config.relative_path, Path::new(".agents/mcp_config.json"));
    assert_eq!(
        server(&config, "sqlite").transport,
        command("uvx", &["mcp-server-sqlite"])
    );
    assert_eq!(
        server(&config, "my-remote-server").transport,
        remote("https://api.example.com/mcp/")
    );
    let older = server(&config, "older");
    assert_eq!(older.transport, remote("https://older.example.com/mcp"));
    assert!(!older.enabled);
}

#[test]
fn an_agents_directory_alone_is_not_antigravity() {
    // `.agents/skills` is where many agents look for skills.
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".agents/skills/review/SKILL.md", "Be brief.\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.changed.is_empty(), "{:?}", report.changed);
}

#[test]
fn fix_writes_antigravity_servers_with_server_url() {
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".agents/mcp_config.json", "{}\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    assert_eq!(
        read(&dir, ".agents/mcp_config.json"),
        CLAUDE_CODE_AS_URL.replace("\"url\"", "\"serverUrl\"")
    );
}

#[test]
fn antigravity_gets_no_server_reached_over_sse() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "linear": { "type": "sse", "url": "https://mcp.linear.app/sse" } } }"#,
    );
    dir.write(".agents/mcp_config.json", "{}\n");

    let (report, _) = fix(&dir);

    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::Antigravity,
            server: "linear".to_string(),
            reason: "it's reached over SSE, and Antigravity only reaches remote servers over streamable HTTP"
                .to_string(),
        }]
    );
}

// ---------------------------------------------------------------------------
// GitHub Copilot CLI
// ---------------------------------------------------------------------------

#[test]
fn reads_copilot_cli_servers() {
    let dir = TempDir::new();
    dir.write(
        ".github/mcp.json",
        r#"{
  "mcpServers": {
    "playwright": { "type": "local", "command": "npx", "args": ["@playwright/mcp@latest"], "env": {}, "tools": ["*"] },
    "context7": {
      "type": "http",
      "url": "https://mcp.context7.com/mcp",
      "headers": { "CONTEXT7_API_KEY": "YOUR-API-KEY" },
      "tools": ["*"]
    }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::CopilotCli);

    assert_eq!(config.relative_path, Path::new(".github/mcp.json"));
    assert_eq!(
        server(&config, "playwright").transport,
        command("npx", &["@playwright/mcp@latest"])
    );
    assert_eq!(
        server(&config, "context7").transport,
        remote("https://mcp.context7.com/mcp")
    );
}

#[test]
fn a_github_directory_alone_is_not_copilot_cli() {
    let dir = TempDir::new();
    dir.write(".cursor/mcp.json", CURSOR);
    dir.write(".github/workflows/ci.yml", "on: push\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.changed.is_empty(), "{:?}", report.changed);
}

#[test]
fn fix_gives_every_copilot_cli_server_a_type() {
    let dir = TempDir::new();
    dir.write(".cursor/mcp.json", CURSOR);
    dir.write(".github/mcp.json", "{}\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    assert_eq!(
        read(&dir, ".github/mcp.json"),
        CLAUDE_CODE_AS_TYPED_URL.replace(
            "\"command\": \"npx\"",
            "\"type\": \"local\",\n      \"command\": \"npx\""
        )
    );
}

#[test]
fn copilot_names_copilot_cli() {
    assert_eq!(Harness::parse("copilot"), Some(Harness::CopilotCli));
    assert_eq!(Harness::parse("copilot-cli"), Some(Harness::CopilotCli));
}

// ---------------------------------------------------------------------------
// Agents that also read Claude Code's `.mcp.json`: Copilot CLI and Qoder
// ---------------------------------------------------------------------------

#[test]
fn copilot_cli_is_not_compared_while_it_reads_mcp_json() {
    // It reads `.mcp.json` then, so it gets the same servers as Claude Code.
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".github/mcp.json", "{}\n");

    let configs = read_mcp(dir.path());
    let check = check_mcp(dir.path(), &configs);

    assert_eq!(check.harnesses, [Harness::ClaudeCode]);
    assert_eq!(check.findings().count(), 0, "{:?}", findings(&dir));
    assert!(
        configs
            .iter()
            .any(|config| config.harness == Harness::CopilotCli && config.ignored)
    );
    assert!(fix(&dir).0.changed.is_empty());
}

#[test]
fn a_server_missing_from_mcp_json_is_added_there_for_copilot_cli_too() {
    // Copilot CLI reads `.mcp.json` instead of `.github/mcp.json`, so it isn't compared.
    let dir = TempDir::new();
    dir.write(".cursor/mcp.json", CURSOR);
    dir.write(".mcp.json", "{ \"mcpServers\": {} }\n");
    dir.write(".github/mcp.json", "{}\n");

    let before = compare_mcp(dir.path(), &read_mcp(dir.path()));
    let (report, left) = fix(&dir);

    assert_eq!(
        before[0].message,
        "`playwright` is set up for Cursor but not for Claude Code (.mcp.json)"
    );
    assert_eq!(
        before[0].fix.as_deref(),
        Some("Add it to .mcp.json, as set up in .cursor/mcp.json:3")
    );
    assert!(left.is_empty(), "{left:?}");
    let changed: Vec<String> = report
        .changed
        .iter()
        .map(|changed| changed.relative_path.display().to_string())
        .collect();
    assert_eq!(changed, [".mcp.json"]);
    assert_eq!(read(&dir, ".github/mcp.json"), "{}\n");
}

#[test]
fn qoder_needs_no_config_file_of_its_own_beside_mcp_json() {
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".cursor/mcp.json", "{ \"mcpServers\": {} }\n");
    dir.write(".qoder/rules/style.md", "Be brief.\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    let changed: Vec<String> = report
        .changed
        .iter()
        .map(|changed| changed.relative_path.display().to_string())
        .collect();
    assert_eq!(changed, [".cursor/mcp.json"]);
    assert!(!dir.path().join(".qoder/settings.json").exists());
}

#[test]
fn copilot_cli_ignores_its_own_file_while_mcp_json_exists() {
    // Copilot CLI 1.x reads `.mcp.json` instead of `.github/mcp.json` whenever `.mcp.json` is
    // there, even with no servers in it.
    let dir = TempDir::new();
    dir.write(".mcp.json", "{ \"mcpServers\": {} }\n");
    dir.write(
        ".github/mcp.json",
        r#"{ "mcpServers": { "context7": { "type": "http", "url": "https://mcp.context7.com/mcp" } } }"#,
    );

    let configs = read_mcp(dir.path());
    let copilot = configs
        .iter()
        .find(|config| config.harness == Harness::CopilotCli)
        .unwrap();
    let (report, left) = fix(&dir);

    assert!(copilot.servers.is_empty(), "{:?}", copilot.servers);
    assert_eq!(
        copilot.warnings,
        ["Copilot CLI doesn't read this file while .mcp.json exists; it reads .mcp.json instead"]
    );
    assert!(left.is_empty(), "{left:?}");
    assert!(report.changed.is_empty(), "{:?}", report.changed);
}

#[test]
fn reads_copilot_cli_servers_listed_without_mcp_servers() {
    let dir = TempDir::new();
    dir.write(
        ".github/mcp.json",
        r#"{ "playwright": { "type": "local", "command": "npx", "args": ["@playwright/mcp@latest"] } }"#,
    );

    let config = config_of(&dir, Harness::CopilotCli);

    assert_eq!(
        server(&config, "playwright").transport,
        command("npx", &["@playwright/mcp@latest"])
    );
}

#[test]
fn fix_adds_to_a_copilot_cli_file_without_mcp_servers_at_its_top() {
    let dir = TempDir::new();
    dir.write(
        ".cursor/mcp.json",
        r#"{ "mcpServers": { "docs": { "url": "https://docs.example.com/mcp" } } }"#,
    );
    dir.write(
        ".github/mcp.json",
        "{\n  \"docs-old\": { \"type\": \"local\", \"command\": \"uvx\", \"args\": [\"docs-mcp\"] }\n}\n",
    );

    let (report, _) = fix(&dir);

    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    assert_eq!(
        read(&dir, ".github/mcp.json"),
        "{\n  \"docs-old\": { \"type\": \"local\", \"command\": \"uvx\", \"args\": [\"docs-mcp\"] },\n  \
         \"docs\": {\n    \"type\": \"http\",\n    \"url\": \"https://docs.example.com/mcp\"\n  }\n}\n"
    );
}

#[test]
fn qoder_gets_values_and_command_lines_that_read_the_environment() {
    // Qoder CLI 1.1.89 fills in `$VAR` and `${VAR}` in `env`, `args` and `headers`, though its
    // docs don't say so.
    let dir = TempDir::new();
    dir.write(
        ".cursor/mcp.json",
        r#"{ "mcpServers": {
  "search": { "command": "uvx", "args": ["search-mcp", "--root", "${env:ROOT}"], "env": { "TOKEN": "${env:TOKEN}" } },
  "docs": { "url": "https://docs.example.com/mcp", "headers": { "Authorization": "Bearer ${env:DOCS_KEY}" } }
} }"#,
    );
    dir.write(".qoder/settings.json", "{}\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.unfixed.is_empty(), "{:?}", report.unfixed);
    let settings = read(&dir, ".qoder/settings.json");
    for written in [
        r#""args": ["search-mcp", "--root", "${ROOT}"]"#,
        r#""TOKEN": "${TOKEN}""#,
        r#""Authorization": "Bearer ${DOCS_KEY}""#,
    ] {
        assert!(settings.contains(written), "{written} not in {settings}");
    }
}

#[test]
fn reads_the_environment_variables_a_qoder_server_takes() {
    let dir = TempDir::new();
    dir.write(
        ".qoder/settings.json",
        r#"{ "mcpServers": { "db": { "command": "db-mcp", "env": { "URL": "$DB_URL", "KEY": "${DB_KEY}" } } } }"#,
    );

    let config = config_of(&dir, Harness::Qoder);

    assert_eq!(
        server(&config, "db").env,
        [setting("KEY", &[env("DB_KEY")]), setting("URL", &[env("DB_URL")])]
    );
}

#[test]
fn fix_creates_no_mcp_json_that_would_lose_a_copilot_cli_server() {
    // Once `.mcp.json` exists, Copilot CLI reads it instead of `.github/mcp.json`, so it has to
    // get every server Copilot has, or Copilot loses one.
    let dir = TempDir::new();
    dir.write(".claude/settings.json", "{}\n");
    let github = r#"{ "mcpServers": {
  "sentry": { "type": "local", "command": "npx", "args": ["-y", "@sentry/mcp-server"], "env": { "SENTRY_ACCESS_TOKEN": "${SENTRY_ACCESS_TOKEN}" } },
  "docs": { "type": "local", "command": "uvx", "args": ["docs-mcp"] }
} }"#;
    dir.write(".github/mcp.json", github);

    let (report, left) = fix(&dir);

    assert!(!dir.path().join(".mcp.json").exists());
    assert_eq!(read(&dir, ".github/mcp.json"), github);
    assert!(report.changed.is_empty(), "{:?}", report.changed);
    let reasons: Vec<(&str, &str)> = report
        .unfixed
        .iter()
        .map(|unfixed| (unfixed.server.as_str(), unfixed.reason.as_str()))
        .collect();
    assert_eq!(
        reasons,
        [
            (
                "sentry",
                "the value of `SENTRY_ACCESS_TOKEN` can't be written for Claude Code"
            ),
            (
                "docs",
                "creating .mcp.json would make Copilot CLI read it instead of .github/mcp.json, without `sentry`"
            ),
        ]
    );
    assert_eq!(left.len(), 1, "{left:?}");
}

#[test]
fn fix_creates_mcp_json_with_every_server_copilot_cli_had() {
    let dir = TempDir::new();
    dir.write(".claude/settings.json", "{}\n");
    dir.write(
        ".github/mcp.json",
        r#"{ "mcpServers": { "docs": { "type": "local", "command": "uvx", "args": ["docs-mcp"] } } }"#,
    );

    let (report, left) = fix(&dir);
    let configs = read_mcp(dir.path());
    let copilot = configs
        .iter()
        .find(|config| config.harness == Harness::CopilotCli)
        .unwrap();

    assert!(left.is_empty(), "{left:?}");
    assert_eq!(report.changed.len(), 1);
    assert!(read(&dir, ".mcp.json").contains("\"docs\""));
    // Copilot CLI reads `.mcp.json` now, and gets the server there.
    assert!(copilot.ignored);
    assert_eq!(copilot.warnings.len(), 1, "{:?}", copilot.warnings);
}

#[test]
fn fix_names_the_text_it_cannot_write_for_an_agent() {
    // Gemini CLI fills in `${DB_USER}`, but not a default such as `${DB_HOST:-localhost}`.
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "db": { "command": "sh", "args": ["-c", "psql postgresql://${DB_USER}@${DB_HOST:-localhost}/app"] } } }"#,
    );
    dir.write(".gemini/settings.json", "{}\n");

    let (report, _) = fix(&dir);

    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::GeminiCli,
            server: "db".to_string(),
            reason: "the command line has `${DB_HOST:-localhost}`, which can't be written for Gemini CLI".to_string(),
        }]
    );
}

#[test]
fn a_server_without_a_url_antigravity_reads_names_the_keys_it_reads() {
    // Antigravity doesn't read Gemini CLI's `httpUrl`.
    let dir = TempDir::new();
    dir.write(
        ".agents/mcp_config.json",
        r#"{ "mcpServers": { "context7": { "httpUrl": "https://mcp.context7.com/mcp" } } }"#,
    );

    let configs = read_mcp(dir.path());

    assert_eq!(
        configs[0].warnings,
        ["`context7` has neither a `command` nor a `serverUrl` or `url`"]
    );
}

#[test]
fn copilot_cli_skips_a_file_without_mcp_servers_and_fix_leaves_it() {
    // Copilot CLI 1.x rejects such a file: "mcpServers: Required". Some repositories have VS
    // Code's `servers` here, or a package manifest.
    for github in [
        r#"{ "servers": { "docs": { "type": "stdio", "command": "uvx", "args": ["docs-mcp"] } } }"#,
        r#"{ "name": "pdf-reader-mcp", "version": "0.1.0", "categories": ["mcp"] }"#,
    ] {
        let dir = TempDir::new();
        dir.write(".cursor/mcp.json", CURSOR);
        dir.write(".github/mcp.json", github);

        let configs = read_mcp(dir.path());
        let copilot = configs
            .iter()
            .find(|config| config.harness == Harness::CopilotCli)
            .unwrap();
        let (report, left) = fix(&dir);

        assert!(copilot.ignored);
        assert_eq!(
            copilot.warnings,
            ["Copilot CLI skips this file: it lists no servers under `mcpServers` or at its top"]
        );
        assert!(left.is_empty(), "{left:?}");
        assert!(report.changed.is_empty(), "{:?}", report.changed);
        assert_eq!(read(&dir, ".github/mcp.json"), github);
    }
}

#[test]
fn a_server_gemini_cli_excludes_or_does_not_allow_is_turned_off() {
    // Gemini CLI loads only the servers in `mcp.allowed`, when there is one, even an empty
    // one, and never those in `mcp.excluded`.
    let servers = r#""a": { "command": "a-mcp" }, "b": { "command": "b-mcp" }, "c": { "command": "c-mcp" }"#;
    for (lists, on) in [
        (r#"{ "excluded": ["b"] }"#, vec!["a", "c"]),
        (r#"{ "allowed": ["a", "b"], "excluded": ["b"] }"#, vec!["a"]),
        (r#"{ "allowed": [] }"#, vec![]),
        (r#"{ "serverCommand": "x" }"#, vec!["a", "b", "c"]),
    ] {
        let dir = TempDir::new();
        dir.write(
            ".gemini/settings.json",
            format!("{{ \"mcp\": {lists}, \"mcpServers\": {{ {servers} }} }}"),
        );

        let config = config_of(&dir, Harness::GeminiCli);

        let enabled: Vec<&str> = config
            .servers
            .iter()
            .filter(|server| server.enabled)
            .map(|server| server.name.as_str())
            .collect();
        assert_eq!(enabled, on, "{lists}");
    }
}

// ---------------------------------------------------------------------------
// Amp
// ---------------------------------------------------------------------------

#[test]
fn reads_amp_servers() {
    let dir = TempDir::new();
    dir.write(
        ".amp/settings.json",
        r#"{
  "amp.dangerouslyAllowAll": false,
  "amp.mcpServers": {
    "playwright": { "command": "npx", "args": ["-y", "@playwright/mcp@latest", "--headless"] },
    "sourcegraph": {
      "url": "${SRC_ENDPOINT}/.api/mcp/v1",
      "headers": { "Authorization": "token ${SRC_ACCESS_TOKEN}" }
    }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::Amp);

    assert_eq!(config.relative_path, Path::new(".amp/settings.json"));
    assert_eq!(
        server(&config, "playwright").transport,
        command("npx", &["-y", "@playwright/mcp@latest", "--headless"])
    );
    let sourcegraph = server(&config, "sourcegraph");
    assert_eq!(sourcegraph.transport, remote("${SRC_ENDPOINT}/.api/mcp/v1"));
    assert_eq!(
        sourcegraph.headers,
        [setting("Authorization", &[text("token "), env("SRC_ACCESS_TOKEN")])]
    );
}

#[test]
fn fix_creates_an_amp_config() {
    assert_eq!(
        created_from_claude_code(".amp/notes.md", ".amp/settings.json"),
        CLAUDE_CODE_AS_URL.replace("\"mcpServers\"", "\"amp.mcpServers\"")
    );
}

#[test]
fn amp_gets_values_and_command_lines_that_read_the_environment() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "search": { "command": "uvx", "args": ["search-mcp", "${ROOT}"], "env": { "TOKEN": "${TOKEN}" } } } }"#,
    );
    dir.write(".amp/settings.json", "{}\n");

    let (report, left) = fix(&dir);

    assert!(
        left.is_empty() && report.unfixed.is_empty(),
        "{left:?} {:?}",
        report.unfixed
    );
    let written = read(&dir, ".amp/settings.json");
    assert!(written.contains(r#""args": ["search-mcp", "${ROOT}"]"#), "{written}");
    assert!(written.contains(r#""TOKEN": "${TOKEN}""#), "{written}");
}

// ---------------------------------------------------------------------------
// Crush
// ---------------------------------------------------------------------------

#[test]
fn reads_crush_servers() {
    let dir = TempDir::new();
    dir.write(
        ".crush.json",
        r#"{
  "$schema": "https://charm.land/crush.json",
  "mcp": {
    "filesystem": {
      "type": "stdio",
      "command": "node",
      "args": ["/srv/mcp-server.js"],
      "env": { "NODE_ENV": "production", "TOKEN": "$FS_TOKEN" }
    },
    "github": {
      "type": "http",
      "url": "https://api.githubcopilot.com/mcp/",
      "headers": { "Authorization": "Bearer ${GH_PAT}" }
    },
    "old": { "command": "old-mcp", "disabled": true }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::Crush);

    let filesystem = server(&config, "filesystem");
    assert_eq!(filesystem.transport, command("node", &["/srv/mcp-server.js"]));
    assert_eq!(
        filesystem.env,
        [
            setting("NODE_ENV", &[text("production")]),
            setting("TOKEN", &[env("FS_TOKEN")]),
        ]
    );
    let github = server(&config, "github");
    assert_eq!(github.transport, remote("https://api.githubcopilot.com/mcp/"));
    assert_eq!(
        github.headers,
        [setting("Authorization", &[text("Bearer "), env("GH_PAT")])]
    );
    assert!(!server(&config, "old").enabled);
}

#[test]
fn fix_writes_crush_servers_next_to_its_settings_but_never_creates_its_file() {
    // Crush's JSON is its older format: a repository starting now uses `crushrc` instead.
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".crush/logs/crush.log", "\n");
    assert!(fix(&dir).0.changed.is_empty());

    dir.write("crush.json", "{\n  \"options\": { \"debug\": false }\n}\n");
    let (report, left) = fix(&dir);

    assert!(
        left.is_empty() && report.unfixed.is_empty(),
        "{left:?} {:?}",
        report.unfixed
    );
    assert_eq!(
        read(&dir, "crush.json"),
        r#"{
  "options": { "debug": false },
  "mcp": {
    "playwright": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@playwright/mcp@0.0.78"],
      "env": {
        "CI": "1"
      }
    },
    "docs": {
      "type": "http",
      "url": "https://docs.example.com/mcp",
      "headers": {
        "X-Team": "web"
      }
    }
  }
}
"#
    );
}

#[test]
fn crush_gets_no_text_its_shell_would_expand_or_run() {
    // Claude Code leaves these as they are; Crush runs every value through the shell.
    for (arg, shown) in [("$HOME", "$HOME"), ("$(whoami)", "$(")] {
        let dir = TempDir::new();
        dir.write(
            ".mcp.json",
            format!(r#"{{ "mcpServers": {{ "files": {{ "command": "npx", "args": ["files-mcp", "{arg}"] }} }} }}"#),
        );
        dir.write(".crush.json", "{}\n");

        let (report, _) = fix(&dir);

        assert_eq!(
            report.unfixed,
            [Unfixed {
                harness: Harness::Crush,
                server: "files".to_string(),
                reason: format!("the command line has `{shown}`, which can't be written for Crush"),
            }],
            "{arg}"
        );
    }
}

#[test]
fn crush_says_it_reads_crushrc_too() {
    let dir = TempDir::new();
    dir.write(".crush.json", "{ \"mcp\": {} }\n");
    dir.write(
        ".crushrc",
        "mcp add github --type http --url https://api.githubcopilot.com/mcp/\n",
    );

    let configs = read_mcp(dir.path());

    let crush = configs.iter().find(|config| config.harness == Harness::Crush).unwrap();
    assert_eq!(
        crush.warnings,
        ["Crush also reads servers from .crushrc, a shell script gglint doesn't read"]
    );
}

// ---------------------------------------------------------------------------
// Kilo Code
// ---------------------------------------------------------------------------

#[test]
fn reads_kilo_code_servers() {
    let dir = TempDir::new();
    dir.write(
        "kilo.jsonc",
        r#"{
  // Servers for this project.
  "mcp": {
    "my-local-server": {
      "type": "local",
      "command": ["node", "/path/to/server.js"],
      "environment": { "LOG_LEVEL": "debug" },
      "enabled": true,
      "timeout": 10000
    },
    "my-remote-server": {
      "type": "remote",
      "url": "https://your-server-url.com/mcp",
      "headers": { "X-Team": "web" },
      "enabled": false
    }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::KiloCode);

    let local = server(&config, "my-local-server");
    assert_eq!(local.transport, command("node", &["/path/to/server.js"]));
    assert_eq!(local.env, [setting("LOG_LEVEL", &[text("debug")])]);
    let remote_server = server(&config, "my-remote-server");
    assert_eq!(remote_server.transport, remote("https://your-server-url.com/mcp"));
    assert_eq!(remote_server.headers, [setting("X-Team", &[text("web")])]);
    assert!(!remote_server.enabled);
}

#[test]
fn kilo_code_gets_no_server_that_reads_the_environment() {
    // Kilo Code skips a repository's whole config file when it has `{env:VAR}`.
    assert_eq!(
        unfixed_env_reference(".kilo/agents/review.md"),
        [Unfixed {
            harness: Harness::KiloCode,
            server: "search".to_string(),
            reason: "the value of `TOKEN` can't be written for Kilo Code".to_string(),
        }]
    );
}

#[test]
fn fix_creates_a_kilo_code_config_with_servers_right_under_mcp() {
    assert_eq!(
        created_from_claude_code(".kilo/agents/review.md", ".kilo/kilo.json"),
        r#"{
  "mcp": {
    "playwright": {
      "type": "local",
      "command": ["npx", "-y", "@playwright/mcp@0.0.78"],
      "environment": {
        "CI": "1"
      }
    },
    "docs": {
      "type": "remote",
      "url": "https://docs.example.com/mcp",
      "headers": {
        "X-Team": "web"
      }
    }
  }
}
"#
    );
}

#[test]
fn kilo_names_kilo_code() {
    assert_eq!(Harness::parse("kilo"), Some(Harness::KiloCode));
    assert_eq!(Harness::parse("kilo-code"), Some(Harness::KiloCode));
}

// ---------------------------------------------------------------------------
// VS Code
// ---------------------------------------------------------------------------

#[test]
fn reads_vs_code_servers() {
    let dir = TempDir::new();
    dir.write(
        ".vscode/mcp.json",
        r#"{
  "inputs": [{ "type": "promptString", "id": "github-token", "description": "GitHub token", "password": true }],
  "servers": {
    "github": {
      "type": "http",
      "url": "https://api.githubcopilot.com/mcp/",
      "headers": { "Authorization": "Bearer ${input:github-token}" }
    },
    "files": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "files-mcp", "${workspaceFolder}"],
      "env": { "TOKEN": "${env:FILES_TOKEN}" }
    }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::VsCode);

    let files = server(&config, "files");
    assert_eq!(
        files.transport,
        command("npx", &["-y", "files-mcp", "${workspaceFolder}"])
    );
    assert_eq!(files.env, [setting("TOKEN", &[env("FILES_TOKEN")])]);
    assert_eq!(
        server(&config, "github").headers,
        [setting("Authorization", &[text("Bearer ${input:github-token}")])]
    );
}

#[test]
fn a_vscode_directory_alone_is_not_vs_code() {
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".vscode/settings.json", "{ \"editor.tabSize\": 2 }\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.changed.is_empty(), "{:?}", report.changed);
}

#[test]
fn fix_gives_every_vs_code_server_a_type() {
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".vscode/mcp.json", "{\n  \"servers\": {}\n}\n");

    let (report, left) = fix(&dir);

    assert!(
        left.is_empty() && report.unfixed.is_empty(),
        "{left:?} {:?}",
        report.unfixed
    );
    assert_eq!(
        read(&dir, ".vscode/mcp.json"),
        r#"{
  "servers": {
    "playwright": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@playwright/mcp@0.0.78"],
      "env": {
        "CI": "1"
      }
    },
    "docs": {
      "type": "http",
      "url": "https://docs.example.com/mcp",
      "headers": {
        "X-Team": "web"
      }
    }
  }
}
"#
    );
}

#[test]
fn a_vs_code_server_that_asks_for_an_input_isnt_copied() {
    let dir = TempDir::new();
    dir.write(".mcp.json", "{ \"mcpServers\": {} }\n");
    dir.write(
        ".vscode/mcp.json",
        r#"{ "servers": { "github": { "type": "http", "url": "https://api.githubcopilot.com/mcp/", "headers": { "Authorization": "Bearer ${input:github-token}" } } } }"#,
    );

    let (report, _) = fix(&dir);

    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::ClaudeCode,
            server: "github".to_string(),
            reason: "the value of the `Authorization` header can't be written for Claude Code".to_string(),
        }]
    );
}

// ---------------------------------------------------------------------------
// Zed
// ---------------------------------------------------------------------------

#[test]
fn reads_zed_servers() {
    let dir = TempDir::new();
    dir.write(
        ".zed/settings.json",
        r#"// Zed settings for this project
{
  "tab_size": 2,
  "context_servers": {
    "local-mcp-server": { "command": "some-command", "args": ["arg-1", "arg-2"], "env": { "MODE": "dev" } },
    "remote-mcp-server": { "url": "https://example.com/mcp", "headers": { "X-Team": "web" } },
    "off": { "command": "off-mcp", "enabled": false }
  }
}
"#,
    );

    let config = config_of(&dir, Harness::Zed);

    let local = server(&config, "local-mcp-server");
    assert_eq!(local.transport, command("some-command", &["arg-1", "arg-2"]));
    assert_eq!(local.env, [setting("MODE", &[text("dev")])]);
    assert_eq!(
        server(&config, "remote-mcp-server").transport,
        remote("https://example.com/mcp")
    );
    assert!(!server(&config, "off").enabled);
}

#[test]
fn fix_writes_zed_servers_next_to_its_settings() {
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".zed/settings.json", "{\n  \"tab_size\": 2\n}\n");

    let (report, left) = fix(&dir);

    assert!(
        left.is_empty() && report.unfixed.is_empty(),
        "{left:?} {:?}",
        report.unfixed
    );
    assert_eq!(
        read(&dir, ".zed/settings.json"),
        CLAUDE_CODE_AS_URL.replace("{\n  \"mcpServers\"", "{\n  \"tab_size\": 2,\n  \"context_servers\"")
    );
}

#[test]
fn a_zed_directory_alone_is_not_zed() {
    let dir = TempDir::new();
    dir.write(".mcp.json", CLAUDE_CODE);
    dir.write(".zed/tasks.json", "[]\n");

    let (report, left) = fix(&dir);

    assert!(left.is_empty(), "{left:?}");
    assert!(report.changed.is_empty(), "{:?}", report.changed);
}

#[test]
fn zed_gets_no_server_that_reads_the_environment() {
    assert_eq!(
        unfixed_env_reference(".zed/settings.json"),
        [Unfixed {
            harness: Harness::Zed,
            server: "search".to_string(),
            reason: "the value of `TOKEN` can't be written for Zed".to_string(),
        }]
    );
}

#[test]
fn a_zed_server_from_an_extension_is_named_not_compared() {
    let dir = TempDir::new();
    dir.write(
        ".zed/settings.json",
        r#"{ "context_servers": { "postgres": { "settings": { "database_url": "postgresql://localhost/app" } } } }"#,
    );

    let configs = read_mcp(dir.path());

    let zed = configs.iter().find(|config| config.harness == Harness::Zed).unwrap();
    assert!(zed.servers.is_empty());
    assert_eq!(
        zed.warnings,
        ["`postgres` is started by a Zed extension, so gglint can't compare it"]
    );
}

#[test]
fn reads_and_updates_zed_servers_in_its_older_layout() {
    // Zed used to nest the command: `"command": { "path", "args", "env" }`.
    let dir = TempDir::new();
    dir.write(
        ".zed/settings.json",
        r#"{
  "context_servers": {
    "docs": { "command": { "path": "npx", "args": ["-y", "docs-mcp@1"], "env": { "MODE": "dev" } } }
  }
}
"#,
    );
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "docs": { "command": "npx", "args": ["-y", "docs-mcp@2"], "env": { "MODE": "dev" } } } }"#,
    );

    let config = config_of(&dir, Harness::Zed);
    let docs = server(&config, "docs");
    assert_eq!(docs.transport, command("npx", &["-y", "docs-mcp@1"]));
    assert_eq!(docs.env, [setting("MODE", &[text("dev")])]);

    let (report, left) = fix(&dir);

    assert!(
        left.is_empty() && report.unfixed.is_empty(),
        "{left:?} {:?}",
        report.unfixed
    );
    assert_eq!(
        read(&dir, ".zed/settings.json"),
        r#"{
  "context_servers": {
    "docs": { "command": { "path": "npx", "args": ["-y", "docs-mcp@2"], "env": { "MODE": "dev" } } }
  }
}
"#
    );
}

#[test]
fn a_command_that_isnt_a_string_is_named() {
    let dir = TempDir::new();
    dir.write(
        "crush.json",
        r#"{ "mcp": { "files": { "type": "stdio", "command": ["npx", "-y", "files-mcp"] } } }"#,
    );

    let configs = read_mcp(dir.path());

    let crush = configs.iter().find(|config| config.harness == Harness::Crush).unwrap();
    assert_eq!(crush.warnings, ["`files` has a `command` that isn't a string"]);
}

// ---------------------------------------------------------------------------
// Mistral Vibe
// ---------------------------------------------------------------------------

#[test]
fn reads_mistral_vibe_servers() {
    let dir = TempDir::new();
    dir.write(
        ".vibe/config.toml",
        r#"active_model = "devstral"

[[mcp_servers]]
name = "fetch_server"
transport = "stdio"
command = "uvx"
args = ["mcp-server-fetch"]
env = { DEBUG = "1" }

[[mcp_servers]]
name = "docs"
transport = "streamable-http"
url = "https://docs.example.com/mcp"
headers = { "X-Team" = "web" }
api_key_env = "DOCS_TOKEN"

[[mcp_servers]]
name = "linear"
transport = "http"
url = "https://mcp.linear.app/mcp"

[mcp_servers.auth]
type = "static"
headers = { X-Org = "corridor" }
api_key_env = "LINEAR_KEY"
api_key_header = "X-API-Key"
api_key_format = "{token}"

[[mcp_servers]]
name = "files"
transport = "stdio"
command = "npx -y 'files mcp'"
disabled = true
"#,
    );

    let config = config_of(&dir, Harness::MistralVibe);

    let fetch = server(&config, "fetch_server");
    assert_eq!(fetch.transport, command("uvx", &["mcp-server-fetch"]));
    assert_eq!(fetch.env, [setting("DEBUG", &[text("1")])]);
    assert_eq!(fetch.location.line, 4);
    let docs = server(&config, "docs");
    assert_eq!(docs.transport, remote("https://docs.example.com/mcp"));
    assert_eq!(
        docs.headers,
        [
            setting("Authorization", &[text("Bearer "), env("DOCS_TOKEN")]),
            setting("X-Team", &[text("web")]),
        ]
    );
    assert_eq!(
        server(&config, "linear").headers,
        [
            setting("X-API-Key", &[env("LINEAR_KEY")]),
            setting("X-Org", &[text("corridor")])
        ]
    );
    let files = server(&config, "files");
    assert_eq!(files.transport, command("npx", &["-y", "files mcp"]));
    assert!(!files.enabled);
}

#[test]
fn fix_creates_a_mistral_vibe_config() {
    assert_eq!(
        created_from_claude_code(".vibe/agents/review.toml", ".vibe/config.toml"),
        r#"[[mcp_servers]]
name = "playwright"
transport = "stdio"
command = "npx"
args = ["-y", "@playwright/mcp@0.0.78"]
env = { CI = "1" }

[[mcp_servers]]
name = "docs"
transport = "streamable-http"
url = "https://docs.example.com/mcp"
headers = { X-Team = "web" }
"#
    );
}

#[test]
fn fix_adds_mistral_vibe_servers_after_its_settings() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "docs": { "type": "http", "url": "https://docs.example.com/mcp", "headers": { "Authorization": "Bearer ${DOCS_TOKEN}" } } } }"#,
    );
    dir.write(".vibe/config.toml", "# Vibe\nactive_model = \"devstral\"\n");

    let (report, left) = fix(&dir);

    assert!(
        left.is_empty() && report.unfixed.is_empty(),
        "{left:?} {:?}",
        report.unfixed
    );
    assert_eq!(
        read(&dir, ".vibe/config.toml"),
        r#"# Vibe
active_model = "devstral"

[[mcp_servers]]
name = "docs"
transport = "streamable-http"
url = "https://docs.example.com/mcp"
api_key_env = "DOCS_TOKEN"
"#
    );
}

#[test]
fn mistral_vibe_gets_no_server_that_reads_the_environment_in_another_way() {
    assert_eq!(
        unfixed_env_reference(".vibe/agents/review.toml"),
        [Unfixed {
            harness: Harness::MistralVibe,
            server: "search".to_string(),
            reason: "the value of `TOKEN` can't be written for Mistral Vibe".to_string(),
        }]
    );
}

#[test]
fn mistral_vibe_gets_no_server_reached_over_sse() {
    let dir = TempDir::new();
    dir.write(
        ".mcp.json",
        r#"{ "mcpServers": { "linear": { "type": "sse", "url": "https://mcp.linear.app/sse" } } }"#,
    );
    dir.write(".vibe/config.toml", "");

    let (report, _) = fix(&dir);

    assert_eq!(
        report.unfixed,
        [Unfixed {
            harness: Harness::MistralVibe,
            server: "linear".to_string(),
            reason: "it's reached over SSE, and Mistral Vibe only reaches remote servers over streamable HTTP"
                .to_string(),
        }]
    );
}

#[test]
fn vibe_names_mistral_vibe() {
    assert_eq!(Harness::parse("vibe"), Some(Harness::MistralVibe));
    assert_eq!(Harness::parse("mistral-vibe"), Some(Harness::MistralVibe));
}
