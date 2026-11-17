//! The `gglint` command.
//!
//! ```text
//! gglint mcp [--fix] [--source AGENT] [--format F] [PATH]
//!                               compare each agent's MCP servers, and fix them
//! gglint mcp --servers [PATH]   list every server
//! gglint secrets [--fix] [--staged] [--format F] [PATH | FILE...]
//!                               find secrets written out in agent config files
//! gglint hook [install|print] [--tool T] [--with-mcp] [PATH]
//!                               run gglint before every commit
//! ```

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use gglint::harness::{
    Cell, Finding, FixReport, HARNESSES, Harness, McpCheck, McpConfig, McpServer, Rule, Setting, Transport,
    check_mcp_from, fix_mcp, read_mcp,
};
use gglint::hook::{self, Outcome, TOOLS, Tool};
use gglint::report::{self, Diagnostic, Paint, Severity, exit_code};
use gglint::secrets::{Scope, apply_fixes, ignore_local_files, scan_files};

const USAGE: &str = "\
Keep the MCP servers of every coding agent in a repository in sync.

Usage:
  gglint mcp [PATH]                 compare each coding agent's MCP servers
  gglint mcp --fix [PATH]           give every agent the same servers, started the same way
  gglint mcp --fix --source codex   take each server's setup from Codex where it has one
  gglint mcp --servers [PATH]       list every server, file by file
  gglint mcp --format lines [PATH]  one line per problem, for scripts and CI
  gglint secrets [PATH]             find secrets written out in agent config files
  gglint secrets --fix [PATH]       replace each with a reference to an environment variable
  gglint secrets --staged           check only the files staged in git, for a pre-commit hook
  gglint secrets FILE...            check these files, as a pre-commit hook passes them
  gglint hook [PATH]                run `gglint secrets --staged` before every commit, through
                                    husky or git; for pre-commit, lefthook and lint-staged it
                                    prints the lines to add. `--tool NAME` picks the tool,
                                    `--with-mcp` adds the MCP check, `hook print` only prints.

`--format` takes table, lines, github (annotations for a pull request's diff), sarif (for
GitHub's Code scanning) or markdown (a pull request comment), for `mcp` and `secrets`.

PATH is the repository's root, the current directory by default. `--fix` adds missing
servers, creates the config file of an agent that has its own directory, and copies the
command and arguments of the first agent where a server is turned on to the others, in the
order of the agents below. `--source` puts one agent first, named as below. `--fix` never
removes a server; everything else set up differently is reported only.
";

/// What `gglint --help` prints: `USAGE`, then every agent with the name `--source` takes and
/// the file its servers are read from.
fn usage() -> String {
    let id_width = HARNESSES
        .iter()
        .map(|harness| harness.as_str().len())
        .max()
        .unwrap_or(0);
    let name_width = HARNESSES
        .iter()
        .map(|harness| harness.display_name().len())
        .max()
        .unwrap_or(0);
    let agents: String = HARNESSES
        .iter()
        .map(|harness| {
            format!(
                "  {:<id_width$}  {:<name_width$}  {}\n",
                harness.as_str(),
                harness.display_name(),
                harness.mcp_file()
            )
        })
        .collect();
    format!("{USAGE}\nAgents:\n{agents}")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("mcp") => mcp(&args[1..]),
        Some("secrets") => scan_command(&args[1..]),
        Some("hook") => hook_command(&args[1..]),
        Some("--version" | "-V") => {
            println!("gglint {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h" | "help") => {
            print!("{}", usage());
            ExitCode::SUCCESS
        }
        None => {
            eprint!("{}", usage());
            ExitCode::FAILURE
        }
        Some(other) => {
            eprintln!("error: unknown command {other:?}\n");
            eprint!("{}", usage());
            ExitCode::FAILURE
        }
    }
}

/// How a command prints its findings.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Output {
    /// The command's own drawing, for reading in a terminal.
    Table,
    /// One line per problem.
    Lines,
    /// GitHub Actions annotations on the lines of a pull request.
    Github,
    /// A SARIF log, for GitHub's Code scanning.
    Sarif,
    /// A section for a pull request comment.
    Markdown,
}

