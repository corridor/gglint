//! `gglint ask` on real directories: the graph built from a scan, the questions it
//! answers, and how questions are understood.

mod common;

use common::TempDir;
use gglint::ask::graph::Graph;
use gglint::ask::query::{Intent, Query, run};
use gglint::ask::understand::Understanding;
use gglint::discovery::scan;

const SUPPORT_BOT: &str = r#"from langchain.agents import create_agent

agent = create_agent(
    model="openai:gpt-4o",
    name="support",
    system_prompt="You are a support agent. Refunds over $100 need a manager's approval before you confirm them.",
)
"#;

const TRIAGE_CREW: &str = r#"from crewai import Agent

triage = Agent(
    role="triage",
    goal="Sort incoming tickets by urgency and route each one to the team that owns it.",
    backstory="You have worked the help desk for ten years.",
    llm="claude-sonnet-4-5",
)
"#;

const SUMMARISE: &str = r#"from pathlib import Path

from anthropic import Anthropic

client = Anthropic()


def summarise(ticket: str) -> str:
    client.messages.create(
        model="claude-sonnet-4-5",
        max_tokens=512,
        system=(Path(__file__).parent / "prompts" / "summarise.md").read_text(),
        messages=[{"role": "user", "content": f"Summarise ticket {ticket}."}],
    )


def summarise_again(ticket: str) -> str:
    client.messages.create(
        model="claude-sonnet-4-5",
        max_tokens=512,
        system="You summarise support tickets in three short bullet points for the on-call engineer.",
        messages=[],
    )


def summarise_once_more(ticket: str) -> str:
    client.messages.create(
        model="claude-sonnet-4-5",
        max_tokens=512,
        system="You summarise support tickets in three short bullet points for the on-call engineers.",
        messages=[],
    )
"#;

/// Two agents (`support` on gpt-4o, `triage` on Claude) and Anthropic calls whose
/// system prompts are near-duplicates, one of them read from a Markdown file.
fn fixture() -> TempDir {
    let dir = TempDir::new();
    dir.write("support/bot.py", SUPPORT_BOT);
    dir.write("crew/main.py", TRIAGE_CREW);
    dir.write("tools/summarise.py", SUMMARISE);
    dir.write(
        "tools/prompts/summarise.md",
        "You summarise support tickets in three short bullet points for the on-call engineer.",
    );
    dir
}

fn graph(dir: &TempDir) -> Graph {
    Graph::build(&scan(&[dir.path().to_path_buf()], false))
}

fn query(intent: Intent) -> Query {
    Query { intent, ..Query::default() }
}

fn no_search(_: &str) -> Vec<usize> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// The graph
// ---------------------------------------------------------------------------

