//! The `gglint` command: prints the agents found under the given directories.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use gglint::discovery::{AgentKind, DiscoveredAgent, DiscoveredPrompt, Location, scan};

fn main() -> ExitCode {
    let mut parse_all = false;
    let mut roots = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--all" => parse_all = true,
            _ => roots.push(PathBuf::from(arg)),
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from("."));
    }

    let start = Instant::now();
    let results = scan(&roots, parse_all);

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
