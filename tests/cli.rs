//! The `gglint` command.

mod common;

use std::process::Command;

use common::{LANGCHAIN_AGENT, TempDir};

fn gglint(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gglint"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn prints_each_agent_and_a_summary() {
    let dir = TempDir::new();
    dir.write("bot.py", LANGCHAIN_AGENT);
    dir.write("plain.py", "x = 1\n");

    let output = gglint(&[dir.path().as_os_str()]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    assert!(lines[0].ends_with("bot.py:3:9  langchain      agent         via langchain.agents.create_agent  (name=\"support\", model=\"openai:gpt-4o\")"), "{stdout}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Found 1 agent(s) and 0 prompt(s) in 2 file(s)"), "{stderr}");
    assert!(stderr.contains("parsed 1 of them"), "{stderr}");
}

#[test]
fn all_parses_every_file() {
    let dir = TempDir::new();
    dir.write("bot.py", LANGCHAIN_AGENT);
    dir.write("plain.py", "x = 1\n");

    let output = gglint(&["--all".as_ref(), dir.path().as_os_str()]);

    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("parsed 2 of them")
    );
}

#[test]
fn warns_about_syntax_errors() {
    let dir = TempDir::new();
    dir.write("broken.py", format!("{LANGCHAIN_AGENT}def oops(:\n"));

    let output = gglint(&[dir.path().as_os_str()]);

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("warning:")
            && stderr.contains("broken.py")
            && stderr.contains("syntax error"),
        "{stderr}"
    );
}

#[test]
fn fails_when_there_are_no_python_files() {
    let dir = TempDir::new();
    dir.write("README.md", "# nothing here");

    let output = gglint(&[dir.path().as_os_str()]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("No Python files found.")
    );
}

#[test]
fn graph_prints_the_lineage_graph_as_json() {
    let dir = TempDir::new();
    dir.write(
        "bot.py",
        "from langchain.agents import create_agent\n\
         TONE = \"Be brief.\"\n\
         agent = create_agent(model=\"x\", system_prompt=TONE)\n",
    );

    let output = gglint(&["--graph".as_ref(), dir.path().as_os_str()]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("{\"directed\":true"), "{stdout}");
    assert!(stdout.contains("\"id\":\"agent:bot.agent\""), "{stdout}");
    assert!(
        stdout.contains("{\"source\":\"constant:bot.TONE\",\"target\":\"prompt:bot.py:3:47\",\"kind\":\"feeds\",\"position\":0}"),
        "{stdout}"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Graph of 3 node(s) and 2 edge(s) from 1 file(s)"), "{stderr}");
}

// ---------------------------------------------------------------------------
// `gglint ask`, `gglint chat` and `gglint models`
// ---------------------------------------------------------------------------

/// Runs gglint with a models directory that has nothing in it, so nothing is loaded
/// or downloaded.
fn gglint_without_models(args: &[&str], stdin: &str) -> std::process::Output {
    use std::io::Write;

    let models = TempDir::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_gglint"))
        .args(args)
        .env("GGLINT_MODELS_DIR", models.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn ask_answers_with_the_line_it_rests_on() {
    let dir = TempDir::new();
    dir.write("bot.py", LANGCHAIN_AGENT);
    let path = dir.path().to_str().unwrap();

    let output = gglint_without_models(&["ask", "--offline", path, "which agents use gpt-4o"], "");

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "1 agent uses gpt-4o: support.\n  bot.py:3  agent support (langchain, openai:gpt-4o)\n");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("(agents_by_model, layer 0, "), "{stderr}");
}

#[cfg(feature = "embed")]
#[test]
fn ask_offline_without_models_warns_and_still_answers() {
    let dir = TempDir::new();
    dir.write("bot.py", LANGCHAIN_AGENT);
    let path = dir.path().to_str().unwrap();

    let output = gglint_without_models(&["ask", "--offline", path, "list the agents"], "");

    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().starts_with("1 agent: support.\n"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("gglint models pull"), "{stderr}");
}

#[test]
fn chat_answers_each_line_of_input() {
    let dir = TempDir::new();
    dir.write("bot.py", LANGCHAIN_AGENT);
    let path = dir.path().to_str().unwrap();

    let output = gglint_without_models(
        &["chat", "--offline", path],
        "list the agents\n\nwhich models are used\n",
    );

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("1 agent: support."), "{stdout}");
    assert!(stdout.contains("1 model: openai:gpt-4o (1 agent)."), "{stdout}");
}