impl Output {
    fn parse(name: Option<&str>) -> Result<Output, ExitCode> {
        match name {
            Some("table") => Ok(Output::Table),
            Some("lines") => Ok(Output::Lines),
            Some("github") => Ok(Output::Github),
            Some("sarif") => Ok(Output::Sarif),
            Some("markdown") => Ok(Output::Markdown),
            other => {
                eprintln!(
                    "error: unknown format {:?}; use `table`, `lines`, `github`, `sarif` or `markdown`",
                    other.unwrap_or("")
                );
                Err(ExitCode::FAILURE)
            }
        }
    }
}

/// Prints `diagnostics` in `output`, for every format but the table, which each command draws
/// itself. `command` names the SARIF run and the Markdown section.
fn print_report(output: Output, command: &str, diagnostics: &[Diagnostic]) {
    match output {
        Output::Table => {}
        Output::Lines => {
            for diagnostic in diagnostics {
                print!("{}", diagnostic.line());
            }
        }
        Output::Github => print!("{}", report::github(diagnostics)),
        Output::Sarif => print!("{}", report::sarif(command, diagnostics)),
        Output::Markdown => print!("{}", report::markdown(&format!("gglint {command}"), diagnostics)),
    }
}

/// How `gglint mcp` prints what it finds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum McpView {
    /// A table of servers and agents, then the problems under each server.
    Table,
    /// The problems in one of the formats for scripts and CI.
    Report(Output),
    /// One line per server, without comparing them.
    Servers,
}

