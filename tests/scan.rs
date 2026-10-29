//! `scan` on real directories: which files are read, how they're reported, and what is
//! found in them.

mod common;

use std::path::{Path, PathBuf};

use common::{LANGCHAIN_AGENT, TempDir};
use gglint::discovery::{AgentKind, FileResult, scan};

fn scan_dir(root: &Path) -> Vec<FileResult> {
    scan(&[root.to_path_buf()], false)
}

fn relative_paths(results: &[FileResult]) -> Vec<String> {
    results
        .iter()
        .map(|result| result.relative_path.to_string_lossy().replace('\\', "/"))
        .collect()
}

#[test]
fn finds_python_files_in_subdirectories_in_path_order() {
    let dir = TempDir::new();
    dir.write("z.py", "");
    dir.write("app/bots/support.py", LANGCHAIN_AGENT);
    dir.write("app/__init__.py", "");
    dir.write("notes.txt", "agent = create_agent()");
    dir.write("script.pyi", "");

    let results = scan_dir(dir.path());

    assert_eq!(
        relative_paths(&results),
        vec!["app/__init__.py", "app/bots/support.py", "z.py"]
    );
}

#[test]
fn skips_hidden_virtualenv_and_build_directories() {
    let dir = TempDir::new();
    for skipped in [
        ".git/hooks/x.py",
        ".venv/lib/x.py",
        "venv/lib/x.py",
        "node_modules/pkg/x.py",
        "__pycache__/x.py",
        "target/x.py",
        "lib/site-packages/x.py",
    ] {
        dir.write(skipped, LANGCHAIN_AGENT);
    }
    dir.write("lib/kept.py", LANGCHAIN_AGENT);

    let results = scan_dir(dir.path());

    assert_eq!(relative_paths(&results), vec!["lib/kept.py"]);
}

#[test]
fn reports_the_module_path_and_the_location_of_each_agent() {
    let dir = TempDir::new();
    dir.write("app/__init__.py", LANGCHAIN_AGENT);
    dir.write("app/bots.py", format!("\n\n{LANGCHAIN_AGENT}"));

    let results = scan_dir(dir.path());

    let found: Vec<_> = results
        .iter()
        .flat_map(|result| {
            result.agents.iter().map(move |(location, agent)| {
                (
                    result.module.as_str(),
                    location.line,
                    location.column,
                    agent.kind,
                    agent.variable.as_deref(),
                    agent.name.as_deref(),
                    agent.model.as_deref(),
                )
            })
        })
        .collect();
    assert_eq!(
        found,
        vec![
            (
                "app",
                3,
                9,
                AgentKind::Instance,
                Some("agent"),
                Some("support"),
                Some("openai:gpt-4o")
            ),
            (
                "app.bots",
                5,
                9,
                AgentKind::Instance,
                Some("agent"),
                Some("support"),
                Some("openai:gpt-4o")
            ),
        ]
    );
}

#[test]
fn files_that_cannot_contain_an_agent_are_not_parsed() {
    let dir = TempDir::new();
    dir.write("plain.py", "def add(a, b):\n    return a + b\n");
    dir.write("bot.py", LANGCHAIN_AGENT);

    let results = scan_dir(dir.path());
    let parsed: Vec<_> = results
        .iter()
        .map(|result| (result.relative_path.clone(), result.parsed))
        .collect();
    assert_eq!(
        parsed,
        vec![
            (PathBuf::from("bot.py"), true),
            (PathBuf::from("plain.py"), false)
        ]
    );

    // `parse_all` parses them anyway, and finds the same agents.
    let results = scan(&[dir.path().to_path_buf()], true);
    assert!(results.iter().all(|result| result.parsed));
    assert_eq!(
        results
            .iter()
            .map(|result| result.agents.len())
            .sum::<usize>(),
        1
    );
}

#[test]
fn a_syntax_error_is_reported_and_the_rest_of_the_file_is_still_read() {
    let dir = TempDir::new();
    dir.write(
        "broken.py",
        format!("{LANGCHAIN_AGENT}def oops(:\n    pass\n\nlater = create_agent(model='x')\n"),
    );

    let results = scan_dir(dir.path());

    let result = &results[0];
    let warning = result
        .warning
        .as_deref()
        .expect("a warning for the syntax error");
    assert!(warning.contains("syntax error"), "{warning}");
    assert!(
        warning.contains("4:"),
        "the warning names the line of the error: {warning}"
    );
    let variables: Vec<_> = result
        .agents
        .iter()
        .map(|(_, agent)| agent.variable.as_deref())
        .collect();
    assert_eq!(variables, vec![Some("agent"), Some("later")]);
}

#[test]
fn a_file_that_is_not_utf8_is_skipped() {
    let dir = TempDir::new();
    dir.write(
        "latin1.py",
        b"# caf\xe9\nfrom langchain.agents import create_agent\n",
    );
    dir.write("bot.py", LANGCHAIN_AGENT);

    let results = scan_dir(dir.path());

    assert_eq!(relative_paths(&results), vec!["bot.py"]);
}

#[test]
fn a_single_file_can_be_scanned() {
    let dir = TempDir::new();
    let file = dir.write("app/bot.py", LANGCHAIN_AGENT);

    let results = scan(&[file], false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].agents.len(), 1);
}

#[test]
fn several_roots_are_scanned_together() {
    let first = TempDir::new();
    let second = TempDir::new();
    first.write("a.py", LANGCHAIN_AGENT);
    second.write("b.py", LANGCHAIN_AGENT);

    let results = scan(
        &[first.path().to_path_buf(), second.path().to_path_buf()],
        false,
    );

    assert_eq!(
        results
            .iter()
            .map(|result| result.agents.len())
            .sum::<usize>(),
        2
    );
    // Paths are relative to the root each file was found under.
    let mut paths = relative_paths(&results);
    paths.sort();
    assert_eq!(paths, vec!["a.py", "b.py"]);
}

#[test]
fn a_missing_path_gives_no_results() {
    let dir = TempDir::new();
    assert!(scan_dir(&dir.path().join("missing")).is_empty());
}
