//! Layer 0's answers: one function per kind of question, each a walk over the graph.
//!
//! Every answer is a sentence plus the facts behind it, each fact pointing at the line
//! it came from. Answers are written for any repository, including one with no agents
//! or prompts at all.

use std::collections::BTreeMap;

use super::graph::{AgentNode, Graph, PromptNode};

/// What a question asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Intent {
    #[default]
    Summary,
    ListAgents,
    ListPrompts,
    AgentDetails,
    AgentPrompts,
    AgentsByModel,
    AgentsByFramework,
    ListModels,
    ListFrameworks,
    DynamicPrompts,
    ExternalPrompts,
    DuplicatePrompts,
    AgentsWithoutSystemPrompt,
    FileContents,
    SearchPrompts,
    Help,
}

impl Intent {
    pub const ALL: [Intent; 16] = [
        Intent::Summary,
        Intent::ListAgents,
        Intent::ListPrompts,
        Intent::AgentDetails,
        Intent::AgentPrompts,
        Intent::AgentsByModel,
        Intent::AgentsByFramework,
        Intent::ListModels,
        Intent::ListFrameworks,
        Intent::DynamicPrompts,
        Intent::ExternalPrompts,
        Intent::DuplicatePrompts,
        Intent::AgentsWithoutSystemPrompt,
        Intent::FileContents,
        Intent::SearchPrompts,
        Intent::Help,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Intent::Summary => "summary",
            Intent::ListAgents => "list_agents",
            Intent::ListPrompts => "list_prompts",
            Intent::AgentDetails => "agent_details",
            Intent::AgentPrompts => "agent_prompts",
            Intent::AgentsByModel => "agents_by_model",
            Intent::AgentsByFramework => "agents_by_framework",
            Intent::ListModels => "list_models",
            Intent::ListFrameworks => "list_frameworks",
            Intent::DynamicPrompts => "dynamic_prompts",
            Intent::ExternalPrompts => "external_prompts",
            Intent::DuplicatePrompts => "duplicate_prompts",
            Intent::AgentsWithoutSystemPrompt => "agents_without_system_prompt",
            Intent::FileContents => "file_contents",
            Intent::SearchPrompts => "search_prompts",
            Intent::Help => "help",
        }
    }

    pub fn from_name(name: &str) -> Option<Intent> {
        Intent::ALL.into_iter().find(|intent| intent.as_str() == name)
    }
}

/// A question, understood: what it asks for and the things it names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    pub intent: Intent,
    /// Index into `Graph::agents`.
    pub agent: Option<usize>,
    /// A word in the question that looks like an agent's name but matches none.
    pub unknown_agent: Option<String>,
    pub model: Option<String>,
    pub framework: Option<String>,
    pub file: Option<String>,
    /// The question's words, for topic searches.
    pub topic: String,
}

/// One thing an answer rests on, and where it is in the code.
#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    pub path: String,
    pub line: usize,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub text: String,
    pub facts: Vec<Fact>,
}

/// How many prompts a topic search returns.
const SEARCH_LIMIT: usize = 5;
/// Prompt text is cut to this many characters in a fact.
const PREVIEW_CHARS: usize = 100;

/// Answers `query` from the graph. `search` ranks prompts for a topic, best first.
pub fn run(graph: &Graph, query: &Query, search: &dyn Fn(&str) -> Vec<usize>) -> Reply {
    match query.intent {
        Intent::Summary => summary(graph),
        Intent::ListAgents => list_agents(graph),
        Intent::ListPrompts => list_prompts(graph),
        Intent::AgentDetails => match agent(graph, query) {
            Ok(agent) => agent_details(graph, agent),
            Err(reply) => reply,
        },
        Intent::AgentPrompts => match agent(graph, query) {
            Ok(agent) => agent_prompts(graph, agent),
            Err(reply) => reply,
        },
        Intent::AgentsByModel => match &query.model {
            Some(model) => agents_by_model(graph, model),
            None => list_models(graph),
        },
        Intent::AgentsByFramework => match &query.framework {
            Some(framework) => agents_by_framework(graph, framework),
            None => list_frameworks(graph),
        },
        Intent::ListModels => list_models(graph),
        Intent::ListFrameworks => list_frameworks(graph),
        Intent::DynamicPrompts => dynamic_prompts(graph),
        Intent::ExternalPrompts => external_prompts(graph),
        Intent::DuplicatePrompts => duplicate_prompts(graph),
        Intent::AgentsWithoutSystemPrompt => agents_without_system_prompt(graph),
        Intent::FileContents => match &query.file {
            Some(file) => file_contents(graph, file),
            None => reply("Which file? Name a path, such as `app/agents.py`.".to_string(), Vec::new()),
        },
        Intent::SearchPrompts => search_prompts(graph, &query.topic, search),
        Intent::Help => help(),
    }
}

