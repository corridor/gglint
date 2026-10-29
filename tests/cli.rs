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