#[test]
fn builds_agents_and_links_their_prompts() {
    let dir = fixture();
    let graph = graph(&dir);

    let support = graph.agents.iter().find(|agent| agent.label() == "support").unwrap();
    assert_eq!(support.path, "support/bot.py");
    assert_eq!(support.model.as_deref(), Some("openai:gpt-4o"));
    assert_eq!(support.prompts.len(), 1);
    let prompt = &graph.prompts[support.prompts[0]];
    assert!(prompt.text.starts_with("You are a support agent."));
    assert_eq!(graph.agents[prompt.agent.unwrap()].label(), "support");

    let triage = graph.agents.iter().find(|agent| agent.label() == "triage").unwrap();
    assert_eq!(triage.model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(triage.prompts.len(), 3, "role, goal and backstory");
}

#[test]
fn records_files_prompts_are_read_from() {
    let dir = fixture();
    let graph = graph(&dir);

    let from_file = graph.prompts.iter().find(|prompt| !prompt.sources.is_empty()).unwrap();
    assert_eq!(from_file.sources, vec!["tools/prompts/summarise.md"]);
}

#[test]
fn groups_near_duplicate_prompts() {
    let dir = fixture();
    let graph = graph(&dir);

    assert_eq!(graph.duplicate_groups.len(), 1);
    let lines: Vec<usize> = graph.duplicate_groups[0]
        .iter()
        .map(|&prompt| graph.prompts[prompt].line)
        .collect();
    assert_eq!(lines.len(), 3, "the file, and the two literals differing by one letter");
}

// ---------------------------------------------------------------------------
// Layer 0: answers
// ---------------------------------------------------------------------------

fn agent_index(graph: &Graph, label: &str) -> usize {
    graph.agents.iter().position(|agent| agent.label() == label).unwrap()
}

#[test]
fn answers_agents_by_model() {
    let dir = fixture();
    let graph = graph(&dir);
    let query = Query { model: Some("gpt-4o".to_string()), ..query(Intent::AgentsByModel) };

    let reply = run(&graph, &query, &no_search);

    assert_eq!(reply.text, "1 agent uses gpt-4o: support.");
    assert_eq!(reply.facts.len(), 1);
    assert_eq!((reply.facts[0].path.as_str(), reply.facts[0].line), ("support/bot.py", 3));
}

#[test]
fn answers_summary_and_lists() {
    let dir = fixture();
    let graph = graph(&dir);

    let summary = run(&graph, &query(Intent::Summary), &no_search).text;
    assert!(summary.starts_with("2 agents and "), "{summary}");
    assert!(summary.contains("crewai (1) and langchain (1)"), "{summary}");
    assert_eq!(run(&graph, &query(Intent::ListAgents), &no_search).text, "2 agents: triage and support.");
    assert_eq!(
        run(&graph, &query(Intent::ListModels), &no_search).text,
        "2 models: claude-sonnet-4-5 (1 agent) and openai:gpt-4o (1 agent)."
    );
}

#[test]
fn shows_an_agents_only_prompt_in_full() {
    let dir = fixture();
    let graph = graph(&dir);
    let query = Query { agent: Some(agent_index(&graph, "support")), ..query(Intent::AgentPrompts) };

    let reply = run(&graph, &query, &no_search);

    assert_eq!(
        reply.text,
        "support's system prompt (system_prompt=): \"You are a support agent. Refunds over $100 need a \
         manager's approval before you confirm them.\""
    );
}

#[test]
fn answers_prompt_questions() {
    let dir = fixture();
    let graph = graph(&dir);

    assert_eq!(
        run(&graph, &query(Intent::ExternalPrompts), &no_search).text,
        "1 prompt takes text from other files."
    );
    assert_eq!(
        run(&graph, &query(Intent::DuplicatePrompts), &no_search).text,
        "1 group of near-duplicate prompts (3 prompts in all)."
    );
    let dynamic = run(&graph, &query(Intent::DynamicPrompts), &no_search);
    assert_eq!(dynamic.facts.len(), 1);
    assert!(dynamic.facts[0].label.contains("Summarise ticket {ticket}."));
    assert_eq!(
        run(&graph, &query(Intent::AgentsWithoutSystemPrompt), &no_search).text,
        "Every agent created in the code has a system prompt written in it."
    );
    let file = Query { file: Some("crew/main.py".to_string()), ..query(Intent::FileContents) };
    assert_eq!(run(&graph, &file, &no_search).text, "crew/main.py has 1 agent and 3 prompts.");
}

#[test]
fn every_intent_answers_on_an_empty_repo() {
    let dir = TempDir::new();
    dir.write("app.py", "print('hello')\n");
    let graph = graph(&dir);

    for intent in Intent::ALL {
        let query = Query {
            model: Some("gpt-4o".to_string()),
            framework: Some("crewai".to_string()),
            file: Some("app.py".to_string()),
            topic: "refunds".to_string(),
            ..query(intent)
        };
        let reply = run(&graph, &query, &no_search);
        assert!(!reply.text.is_empty(), "{intent:?}");
    }
}

// ---------------------------------------------------------------------------
// Layer 1 without a model: understanding questions
// ---------------------------------------------------------------------------

fn understand(dir: &TempDir) -> (Graph, Understanding) {
    let graph = graph(dir);
    let understanding = Understanding::new(&graph, None);
    (graph, understanding)
}

#[test]
fn parses_questions_into_intents_and_names() {
    let dir = fixture();
    let (graph, understanding) = understand(&dir);

    let query = understanding.parse("which agents use gpt-4o?");
    assert_eq!((query.intent, query.model.as_deref()), (Intent::AgentsByModel, Some("gpt-4o")));

    let query = understanding.parse("list the crewai agents");
    assert_eq!((query.intent, query.framework.as_deref()), (Intent::AgentsByFramework, Some("crewai")));

    let query = understanding.parse("what's in crew/main.py");
    assert_eq!((query.intent, query.file.as_deref()), (Intent::FileContents, Some("crew/main.py")));

    let query = understanding.parse("which prompts are duplicated");
    assert_eq!(query.intent, Intent::DuplicatePrompts);

    let query = understanding.parse("tell me about triage");
    assert_eq!((query.intent, query.agent), (Intent::AgentDetails, Some(agent_index(&graph, "triage"))));
}

#[test]
fn parses_an_agent_name_with_a_typo() {
    let dir = fixture();
    let (graph, understanding) = understand(&dir);

    let query = understanding.parse("show the sytem prompt of suport");

    assert_eq!((query.intent, query.agent), (Intent::AgentPrompts, Some(agent_index(&graph, "support"))));
}

#[test]
fn unknown_agent_suggests_the_closest_names() {
    let dir = fixture();
    let (graph, understanding) = understand(&dir);

    let query = understanding.parse("tell me about the billing agent");
    let reply = run(&graph, &query, &no_search);

    assert_eq!(query.unknown_agent.as_deref(), Some("billing"));
    assert!(reply.text.starts_with("No agent named \"billing\". Closest: "), "{}", reply.text);
}

#[test]
fn topic_search_ranks_the_matching_prompt_first() {
    let dir = fixture();
    let (graph, understanding) = understand(&dir);

    let query = understanding.parse("prompts about refunds");
    let reply = run(&graph, &query, &|topic| understanding.search(topic));

    assert_eq!(query.intent, Intent::SearchPrompts);
    assert_eq!(reply.facts[0].path, "support/bot.py");
    assert!(reply.facts[0].label.contains("Refunds over $100"));
    let _ = graph;
}

// ---------------------------------------------------------------------------
// Model files
// ---------------------------------------------------------------------------

#[cfg(feature = "download")]
mod models {
    use super::TempDir;
    use gglint::ask::models::{ModelFile, ensure, local_path, models_dir, sha256, verify};

    /// One test, since `GGLINT_MODELS_DIR` is process-wide.
    #[test]
    fn cached_files_are_found_checked_and_never_downloaded_when_offline() {
        let dir = TempDir::new();
        // SAFETY: no other test in this binary reads or writes the variable.
        unsafe { std::env::set_var("GGLINT_MODELS_DIR", dir.path()) };
        assert_eq!(models_dir(), dir.path());

        let contents = b"weights";
        let file = ModelFile {
            repo: "owner/model",
            revision: "0",
            file: "model.bin",
            sha256: Box::leak(sha256(&mut &contents[..]).unwrap().into_boxed_str()),
            bytes: contents.len() as u64,
        };
        let missing = ensure(&file, false).unwrap_err();
        assert!(missing.contains("gglint models pull"), "{missing}");

        dir.write("owner__model/model.bin", contents);
        assert_eq!(ensure(&file, false).unwrap(), local_path(&file));
        assert!(verify(&file).is_ok());

        let wrong = ModelFile { sha256: "00", ..file };
        assert!(verify(&wrong).unwrap_err().contains("expected 00"));
        unsafe { std::env::remove_var("GGLINT_MODELS_DIR") };
    }
}

// ---------------------------------------------------------------------------
// Layer 1 with the embedding model, when it's downloaded
// ---------------------------------------------------------------------------

#[cfg(feature = "embed")]
#[test]
fn embeddings_understand_paraphrases() {
    use gglint::ask::embed::Embedder;
    use gglint::ask::models::{EMBED_FILES, cached};

    if !cached(&EMBED_FILES) {
        eprintln!("skipped: run `gglint models pull` first");
        return;
    }
    let dir = fixture();
    let graph = graph(&dir);
    let understanding = Understanding::new(&graph, Some(Embedder::shared(false).unwrap()));
    assert!(understanding.uses_embeddings());

    // None of these is in `intents.txt`.
    let cases = [
        ("give me a rundown of this repository", Intent::Summary),
        ("enumerate the bots defined here", Intent::ListAgents),
        ("what llms does this rely on", Intent::ListModels),
        ("are any prompts copy-pasted", Intent::DuplicatePrompts),
        ("which prompts are loaded from markdown or yaml files", Intent::ExternalPrompts),
        ("which prompts contain variables filled at runtime", Intent::DynamicPrompts),
        ("which agents lack a system prompt", Intent::AgentsWithoutSystemPrompt),
        ("what can I ask you", Intent::Help),
        ("which agent frameworks are imported", Intent::ListFrameworks),
        ("print every prompt", Intent::ListPrompts),
    ];
    let wrong: Vec<(&str, Intent)> = cases
        .iter()
        .map(|(question, _)| (*question, understanding.parse(question).intent))
        .zip(cases.iter())
        .filter(|((_, got), (_, expected))| got != expected)
        .map(|(got, _)| got)
        .collect();
    assert!(wrong.len() <= 1, "misread: {wrong:?}");

    let reply = run(&graph, &understanding.parse("prompts about giving money back"), &|topic| {
        understanding.search(topic)
    });
    assert_eq!(reply.facts[0].path, "support/bot.py", "{reply:?}");
}

#[test]
fn long_lists_are_cut_short_in_the_sentence() {
    let dir = TempDir::new();
    let agents: String = (0..30)
        .map(|n| format!("agent_{n} = create_agent(model=\"openai:gpt-4o\", name=\"bot{n:02}\")\n"))
        .collect();
    dir.write("bots.py", format!("from langchain.agents import create_agent\n\n{agents}"));
    let graph = graph(&dir);

    let reply = run(&graph, &query(Intent::ListAgents), &no_search);

    assert!(reply.text.ends_with("bot09 and 20 more."), "{}", reply.text);
    assert_eq!(reply.facts.len(), 30);
}

#[test]
fn unknown_hyphenated_agent_name_is_reported() {
    let dir = fixture();
    let (graph, understanding) = understand(&dir);

    let query = understanding.parse("show the system prompt of research-agent");
    let reply = run(&graph, &query, &no_search);

    assert_eq!(query.unknown_agent.as_deref(), Some("research-agent"));
    assert!(reply.text.starts_with("No agent named \"research-agent\"."), "{}", reply.text);
}

// ---------------------------------------------------------------------------
// Review fixes
// ---------------------------------------------------------------------------

#[test]
fn a_model_or_framework_not_in_the_code_is_reported_as_missing() {
    let dir = fixture();
    let (graph, understanding) = understand(&dir);
    let answer = |question: &str| run(&graph, &understanding.parse(question), &no_search).text;

    assert!(answer("which agents use gemini").starts_with("No agent uses gemini."), "{}", answer("which agents use gemini"));
    assert_eq!(answer("which agents are written in langgraph"), "No langgraph agents were found.");
    assert!(answer("list the agents built with mistral").starts_with("No agent uses mistral."));
}

#[test]
fn common_words_do_not_match_model_names() {
    let dir = TempDir::new();
    dir.write(
        "bot.py",
        "from langchain.agents import create_agent\n\n\
         agent = create_agent(model=\"anthropic:claude-3-5-haiku-latest\", name=\"assistant\", system_prompt=\"Help.\")\n",
    );
    let (_, understanding) = understand(&dir);

    assert_eq!(understanding.parse("show the latest prompts").model, None);
    assert_eq!(understanding.parse("which agents use haiku").model.as_deref(), Some("haiku"));
}

#[test]
fn an_apostrophe_is_not_a_quote() {
    let dir = fixture();
    let (_, understanding) = understand(&dir);

    assert_eq!(understanding.parse("what's the agent billing").unknown_agent.as_deref(), Some("billing"));
}