fn reply(text: String, facts: Vec<Fact>) -> Reply {
    Reply { text, facts }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

/// Names listed in a sentence before the rest are counted: the facts list them all.
const LISTED: usize = 10;

/// `a`, `a and b`, `a, b and c`, `a, b, ... j and 20 more`.
fn join(items: &[String]) -> String {
    if items.len() > LISTED {
        return format!("{} and {} more", items[..LISTED].join(", "), items.len() - LISTED);
    }
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Prompt text on one line, cut short.
pub fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= PREVIEW_CHARS {
        return flat;
    }
    let cut: String = flat.chars().take(PREVIEW_CHARS).collect();
    format!("{cut}...")
}

fn agent_fact(agent: &AgentNode) -> Fact {
    let mut details = vec![agent.framework.to_string()];
    if let Some(model) = &agent.model {
        details.push(model.clone());
    }
    Fact {
        path: agent.path.clone(),
        line: agent.line,
        label: format!("agent {} ({})", agent.label(), details.join(", ")),
    }
}

fn prompt_fact(graph: &Graph, prompt: &PromptNode) -> Fact {
    let owner = prompt
        .agent
        .map(|agent| format!(" of agent {}", graph.agents[agent].label()))
        .unwrap_or_default();
    Fact {
        path: prompt.path.clone(),
        line: prompt.line,
        label: format!("{} prompt{owner} ({}): \"{}\"", prompt.role, prompt.framework, preview(&prompt.text)),
    }
}

/// The agent a query names, or a reply saying it wasn't found.
fn agent<'g>(graph: &'g Graph, query: &Query) -> Result<&'g AgentNode, Reply> {
    if let Some(index) = query.agent {
        return Ok(&graph.agents[index]);
    }
    let labels: Vec<String> = graph.agents.iter().map(AgentNode::label).collect();
    if graph.agents.is_empty() {
        return Err(reply("No agents were found in this code.".to_string(), Vec::new()));
    }
    match &query.unknown_agent {
        Some(word) => {
            let mut closest: Vec<(f64, &String)> = labels
                .iter()
                .map(|label| (strsim::jaro_winkler(&word.to_lowercase(), &label.to_lowercase()), label))
                .collect();
            closest.sort_by(|a, b| b.0.total_cmp(&a.0));
            let closest: Vec<String> = closest.into_iter().take(3).map(|(_, label)| label.clone()).collect();
            Err(reply(
                format!("No agent named \"{word}\". Closest: {}.", closest.join(", ")),
                Vec::new(),
            ))
        }
        None => Err(reply(
            format!("Which agent? There are {}: {}.", count(labels.len(), "agent"), join(&labels)),
            graph.agents.iter().map(agent_fact).collect(),
        )),
    }
}

fn summary(graph: &Graph) -> Reply {
    let mut frameworks: BTreeMap<&str, usize> = BTreeMap::new();
    for agent in &graph.agents {
        *frameworks.entry(agent.framework).or_default() += 1;
    }
    let mut text = format!(
        "{} and {} in {}.",
        count(graph.agents.len(), "agent"),
        count(graph.prompts.len(), "prompt"),
        count(graph.files, "Python file"),
    );
    if !frameworks.is_empty() {
        let frameworks: Vec<String> = frameworks
            .iter()
            .map(|(framework, n)| format!("{framework} ({n})"))
            .collect();
        text.push_str(&format!(" Agent frameworks: {}.", join(&frameworks)));
    }
    let models = models(graph);
    if !models.is_empty() {
        let models: Vec<String> = models.keys().map(|model| model.to_string()).collect();
        text.push_str(&format!(" Models: {}.", join(&models)));
    }
    reply(text, graph.agents.iter().map(agent_fact).collect())
}

fn list_agents(graph: &Graph) -> Reply {
    if graph.agents.is_empty() {
        return reply("No agents were found in this code.".to_string(), Vec::new());
    }
    let labels: Vec<String> = graph.agents.iter().map(AgentNode::label).collect();
    reply(
        format!("{}: {}.", count(labels.len(), "agent"), join(&labels)),
        graph.agents.iter().map(agent_fact).collect(),
    )
}

fn list_prompts(graph: &Graph) -> Reply {
    if graph.prompts.is_empty() {
        return reply("No prompts were found in this code.".to_string(), Vec::new());
    }
    let mut roles: BTreeMap<&str, usize> = BTreeMap::new();
    for prompt in &graph.prompts {
        *roles.entry(prompt.role).or_default() += 1;
    }
    let roles: Vec<String> = roles.iter().map(|(role, n)| format!("{n} {role}")).collect();
    reply(
        format!("{}: {}.", count(graph.prompts.len(), "prompt"), join(&roles)),
        graph.prompts.iter().map(|prompt| prompt_fact(graph, prompt)).collect(),
    )
}