fn mcp(args: &[String]) -> ExitCode {
    let mut view = McpView::Table;
    let mut fix = false;
    let mut source: Option<Harness> = None;
    let mut path = ".";
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let format = match arg.as_str() {
            "--servers" => {
                view = McpView::Servers;
                continue;
            }
            "--fix" => {
                fix = true;
                continue;
            }
            "--help" | "-h" => {
                print!("{}", usage());
                return ExitCode::SUCCESS;
            }
            "--format" => args.next().map(String::as_str),
            "--source" => match parse_source(args.next().map(String::as_str)) {
                Ok(harness) => {
                    source = Some(harness);
                    continue;
                }
                Err(code) => return code,
            },
            other if other.starts_with("--source=") => match parse_source(other.strip_prefix("--source=")) {
                Ok(harness) => {
                    source = Some(harness);
                    continue;
                }
                Err(code) => return code,
            },
            other => match other.strip_prefix("--format=") {
                Some(format) => Some(format),
                None if other.starts_with('-') => {
                    eprintln!("error: unknown option {other:?}; see `gglint --help`");
                    return ExitCode::FAILURE;
                }
                None => {
                    path = other;
                    continue;
                }
            },
        };
        view = match Output::parse(format) {
            Ok(Output::Table) => McpView::Table,
            Ok(output) => McpView::Report(output),
            Err(code) => return code,
        };
    }
    if fix && view == McpView::Servers {
        eprintln!("error: `--fix` changes what `--servers` lists; use one at a time");
        return ExitCode::FAILURE;
    }
    let root = Path::new(path);
    if !root.exists() {
        eprintln!("error: {path} doesn't exist");
        return ExitCode::FAILURE;
    }
    let start = Instant::now();
    let configs = read_mcp(root);
    let mut check = if view == McpView::Servers {
        McpCheck::default()
    } else {
        check_mcp_from(root, &configs, source)
    };
    let elapsed = start.elapsed();
    // After fixing, everything below is about the files as they are now.
    let configs = if fix {
        print_fixes(&fix_mcp(root, &configs, &check));
        let configs = read_mcp(root);
        check = check_mcp_from(root, &configs, source);
        configs
    } else {
        configs
    };

    let mut servers = 0;
    for config in &configs {
        for warning in &config.warnings {
            eprintln!("warning: {}: {warning}", config.path.display());
        }
        servers += config.servers.len();
        if view == McpView::Servers {
            for server in &config.servers {
                print_mcp_server(config, server);
            }
        }
    }
    if let Some(source) = source
        && view != McpView::Servers
        && !check.harnesses.contains(&source)
    {
        let what = match source.directory() {
            Some(directory) => format!("no MCP config file or {directory} directory"),
            None => format!("no {}", source.mcp_file()),
        };
        eprintln!(
            "note: {} has {what} here, so --source {} changes nothing",
            source.display_name(),
            source.as_str()
        );
    }
    let problems = check.findings().count();
    match view {
        McpView::Table if !configs.is_empty() => draw_mcp(root, &check, problems),
        McpView::Report(output) => {
            let diagnostics: Vec<Diagnostic> = check.findings().map(Diagnostic::from).collect();
            print_report(output, "mcp", &diagnostics);
            if output == Output::Markdown {
                print!("{}", mcp_matrix_markdown(&check));
            }
        }
        _ => {}
    }

    let milliseconds = elapsed.as_secs_f64() * 1000.0;
    if view == McpView::Servers {
        let mut with_servers: Vec<_> = configs.iter().map(|config| config.harness).collect();
        with_servers.dedup();
        eprintln!(
            "\nFound {servers} MCP server(s) in {} file(s) for {} agent(s); {milliseconds:.2} ms",
            configs.len(),
            with_servers.len(),
        );
    } else {
        let names: Vec<&str> = check.harnesses.iter().map(|harness| harness.display_name()).collect();
        let names = match names.as_slice() {
            [] => "no agent".to_string(),
            [only] => only.to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        let problems = match problems {
            0 => "no problems".to_string(),
            count => format!("{count} problem(s)"),
        };
        eprintln!(
            "\nChecked {servers} MCP server(s) in {} file(s) for {names}: {problems}; {milliseconds:.2} ms",
            configs.len(),
        );
    }
    if configs.is_empty() {
        eprintln!("No MCP config files found.");
        return ExitCode::FAILURE;
    }
    if problems == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `gglint secrets`: finds secrets written out in the agents' config and instruction files.
fn scan_command(args: &[String]) -> ExitCode {
    let mut fix = false;
    let mut staged = false;
    let mut output = Output::Table;
    let mut root: Option<&str> = None;
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let format = match arg.as_str() {
            "--fix" => {
                fix = true;
                continue;
            }
            "--staged" => {
                staged = true;
                continue;
            }
            "--help" | "-h" => {
                print!("{}", usage());
                return ExitCode::SUCCESS;
            }
            "--format" => args.next().map(String::as_str),
            other => match other.strip_prefix("--format=") {
                Some(format) => Some(format),
                None if other.starts_with('-') => {
                    eprintln!("error: unknown option {other:?}; see `gglint --help`");
                    return ExitCode::FAILURE;
                }
                None => {
                    // A directory is the repository's root; anything else is a file to check.
                    if Path::new(other).is_dir() {
                        root = Some(other);
                    } else {
                        files.push(other.into());
                    }
                    continue;
                }
            },
        };
        output = match Output::parse(format) {
            Ok(output) => output,
            Err(code) => return code,
        };
    }
    if staged && !files.is_empty() {
        eprintln!("error: `--staged` and a list of files both say what to check; use one");
        return ExitCode::FAILURE;
    }
    if staged && fix {
        eprintln!(
            "error: `--fix` changes the files on disk, not what's staged; run `gglint secrets --fix`, then `git add` the files again"
        );
        return ExitCode::FAILURE;
    }
    let root = Path::new(root.unwrap_or("."));
    let scope = if staged {
        Scope::Staged
    } else if files.is_empty() {
        Scope::Known
    } else {
        Scope::Files(files)
    };

    let start = Instant::now();
    let mut check = match scan_files(root, &scope) {
        Ok(check) => check,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    if fix && (!check.fixes.is_empty() || !check.ignore.is_empty()) {
        match apply_fixes(&check) {
            Ok(changed) if !changed.is_empty() => {
                let count = changed.iter().map(|(_, count)| count).sum::<usize>();
                println!("Replaced {count} secret(s) with references to environment variables:");
                for (path, count) in &changed {
                    println!("  {}  ({count})", path.display());
                }
                println!(
                    "\nSet those variables in your environment. Anything already committed stays in git history: rotate it.\n"
                );
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
        match ignore_local_files(root, &check) {
            Ok(added) if !added.is_empty() => {
                println!("Added to {}:", root.join(".gitignore").display());
                for path in &added {
                    println!("  {}", path.display());
                }
                println!();
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
        check = match scan_files(root, &scope) {
            Ok(check) => check,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        };
    }
    let elapsed = start.elapsed();

    if output != Output::Table {
        print_report(output, "secrets", &check.diagnostics);
    } else if !check.diagnostics.is_empty() {
        let paint = Paint::new();
        let mut out = String::new();
        let mut file: Option<&Path> = None;
        for diagnostic in &check.diagnostics {
            if file != Some(diagnostic.relative_path.as_path()) {
                file = Some(diagnostic.relative_path.as_path());
                out.push_str(&format!(
                    "\n{}\n",
                    paint.bold(&diagnostic.relative_path.display().to_string())
                ));
            }
            let symbol = match diagnostic.severity {
                Severity::Error => paint.red("✗"),
                Severity::Warning => paint.yellow("!"),
            };
            out.push_str(&format!(
                "  {symbol} {:>5}  {}\n{}\n",
                diagnostic.location.to_string(),
                diagnostic.message,
                paint.dim(&format!("            {}", diagnostic.fix.as_deref().unwrap_or("")))
            ));
        }
        print!("{out}");
    }
    let problems = check.diagnostics.len();
    let warnings = check
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Warning)
        .count();
    let problems_text = match (problems - warnings, warnings) {
        (0, 0) => "no problems".to_string(),
        (errors, 0) => format!("{errors} problem(s)"),
        (0, warnings) => format!("{warnings} warning(s)"),
        (errors, warnings) => format!("{errors} problem(s), {warnings} warning(s)"),
    };
    eprintln!(
        "\nChecked {} file(s): {problems_text}; {:.2} ms",
        check.files,
        elapsed.as_secs_f64() * 1000.0
    );
    if problems > 0 && !fix && !check.fixes.is_empty() {
        eprintln!("`gglint secrets --fix` can replace {} of them.", check.fixes.len());
    }
    exit_code(&check.diagnostics)
}

/// `gglint hook`: runs gglint before every commit.
fn hook_command(args: &[String]) -> ExitCode {
    let mut print_only = false;
    let mut with_mcp = false;
    let mut tool: Option<Tool> = None;
    let mut path = ".";
    let mut args = args.iter().peekable();
    if let Some(action) = args.next_if(|arg| matches!(arg.as_str(), "install" | "print")) {
        print_only = action == "print";
    }
    while let Some(arg) = args.next() {
        let name = match arg.as_str() {
            "--with-mcp" => {
                with_mcp = true;
                continue;
            }
            "--help" | "-h" => {
                print!("{}", usage());
                return ExitCode::SUCCESS;
            }
            "--tool" => args.next().map(String::as_str),
            other => match other.strip_prefix("--tool=") {
                Some(name) => Some(name),
                None if other.starts_with('-') => {
                    eprintln!("error: unknown option {other:?}; see `gglint --help`");
                    return ExitCode::FAILURE;
                }
                None => {
                    path = other;
                    continue;
                }
            },
        };
        tool = match name.and_then(Tool::parse) {
            Some(tool) => Some(tool),
            None => {
                let names: Vec<&str> = TOOLS.iter().map(|tool| tool.as_str()).collect();
                eprintln!(
                    "error: unknown tool {:?} for --tool; use {}",
                    name.unwrap_or(""),
                    names.join(", ")
                );
                return ExitCode::FAILURE;
            }
        };
    }
    let root = Path::new(path);
    if !root.is_dir() {
        eprintln!("error: {path} isn't a directory");
        return ExitCode::FAILURE;
    }
    let tool = tool.unwrap_or_else(|| hook::detect(root));
    if print_only {
        let (file, snippet) = hook::print(tool, with_mcp);
        println!("# {} ({file})\n{snippet}", tool.as_str());
        return ExitCode::SUCCESS;
    }
    match hook::install(root, tool, with_mcp) {
        Ok(Outcome::Wrote(file)) => {
            println!(
                "Added gglint to {}. It runs before each commit ({}).",
                file.display(),
                tool.as_str()
            );
            ExitCode::SUCCESS
        }
        Ok(Outcome::AlreadyThere(file)) => {
            println!("{} already runs gglint; nothing to change.", file.display());
            ExitCode::SUCCESS
        }
        Ok(Outcome::Print { file, snippet }) => {
            println!(
                "This repository uses {}. Add this to {file}:\n\n{snippet}",
                tool.as_str()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The agent `--source` names, or the exit code after saying why it isn't one.
fn parse_source(name: Option<&str>) -> Result<Harness, ExitCode> {
    name.and_then(Harness::parse).ok_or_else(|| {
        let ids: Vec<&str> = HARNESSES.iter().map(|harness| harness.as_str()).collect();
        let ids = match ids.as_slice() {
            [rest @ .., last] if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            _ => ids.join(""),
        };
        eprintln!("error: unknown agent {:?} for --source; use {ids}", name.unwrap_or(""));
        ExitCode::FAILURE
    })
}

/// What `--fix` changed, file by file, and what it left as it was.
fn print_fixes(report: &FixReport) {
    let mut out = String::new();
    if report.changed.is_empty() && report.unfixed.is_empty() {
        out.push_str("Nothing to fix.\n\n");
    }
    if !report.changed.is_empty() {
        let labels: Vec<String> = report
            .changed
            .iter()
            .map(|file| {
                let notes: Vec<&str> = [(file.created, "new"), (file.ignored, "gitignored")]
                    .into_iter()
                    .filter_map(|(on, note)| on.then_some(note))
                    .collect();
                if notes.is_empty() {
                    file.relative_path.display().to_string()
                } else {
                    format!("{} ({})", file.relative_path.display(), notes.join(", "))
                }
            })
            .collect();
        let width = labels
            .iter()
            .map(|label| label.chars().count())
            .max()
            .unwrap_or_default();
        let count = report.changed.len();
        out.push_str(&format!("Changed {count} file{}:\n", if count == 1 { "" } else { "s" }));
        for (file, label) in report.changed.iter().zip(&labels) {
            let mut what = Vec::new();
            if !file.added.is_empty() {
                what.push(format!("added {}", file.added.join(", ")));
            }
            if !file.updated.is_empty() {
                what.push(format!("updated {}", file.updated.join(", ")));
            }
            out.push_str(&format!("  {label:<width$}   {}\n", what.join("; ")));
        }
        if report.changed.iter().any(|file| file.ignored) {
            out.push_str("  Gitignored files aren't committed, so those changes stay on this machine.\n");
        }
        out.push('\n');
    }
    if !report.unfixed.is_empty() {
        out.push_str("Not added:\n");
        for unfixed in &report.unfixed {
            out.push_str(&format!(
                "  {} in {}: {}\n",
                unfixed.server,
                unfixed.harness.display_name(),
                unfixed.reason
            ));
        }
        out.push('\n');
    }
    if !report.changed.is_empty() {
        out.push_str("Review the changes with `git diff`.\n\n");
    }
    print!("{out}");
}

/// The symbol for a table cell, coloured.
fn symbol(paint: &Paint, cell: Cell) -> String {
    match cell {
        Cell::Same => paint.green("✓"),
        Cell::Different => paint.yellow("≠"),
        Cell::Missing => paint.red("✗"),
        Cell::Off => paint.dim("○"),
    }
}

/// Draws each server in each agent as a table, then the problems under each server, then the
/// agents missing a config file.
fn draw_mcp(root: &Path, check: &McpCheck, problems: usize) {
    let (harnesses, checks) = (&check.harnesses, &check.servers);
    let paint = Paint::new();
    let repository = std::fs::canonicalize(root)
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_else(|| root.display().to_string());
    let mut out = paint.bold(&format!("MCP servers in {repository}"));
    out.push_str("\n\n");
    if checks.is_empty() {
        out.push_str("  No MCP servers are set up.\n");
        print!("{out}");
        return;
    }

    let first = "Server";
    let first_width = checks
        .iter()
        .map(|check| check.name.chars().count())
        .chain([first.len()])
        .max()
        .unwrap_or_default();
    let widths: Vec<usize> = harnesses.iter().map(|harness| harness.display_name().len()).collect();
    let border = |left: &str, middle: &str, right: &str| {
        let mut line = format!("  {left}{}", "─".repeat(first_width + 2));
        for width in &widths {
            line.push_str(middle);
            line.push_str(&"─".repeat(width + 2));
        }
        line.push_str(right);
        line.push('\n');
        line
    };
    out.push_str(&border("╭", "┬", "╮"));
    out.push_str(&format!("  │ {first:<first_width$} │"));
    for harness in harnesses {
        out.push_str(&format!(" {} │", harness.display_name()));
    }
    out.push('\n');
    out.push_str(&border("├", "┼", "┤"));
    for check in checks {
        out.push_str(&format!("  │ {:<first_width$} │", check.name));
        for (&(_, cell), width) in check.cells.iter().zip(&widths) {
            // Centred: each symbol is one column wide.
            let left = (width - 1) / 2;
            let right = width - 1 - left;
            out.push_str(&format!(
                " {}{}{} │",
                " ".repeat(left),
                symbol(&paint, cell),
                " ".repeat(right)
            ));
        }
        out.push('\n');
    }
    out.push_str(&border("╰", "┴", "╯"));
    out.push_str(&format!(
        "  {} set up   {} set up differently   {} missing   {} turned off\n\n",
        symbol(&paint, Cell::Same),
        symbol(&paint, Cell::Different),
        symbol(&paint, Cell::Missing),
        symbol(&paint, Cell::Off),
    ));

    if problems == 0 {
        out.push_str("No problems: every agent gets the same servers, set up the same way.\n");
        print!("{out}");
        return;
    }
    let count = if problems == 1 {
        "1 problem".to_string()
    } else {
        format!("{problems} problems")
    };
    out.push_str(&paint.bold(&count));
    out.push('\n');
    for check in checks.iter().filter(|check| !check.findings.is_empty()) {
        out.push_str(&format!("\n  {}\n", paint.bold(&check.name)));
        for finding in &check.findings {
            out.push_str(&describe(&paint, finding));
        }
    }
    if !check.missing_configs.is_empty() {
        out.push_str(&format!("\n  {}\n", paint.bold("Missing config files")));
        for finding in &check.missing_configs {
            out.push_str(&describe(&paint, finding));
        }
    }
    print!("{out}");
}

/// A finding under its heading: what's wrong, then how to fix it or where to look.
fn describe(paint: &Paint, finding: &Finding) -> String {
    let symbol = match finding.rule {
        Rule::MissingServer | Rule::MissingConfig => symbol(paint, Cell::Missing),
        Rule::DifferentServer | Rule::RenamedServer => symbol(paint, Cell::Different),
    };
    let next = match &finding.fix {
        Some(fix) => fix.clone(),
        None => std::iter::once(&finding.site)
            .chain(&finding.related)
            .map(|site| format!("{}:{}", site.relative_path.display(), site.location.line))
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!("    {symbol} {}\n      {}\n", finding.detail, paint.dim(&next))
}

/// The servers and the agents that get them, as a Markdown table.
fn mcp_matrix_markdown(check: &McpCheck) -> String {
    if check.servers.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n| Server |");
    for harness in &check.harnesses {
        out.push_str(&format!(" {} |", harness.display_name()));
    }
    out.push_str("\n| --- |");
    for _ in &check.harnesses {
        out.push_str(" :---: |");
    }
    out.push('\n');
    for server in &check.servers {
        out.push_str(&format!("| `{}` |", server.name));
        for &(_, cell) in &server.cells {
            let symbol = match cell {
                Cell::Same => "✅",
                Cell::Different => "⚠️",
                Cell::Missing => "❌",
                Cell::Off => "➖",
            };
            out.push_str(&format!(" {symbol} |"));
        }
        out.push('\n');
    }
    out.push_str("\n✅ set up · ⚠️ set up differently · ❌ missing · ➖ turned off\n");
    out
}

fn print_mcp_server(config: &McpConfig, server: &McpServer) {
    let target = match &server.transport {
        Transport::Command { command, args } => {
            let mut words = vec![command.as_str()];
            words.extend(args.iter().map(String::as_str));
            words.join(" ")
        }
        Transport::Remote { url } => url.clone(),
    };
    // Names only: a value can be a secret written out in full.
    let names = |settings: &[Setting]| {
        settings
            .iter()
            .map(|setting| setting.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut details = Vec::new();
    if !server.env.is_empty() {
        details.push(format!("env=[{}]", names(&server.env)));
    }
    if !server.headers.is_empty() {
        details.push(format!("headers=[{}]", names(&server.headers)));
    }
    if let Some(tools) = &server.enabled_tools {
        details.push(format!("enabled_tools=[{}]", tools.join(", ")));
    }
    if !server.disabled_tools.is_empty() {
        details.push(format!("disabled_tools=[{}]", server.disabled_tools.join(", ")));
    }
    if !server.enabled {
        details.push("disabled".to_string());
    }
    println!(
        "{}:{}  {:<12} {}  {target}{}",
        config.path.display(),
        server.location,
        config.harness.as_str(),
        server.name,
        if details.is_empty() {
            String::new()
        } else {
            format!("  ({})", details.join(", "))
        }
    );
}
