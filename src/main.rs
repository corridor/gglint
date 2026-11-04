//! The `gglint` command: prints the agents found under the given directories, or the
//! lineage graph as JSON, and answers questions about them.
//!
//! ```text
//! gglint [--all] [PATH ...]           list agents and prompts
//! gglint --graph [PATH ...]           print the lineage graph as JSON
//! gglint diagram [TIER] [--json] [PATH ...]
//!                                     draw the agent graphs; TIER is `code`
//! gglint ask [FLAGS] [PATH] QUESTION  answer one question
//! gglint chat [FLAGS] [PATH]          answer each line read from standard input
//! gglint models [pull]                show, or download, the models `ask` uses
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
use gglint::views::{self, Format, Tier};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ask") => ask(&args[1..]),
        Some("chat") => chat(&args[1..]),
        Some("models") => models(&args[1..]),
        Some("diagram") => diagram(&args[1..]),
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

fn diagram(args: &[String]) -> ExitCode {
    let mut tier = Tier::Code;
    let mut format = Format::Mermaid;
    let mut roots = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "--json" => format = Format::Json,
            _ if index == 0 && Tier::parse(arg).is_some() => tier = Tier::parse(arg).expect("checked"),
            _ => roots.push(PathBuf::from(arg)),
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from("."));
    }
    if let Some(missing) = roots.iter().find(|root| !root.exists()) {
        eprintln!("error: {} doesn't exist", missing.display());
        return ExitCode::FAILURE;
    }

    let start = Instant::now();
    let results = scan(&roots, false);
    let scanned = start.elapsed();
    let diagram = views::render(&results, tier, format);
    let elapsed = start.elapsed();
    let graphs: usize = results.iter().map(|result| result.graphs.len()).sum();
    println!("{diagram}");
    eprintln!(
        "\n{} diagram of {graphs} graph(s) from {} file(s); {:.2} ms, of which {:.2} ms drawing it",
        tier.as_str(),
        results.len(),
        elapsed.as_secs_f64() * 1000.0,
        (elapsed - scanned).as_secs_f64() * 1000.0,
    );
    if graphs == 0 {
        eprintln!("No LangGraph graphs found.");
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