#[cfg(feature = "embed")]
#[test]
fn models_lists_what_is_missing() {
    let output = gglint_without_models(&["models"], "");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("missing  minishlab/potion-code-16M/model.safetensors"), "{stdout}");
}

#[test]
fn ask_fails_on_a_path_that_does_not_exist() {
    let output = gglint_without_models(&["ask", "--offline", "no/such/dir", "list the agents"], "");

    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("no/such/dir"));
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
        lines[1].ends_with(
            "config.toml:1:14  codex        angular-cli  pnpm dlx @angular/cli mcp --read-only  (disabled)"
        ),
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
    assert!(String::from_utf8(output.stderr).unwrap().contains("No MCP config files found"));
}

#[test]
fn mcp_format_lines_prints_one_line_per_problem() {
    let dir = TempDir::new();
    dir.write(".claude/settings.json", "{}\n");
    dir.write(
        ".codex/config.toml",
        "[mcp_servers.angular-cli]\ncommand = \"pnpm\"\nargs = [\"dlx\", \"@angular/cli\", \"mcp\", \"--read-only\"]\n",
    );
    dir.write(
        ".cursor/mcp.json",
        "{\n  \"mcpServers\": {\n    \"angular-cli\": { \"command\": \"npx\", \"args\": [\"-y\", \"@angular/cli\", \"mcp\"] }\n  }\n}\n",
    );

    let output = gglint(&["mcp".as_ref(), "--format".as_ref(), "lines".as_ref(), dir.path().as_os_str()]);

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
    assert!(lines[1].starts_with("    also at ") && lines[1].ends_with(".cursor/mcp.json:3:5"), "{stdout}");
    assert!(
        lines[2].ends_with(
            "config.toml:1:14  different-server `angular-cli` is started with different arguments: `mcp --read-only` in Codex; `mcp` in Cursor"
        ),
        "{stdout}"
    );
    assert!(lines[3].starts_with("    also at ") && lines[3].ends_with(".cursor/mcp.json:3:5"), "{stdout}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Checked 2 MCP server(s) in 2 file(s) for Claude Code, Codex and Cursor: 2 problem(s)"),
        "{stderr}"
    );
}

#[test]
fn mcp_passes_when_the_harnesses_agree() {
    let dir = TempDir::new();
    dir.write(".codex/config.toml", "[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\n");
    dir.write(".cursor/mcp.json", "{ \"mcpServers\": { \"docs\": { \"url\": \"https://docs.example.com/mcp\" } } }\n");

    let output = gglint(&["mcp".as_ref(), dir.path().as_os_str()]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("│ docs   │   ✓   │   ✓    │"), "{stdout}");
    assert!(
        stdout.ends_with("No problems: every harness gets the same servers, set up the same way.\n"),
        "{stdout}"
    );
    assert!(String::from_utf8(output.stderr).unwrap().contains(": no problems"));
}

/// GGX's MCP servers: two set up for Codex, one of them also for Cursor, none for Claude Code.
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
fn mcp_draws_each_server_in_each_harness_and_what_to_fix() {
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
    ✗ Missing from Claude Code and Cursor.
      Add it to .mcp.json and .cursor/mcp.json, as set up in .codex/config.toml:1

  angular-cli
    ✗ Missing from Claude Code.
      Add it to .mcp.json, as set up in .codex/config.toml:5
    ≠ Started with different arguments: `mcp --read-only` in Codex; `mcp` in Cursor.
      .codex/config.toml:5, .cursor/mcp.json:1
"
    );
}

#[test]
fn mcp_rejects_an_unknown_format() {
    let dir = ggx_like();

    let output = gglint(&["mcp".as_ref(), "--format".as_ref(), "xml".as_ref(), dir.path().as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr).unwrap().contains("unknown format"));
}