fn agent_details(graph: &Graph, agent: &AgentNode) -> Reply {
    let model = agent
        .model
        .as_ref()
        .map(|model| format!(" using {model}"))
        .unwrap_or_else(|| " (its model isn't written in the code)".to_string());
    let text = format!(
        "{} is a {} {}{model}, defined at {}, with {}.",
        agent.label(),
        agent.framework,
        agent.kind.replace('_', " "),
        agent.location(),
        count(agent.prompts.len(), "prompt"),
    );
    let mut facts = vec![agent_fact(agent)];
    facts.extend(agent.prompts.iter().map(|&prompt| prompt_fact(graph, &graph.prompts[prompt])));
    reply(text, facts)
}

fn agent_prompts(graph: &Graph, agent: &AgentNode) -> Reply {
    let facts: Vec<Fact> = agent
        .prompts
        .iter()
        .map(|&prompt| prompt_fact(graph, &graph.prompts[prompt]))
        .collect();
    let text = match agent.prompts.as_slice() {
        [] => format!("{} has no prompt written in the code.", agent.label()),
        [only] => {
            let prompt = &graph.prompts[*only];
            format!("{}'s {} prompt ({}=): \"{}\"", agent.label(), prompt.role, prompt.argument, prompt.text.trim())
        }
        prompts => format!("{} has {}.", agent.label(), count(prompts.len(), "prompt")),
    };
    reply(text, facts)
}

/// Models written in the code, with how many agents use each.
fn models(graph: &Graph) -> BTreeMap<&str, usize> {
    let mut models = BTreeMap::new();
    for agent in &graph.agents {
        if let Some(model) = &agent.model {
            *models.entry(model.as_str()).or_default() += 1;
        }
    }
    models
}

fn matches(value: &str, wanted: &str) -> bool {
    value.to_lowercase().contains(&wanted.to_lowercase())
}

fn agents_by_model(graph: &Graph, model: &str) -> Reply {
    let agents: Vec<&AgentNode> = graph
        .agents
        .iter()
        .filter(|agent| agent.model.as_deref().is_some_and(|value| matches(value, model)))
        .collect();
    if agents.is_empty() {
        let known: Vec<String> = models(graph).keys().map(|model| model.to_string()).collect();
        let known = if known.is_empty() {
            "No agent's model is written in the code.".to_string()
        } else {
            format!("Models in the code: {}.", join(&known))
        };
        return reply(format!("No agent uses {model}. {known}"), Vec::new());
    }
    let labels: Vec<String> = agents.iter().map(|agent| agent.label()).collect();
    let verb = if agents.len() == 1 { "uses" } else { "use" };
    reply(
        format!("{} {verb} {model}: {}.", count(agents.len(), "agent"), join(&labels)),
        agents.into_iter().map(agent_fact).collect(),
    )
}

fn agents_by_framework(graph: &Graph, framework: &str) -> Reply {
    let agents: Vec<&AgentNode> = graph
        .agents
        .iter()
        .filter(|agent| matches(agent.framework, framework))
        .collect();
    if agents.is_empty() {
        return reply(format!("No {framework} agents were found."), Vec::new());
    }
    let labels: Vec<String> = agents.iter().map(|agent| agent.label()).collect();
    reply(
        format!("{}: {}.", count(agents.len(), &format!("{framework} agent")), join(&labels)),
        agents.into_iter().map(agent_fact).collect(),
    )
}

fn list_models(graph: &Graph) -> Reply {
    let models = models(graph);
    if models.is_empty() {
        return reply("No agent's model is written in the code.".to_string(), Vec::new());
    }
    let listed: Vec<String> = models
        .iter()
        .map(|(model, n)| format!("{model} ({})", count(*n, "agent")))
        .collect();
    reply(
        format!("{}: {}.", count(models.len(), "model"), join(&listed)),
        graph.agents.iter().filter(|agent| agent.model.is_some()).map(agent_fact).collect(),
    )
}

fn list_frameworks(graph: &Graph) -> Reply {
    let mut frameworks: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for agent in &graph.agents {
        frameworks.entry(agent.framework).or_default().0 += 1;
    }
    for prompt in &graph.prompts {
        frameworks.entry(prompt.framework).or_default().1 += 1;
    }
    if frameworks.is_empty() {
        return reply("No agent framework or model SDK is used in this code.".to_string(), Vec::new());
    }
    let listed: Vec<String> = frameworks
        .iter()
        .map(|(framework, (agents, prompts))| {
            format!("{framework} ({}, {})", count(*agents, "agent"), count(*prompts, "prompt"))
        })
        .collect();
    reply(format!("{} in use: {}.", count(frameworks.len(), "framework"), join(&listed)), Vec::new())
}

