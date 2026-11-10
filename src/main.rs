//! The `gglint` command: prints the agents found under the given directories, or the
//! lineage graph as JSON, and answers questions about them. It also checks that the coding
//! harnesses a repository is set up for get the same MCP servers.
//!
//! ```text
//! gglint [--all] [PATH ...]           list agents and prompts
//! gglint --graph [PATH ...]           print the lineage graph as JSON
//! gglint ask [FLAGS] [PATH] QUESTION  answer one question
//! gglint chat [FLAGS] [PATH]          answer each line read from standard input
//! gglint models [pull]                show, or download, the models `ask` uses
//! gglint mcp [--fix] [--format F] [PATH]
//!                                     compare each harness's MCP servers, and fix them
//! gglint mcp --servers [PATH]         list them
//! ```
//!
//! Flags for `ask` and `chat`: `--offline` (never download), `--no-embed` (no
//! embedding model: keywords only).

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use gglint::ask::{Answer, Options, Repo};
use gglint::discovery::{AgentKind, DiscoveredAgent, DiscoveredPrompt, FileResult, Location, scan};
use gglint::harness::{
    Cell, Finding, FixReport, Harness, McpConfig, McpServer, Rule, ServerCheck, Setting, Transport, check_mcp,
    fix_mcp, read_mcp, used_harnesses,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ask") => ask(&args[1..]),
        Some("chat") => chat(&args[1..]),
        Some("models") => models(&args[1..]),
        Some("mcp") => mcp(&args[1..]),
        _ => list(&args),
    }
}

/// Splits `ask` and `chat` arguments into options and the remaining words.
fn ask_options(args: &[String]) -> (Options, Vec<&str>) {
    let mut options = Options::default();
    let mut rest = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--offline" => options.download = false,
            "--no-embed" => options.embed = false,
            _ => rest.push(arg.as_str()),
        }
    }
    (options, rest)
}

fn open(path: &str, options: &Options) -> Option<Repo> {
    if !Path::new(path).exists() {
        eprintln!("error: {path} doesn't exist");
        return None;
    }
    let start = Instant::now();
    let repo = Repo::open(&[PathBuf::from(path)], options);
    eprintln!(
        "gglint: {} agent(s) and {} prompt(s) in {} file(s), ready in {:.0} ms",
        repo.graph().agents.len(),
        repo.graph().prompts.len(),
        repo.graph().files,
        start.elapsed().as_secs_f64() * 1000.0,
    );
    Some(repo)
}

fn print_warnings(repo: &Repo) {
    for warning in &repo.warnings {
        eprintln!("warning: {warning}");
    }
}

fn print_answer(answer: &Answer) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", answer.text);
    for fact in &answer.facts {
        let _ = writeln!(stdout, "  {}:{}  {}", fact.path, fact.line, fact.label);
    }
    let _ = stdout.flush();
    eprintln!(
        "({}, layer {}, {:.2} ms)",
        answer.intent,
        answer.layer,
        answer.elapsed.as_secs_f64() * 1000.0
    );
}

fn ask(args: &[String]) -> ExitCode {
    let (options, rest) = ask_options(args);
    let (path, question) = match rest.as_slice() {
        [question] => (".", question.to_string()),
        [path, question @ ..] if !question.is_empty() => (*path, question.join(" ")),
        _ => {
            eprintln!("usage: gglint ask [--offline] [--no-embed] [PATH] QUESTION");
            return ExitCode::FAILURE;
        }
    };
    let Some(repo) = open(path, &options) else {
        return ExitCode::FAILURE;
    };
    print_warnings(&repo);
    print_answer(&repo.ask(&question));
    ExitCode::SUCCESS
}

fn chat(args: &[String]) -> ExitCode {
    let (options, rest) = ask_options(args);
    let path = rest.first().copied().unwrap_or(".");
    let Some(repo) = open(path, &options) else {
        return ExitCode::FAILURE;
    };
    print_warnings(&repo);
    let interactive = std::io::stdin().is_terminal();
    if interactive {
        eprintln!("Ask about the agents and prompts in {path}. Ctrl-D to quit.");
    }
    let mut lines = std::io::stdin().lock().lines();
    loop {
        if interactive {
            eprint!("> ");
        }
        let Some(Ok(line)) = lines.next() else { break };
        let question = line.trim();
        if question.is_empty() {
            continue;
        }
        print_answer(&repo.ask(question));
    }
    ExitCode::SUCCESS
}

