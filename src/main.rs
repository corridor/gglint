//! The `gglint` command.
//!
//! ```text
//! gglint mcp [--fix [--create]] [--format F] [PATH]
//!                               compare each agent's MCP servers, and add the missing ones
//! gglint mcp --servers [PATH]   list every server
//! ```

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use gglint::harness::{
    Cell, Finding, FixOptions, FixReport, McpCheck, McpConfig, McpServer, Rule, Setting, Transport, check_mcp, fix_mcp,
    read_mcp,
};

const USAGE: &str = "\
Keep the MCP servers of Claude Code, Codex, Cursor and OpenCode in sync.

Usage:
  gglint mcp [PATH]                 compare each coding agent's MCP servers
  gglint mcp --fix [PATH]           add the servers missing from a config file
  gglint mcp --fix --create [PATH]  also create the config file an agent is missing
  gglint mcp --servers [PATH]       list every server, file by file
  gglint mcp --format lines [PATH]  one line per problem, for scripts and CI

PATH is the repository's root, the current directory by default. `--fix` only adds: a
server set up differently in two files is reported and never changed.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("mcp") => mcp(&args[1..]),
        Some("--version" | "-V") => {
            println!("gglint {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h" | "help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        None => {
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
        Some(other) => {
            eprintln!("error: unknown command {other:?}\n");
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// How `gglint mcp` prints what it finds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum McpView {
    /// A table of servers and agents, then the problems under each server.
    Table,
    /// One line per problem.
    Lines,
    /// One line per server, without comparing them.
    Servers,
}

fn mcp(args: &[String]) -> ExitCode {
    let mut view = McpView::Table;
    let mut fix = false;
    let mut options = FixOptions::default();
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
            // Creating a config file is a way of fixing.
            "--create" => {
                fix = true;
                options.create = true;
                continue;
            }
            "--help" | "-h" => {
                print!("{USAGE}");
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
                    path = other;
                    continue;
                }
            },
        };
        view = match format {
            Some("table") => McpView::Table,
            Some("lines") => McpView::Lines,
            other => {
                eprintln!(
                    "error: unknown format {:?}; use `table` or `lines`",
                    other.unwrap_or("")
                );
                return ExitCode::FAILURE;
            }
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
        check_mcp(root, &configs)
    };
    let elapsed = start.elapsed();
    // After fixing, everything below is about the files as they are now.
    let configs = if fix {
        print_fixes(&fix_mcp(root, &configs, &check, &options));
        let configs = read_mcp(root);
        check = check_mcp(root, &configs);
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
    let problems = check.findings().count();
    match view {
        McpView::Table if !configs.is_empty() => draw_mcp(root, &check, problems),
        McpView::Lines => {
            for finding in check.findings() {
                print_finding(finding);
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
            out.push_str(&format!("  {label:<width$}   added {}\n", file.added.join(", ")));
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

/// ANSI colours, when standard output is a terminal and `NO_COLOR` isn't set.
struct Paint {
    on: bool,
}

impl Paint {
    fn new() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
        Self {
            on: std::io::stdout().is_terminal() && !no_color,
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }

    fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }

    fn cell(&self, cell: Cell) -> String {
        match cell {
            Cell::Same => self.paint("32", "✓"),
            Cell::Different => self.paint("33", "≠"),
            Cell::Missing => self.paint("31", "✗"),
            Cell::Off => self.dim("○"),
        }
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
                paint.cell(cell),
                " ".repeat(right)
            ));
        }
        out.push('\n');
    }
    out.push_str(&border("╰", "┴", "╯"));
    out.push_str(&format!(
        "  {} set up   {} set up differently   {} missing   {} turned off\n\n",
        paint.cell(Cell::Same),
        paint.cell(Cell::Different),
        paint.cell(Cell::Missing),
        paint.cell(Cell::Off),
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
        Rule::MissingServer | Rule::MissingConfig => paint.cell(Cell::Missing),
        Rule::DifferentServer | Rule::RenamedServer => paint.cell(Cell::Different),
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

fn print_finding(finding: &Finding) {
    println!(
        "{}:{}  {:<16} {}",
        finding.site.path.display(),
        finding.site.location,
        finding.rule.as_str(),
        finding.message
    );
    for site in &finding.related {
        println!("    also at {}:{}", site.path.display(), site.location);
    }
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