fn prompts_where(graph: &Graph, keep: impl Fn(&PromptNode) -> bool) -> Vec<Fact> {
    graph
        .prompts
        .iter()
        .filter(|prompt| keep(prompt))
        .map(|prompt| prompt_fact(graph, prompt))
        .collect()
}

fn dynamic_prompts(graph: &Graph) -> Reply {
    let facts = prompts_where(graph, |prompt| prompt.dynamic_parts > 0);
    let text = if facts.is_empty() {
        "Every prompt's text is fully written in the code.".to_string()
    } else {
        format!("{} with parts only known at runtime, shown as {{expression}}.", count(facts.len(), "prompt"))
    };
    reply(text, facts)
}

fn external_prompts(graph: &Graph) -> Reply {
    let facts: Vec<Fact> = graph
        .prompts
        .iter()
        .filter(|prompt| !prompt.sources.is_empty())
        .map(|prompt| {
            let mut fact = prompt_fact(graph, prompt);
            fact.label = format!("{} (from {})", fact.label, prompt.sources.join(", "));
            fact
        })
        .collect();
    let text = if facts.is_empty() {
        "Every prompt is written where it's used; none comes from another file.".to_string()
    } else {
        let verb = if facts.len() == 1 { "takes" } else { "take" };
        format!("{} {verb} text from other files.", count(facts.len(), "prompt"))
    };
    reply(text, facts)
}

fn duplicate_prompts(graph: &Graph) -> Reply {
    if graph.duplicate_groups.is_empty() {
        return reply("No near-duplicate prompts were found.".to_string(), Vec::new());
    }
    let mut facts = Vec::new();
    for (number, group) in graph.duplicate_groups.iter().enumerate() {
        for &prompt in group {
            let mut fact = prompt_fact(graph, &graph.prompts[prompt]);
            fact.label = format!("group {}: {}", number + 1, fact.label);
            facts.push(fact);
        }
    }
    let prompts: usize = graph.duplicate_groups.iter().map(Vec::len).sum();
    reply(
        format!(
            "{} of near-duplicate prompts ({} in all).",
            count(graph.duplicate_groups.len(), "group"),
            count(prompts, "prompt"),
        ),
        facts,
    )
}

fn agents_without_system_prompt(graph: &Graph) -> Reply {
    let agents: Vec<&AgentNode> = graph
        .agents
        .iter()
        .filter(|agent| {
            agent.kind == "agent" && !agent.prompts.iter().any(|&prompt| graph.prompts[prompt].role == "system")
        })
        .collect();
    if agents.is_empty() {
        return reply("Every agent created in the code has a system prompt written in it.".to_string(), Vec::new());
    }
    let labels: Vec<String> = agents.iter().map(|agent| agent.label()).collect();
    let verb = if agents.len() == 1 { "has" } else { "have" };
    reply(
        format!("{} {verb} no system prompt written in the code: {}.", count(agents.len(), "agent"), join(&labels)),
        agents.into_iter().map(agent_fact).collect(),
    )
}

fn file_contents(graph: &Graph, file: &str) -> Reply {
    let in_file = |path: &str| path == file || path.ends_with(&format!("/{file}")) || path.contains(file);
    let mut facts: Vec<Fact> = graph.agents.iter().filter(|agent| in_file(&agent.path)).map(agent_fact).collect();
    let agents = facts.len();
    facts.extend(prompts_where(graph, |prompt| in_file(&prompt.path)));
    let prompts = facts.len() - agents;
    let text = if facts.is_empty() {
        format!("No agents or prompts were found in {file}.")
    } else {
        format!("{file} has {} and {}.", count(agents, "agent"), count(prompts, "prompt"))
    };
    reply(text, facts)
}

fn search_prompts(graph: &Graph, topic: &str, search: &dyn Fn(&str) -> Vec<usize>) -> Reply {
    let found: Vec<usize> = search(topic).into_iter().take(SEARCH_LIMIT).collect();
    if found.is_empty() {
        return reply(format!("No prompt mentions \"{}\".", topic.trim()), Vec::new());
    }
    reply(
        format!("{} about \"{}\", best match first.", count(found.len(), "prompt"), topic.trim()),
        found.into_iter().map(|prompt| prompt_fact(graph, &graph.prompts[prompt])).collect(),
    )
}

fn help() -> Reply {
    reply(
        "Ask about the agents and prompts in this code, for example: \"what's in this repo\", \
         \"list the agents\", \"which agents use gpt-4o\", \"show the system prompt of support\", \
         \"which prompts are duplicated\", \"which prompts are read from files\", \
         \"which agents have no system prompt\", \"what's in app/bot.py\" or \"prompts about refunds\"."
            .to_string(),
        Vec::new(),
    )
}