#[cfg(feature = "download")]
fn models(args: &[String]) -> ExitCode {
    use gglint::ask::models::{EMBED_FILES, cached, ensure, local_path, verify};

    let files = EMBED_FILES;
    match args.first().map(String::as_str) {
        None => {
            for file in &files {
                let state = if cached(std::slice::from_ref(file)) { "cached " } else { "missing" };
                println!("{state}  {}/{}  {}", file.repo, file.file, local_path(file).display());
            }
            ExitCode::SUCCESS
        }
        Some("pull") => {
            for file in &files {
                if let Err(error) = ensure(file, true).and_then(|_| verify(file)) {
                    eprintln!("error: {error}");
                    return ExitCode::FAILURE;
                }
                println!("ok  {}/{}", file.repo, file.file);
            }
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown `gglint models` command {other:?}; use `gglint models` or `gglint models pull`");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "download"))]
fn models(_: &[String]) -> ExitCode {
    eprintln!("gglint was built without the embedding model (the `embed` feature)");
    ExitCode::FAILURE
}

/// How `gglint mcp` prints what it finds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum McpView {
    /// A table of servers and harnesses, then the problems under each server.
    Table,
    /// One line per problem.
    Lines,
    /// One line per server, without comparing them.
    Servers,
}

fn mcp(args: &[String]) -> ExitCode {
    let mut view = McpView::Table;
    let mut fix = false;
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
            "--format" => args.next().map(String::as_str),
            other => match other.strip_prefix("--format=") {
                Some(format) => Some(format),
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
                eprintln!("error: unknown format {:?}; use `table` or `lines`", other.unwrap_or(""));
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
    let harnesses = used_harnesses(root);
    let mut checks = if view == McpView::Servers { Vec::new() } else { check_mcp(&configs, &harnesses) };
    let elapsed = start.elapsed();
    // After fixing, everything below is about the files as they are now.
    let configs = if fix {
        print_fixes(&fix_mcp(root, &configs, &checks));
        let configs = read_mcp(root);
        checks = check_mcp(&configs, &harnesses);
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
    let problems: usize = checks.iter().map(|check| check.findings.len()).sum();
    match view {
        McpView::Table if !configs.is_empty() => draw_mcp(root, &harnesses, &checks, problems),
        McpView::Lines => {
            for finding in checks.iter().flat_map(|check| &check.findings) {
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
            "\nFound {servers} MCP server(s) in {} file(s) for {} harness(es); {milliseconds:.2} ms",
            configs.len(),
            with_servers.len(),
        );
    } else {
        let names: Vec<&str> = harnesses.iter().map(|harness| harness.display_name()).collect();
        let names = match names.as_slice() {
            [] => "no harness".to_string(),
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
    if problems == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// What `--fix` changed, file by file, and what it couldn't fix.
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
                let new = if file.created { " (new)" } else { "" };
                format!("{}{new}", file.relative_path.display())
            })
            .collect();
        let width = labels.iter().map(|label| label.chars().count()).max().unwrap_or_default();
        let count = report.changed.len();
        out.push_str(&format!("Fixed {count} file{}:\n", if count == 1 { "" } else { "s" }));
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
        out.push('\n');
    }
    if !report.unfixed.is_empty() {
        out.push_str("Couldn't fix:\n");
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

/// Draws each server in each harness as a table, then the problems under each server.
fn draw_mcp(root: &Path, harnesses: &[Harness], checks: &[ServerCheck], problems: usize) {
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
            out.push_str(&format!(" {}{}{} │", " ".repeat(left), paint.cell(cell), " ".repeat(right)));
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
        out.push_str("No problems: every harness gets the same servers, set up the same way.\n");
        print!("{out}");
        return;
    }
    let count = if problems == 1 { "1 problem".to_string() } else { format!("{problems} problems") };
    out.push_str(&paint.bold(&count));
    out.push('\n');
    for check in checks.iter().filter(|check| !check.findings.is_empty()) {
        out.push_str(&format!("\n  {}\n", paint.bold(&check.name)));
        for finding in &check.findings {
            let symbol = match finding.rule {
                Rule::MissingServer => paint.cell(Cell::Missing),
                Rule::DifferentServer | Rule::RenamedServer => paint.cell(Cell::Different),
            };
            out.push_str(&format!("    {symbol} {}\n", finding.detail));
            let next = match &finding.fix {
                Some(fix) => fix.clone(),
                None => std::iter::once(&finding.site)
                    .chain(&finding.related)
                    .map(|site| format!("{}:{}", site.relative_path.display(), site.location.line))
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            out.push_str(&format!("      {}\n", paint.dim(&next)));
        }
    }
    print!("{out}");
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

fn list(args: &[String]) -> ExitCode {
    let mut parse_all = false;
    let mut graph = false;
    let mut roots = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--all" => parse_all = true,
            "--graph" => graph = true,
            _ => roots.push(PathBuf::from(arg)),
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from("."));
    }

    let start = Instant::now();
    let results = scan(&roots, parse_all);
    if graph {
        return print_graph(results, start);
    }

    let mut total_agents = 0;
    let mut total_prompts = 0;
    let mut parsed_files = 0;
    let mut bytes = 0;
    for result in &results {
        bytes += result.bytes;
        if !result.parsed {
            continue;
        }
        parsed_files += 1;
        if let Some(warning) = &result.warning {
            eprintln!("warning: {}: {warning}", result.path.display());
        }
        for (location, agent) in &result.agents {
            total_agents += 1;
            print_agent(&result.path, *location, agent);
        }
        for (location, prompt) in &result.prompts {
            total_prompts += 1;
            print_prompt(&result.path, *location, prompt);
        }
    }

    let elapsed = start.elapsed();
    eprintln!(
        "\nFound {total_agents} agent(s) and {total_prompts} prompt(s) in {} file(s) ({:.1} KiB); \
         parsed {parsed_files} \
         of them; {:.2} ms on {} threads",
        results.len(),
        bytes as f64 / 1024.0,
        elapsed.as_secs_f64() * 1000.0,
        rayon::current_num_threads(),
    );

    if results.is_empty() {
        eprintln!("No Python files found.");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn print_graph(results: Vec<FileResult>, start: Instant) -> ExitCode {
    let files = results.len();
    let scanned = start.elapsed();
    let graph = gglint::graph::build(results);
    let json = graph.to_json();
    let elapsed = start.elapsed();
    println!("{json}");
    eprintln!(
        "\nGraph of {} node(s) and {} edge(s) from {files} file(s); {:.2} ms, of which {:.2} ms building it",
        graph.nodes.len(),
        graph.edges.len(),
        elapsed.as_secs_f64() * 1000.0,
        (elapsed - scanned).as_secs_f64() * 1000.0,
    );
    if files == 0 {
        eprintln!("No Python files found.");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn print_agent(path: &Path, location: Location, agent: &DiscoveredAgent) {
    let kind = match agent.kind {
        AgentKind::Instance => "agent",
        AgentKind::Class => "agent class",
        AgentKind::Factory => "agent factory",
    };
    let mut details = Vec::new();
    if let Some(name) = &agent.name {
        details.push(format!("name={name:?}"));
    }
    if let Some(model) = &agent.model {
        details.push(format!("model={model:?}"));
    }
    if let Some(factory) = &agent.factory {
        details.push(format!("factory={factory:?}"));
    }
    println!(
        "{}:{}  {:<14} {:<13} via {}{}",
        path.display(),
        location,
        agent.framework,
        kind,
        agent.via,
        if details.is_empty() {
            String::new()
        } else {
            format!("  ({})", details.join(", "))
        }
    );
}

/// Prompts longer than this are cut short when printed.
const PREVIEW_CHARS: usize = 80;

fn print_prompt(path: &Path, location: Location, prompt: &DiscoveredPrompt) {
    let via = match &prompt.via {
        Some(via) if prompt.argument == "return" => format!("returned from @{via}"),
        Some(via) => format!("via {via}({}=)", prompt.argument),
        None => "via message dict".to_string(),
    };
    // One line per prompt: whitespace runs, including newlines, become one space.
    let flat = prompt.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview: String = flat.chars().take(PREVIEW_CHARS).collect();
    let ellipsis = if flat.chars().count() > PREVIEW_CHARS { "..." } else { "" };
    let mut details = Vec::new();
    if let Some(agent) = &prompt.agent {
        details.push(format!("agent={agent:?}"));
    }
    if prompt.dynamic_parts > 0 {
        details.push(format!("{} part(s) set at runtime", prompt.dynamic_parts));
    }
    for source in &prompt.sources {
        details.push(format!("from {}", source.display()));
    }
    println!(
        "{}:{}  {:<14} {:<13} {via}  {:?}{}",
        path.display(),
        location,
        prompt.framework,
        format!("{} prompt", prompt.role.as_str()),
        format!("{preview}{ellipsis}"),
        if details.is_empty() {
            String::new()
        } else {
            format!("  ({})", details.join(", "))
        }
    );
}
