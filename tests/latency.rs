//! Latency budgets for `gglint ask`, on a generated repository with 2,000 prompts.
//!
//! Timing needs an optimized build and the machine to itself, so these are ignored by
//! default and run one at a time:
//!
//! ```sh
//! cargo test --release --test latency -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The embedding model test is skipped when the model isn't downloaded (`gglint models pull`).

mod common;

use std::time::{Duration, Instant};

use common::TempDir;
use gglint::ask::graph::Graph;
use gglint::ask::query::{Intent, run};
use gglint::ask::understand::Understanding;
#[cfg(feature = "embed")]
use gglint::ask::{Options, Repo};
use gglint::discovery::scan;
use gglint::harness::read_mcp;

const QUESTIONS: &[&str] = &[
    "what's in this repo",
    "list the agents",
    "which agents use gpt-4o",
    "tell me about bot0420",
    "show the system prompt of bot0007",
    "which prompts are duplicated",
    "which prompts are read from files",
    "which agents have no system prompt",
    "what's in team_07/bots.py",
    "prompts about refunds over a hundred dollars",
    "which models are used",
    "list the langchain agents",
];

/// 20 files of 50 agents, each with a system prompt, and 1,000 more prompts sent to the
/// Anthropic API: 2,000 prompts.
fn large_repo() -> TempDir {
    let dir = TempDir::new();
    let topics = ["refunds", "shipping", "invoices", "passwords", "outages"];
    for team in 0..20 {
        let mut source = String::from("from langchain.agents import create_agent\nfrom anthropic import Anthropic\n\nclient = Anthropic()\n\n");
        for n in 0..50 {
            let id = team * 50 + n;
            let topic = topics[id % topics.len()];
            let model = if id % 3 == 0 { "openai:gpt-4o" } else { "claude-sonnet-4-5" };
            source.push_str(&format!(
                "agent_{id} = create_agent(model=\"{model}\", name=\"bot{id:04}\", \
                 system_prompt=\"You are bot {id}. You help customers with {topic} for team {team}.\")\n\
                 client.messages.create(model=\"x\", max_tokens=1, system=\"Summarise {topic} case {id} in two lines.\", messages=[])\n"
            ));
        }
        dir.write(&format!("team_{team:02}/bots.py"), source);
    }
    dir
}

fn slowest(times: impl IntoIterator<Item = Duration>) -> Duration {
    times.into_iter().max().unwrap_or_default()
}

#[test]
#[ignore = "timing; run with --release"]
fn layer_0_and_1_answer_within_budget() {
    let dir = large_repo();
    let graph = Graph::build(&scan(&[dir.path().to_path_buf()], false));
    assert_eq!(graph.prompts.len(), 2000);
    let understanding = Understanding::new(&graph, None);

    let queries: Vec<_> = QUESTIONS.iter().map(|question| understanding.parse(question)).collect();
    // Topic searches rank prompts with BM25, which is Layer 1's work.
    let layer_0 = slowest(queries.iter().filter(|query| query.intent != Intent::SearchPrompts).map(|query| {
        let start = Instant::now();
        run(&graph, query, &|topic| understanding.search(topic));
        start.elapsed()
    }));
    let layer_1 = slowest(QUESTIONS.iter().map(|question| {
        let start = Instant::now();
        let query = understanding.parse(question);
        run(&graph, &query, &|topic| understanding.search(topic));
        start.elapsed()
    }));
    eprintln!("slowest: layer 0 {layer_0:?}, layers 0+1 {layer_1:?}");
    assert!(layer_0 < Duration::from_millis(1), "layer 0: {layer_0:?}");
    assert!(layer_1 < Duration::from_millis(10), "layers 0+1: {layer_1:?}");
}

#[cfg(feature = "embed")]
#[test]
#[ignore = "timing; run with --release"]
fn answers_with_the_embedding_model_within_budget() {
    use gglint::ask::models::{EMBED_FILES, cached};

    if !cached(&EMBED_FILES) {
        eprintln!("skipped: run `gglint models pull` first");
        return;
    }
    let dir = large_repo();
    let start = Instant::now();
    let repo = Repo::open(&[dir.path().to_path_buf()], &Options { download: false, ..Options::default() });
    eprintln!("opened in {:?}", start.elapsed());
    assert!(repo.warnings.is_empty(), "{:?}", repo.warnings);

    let times: Vec<Duration> = QUESTIONS
        .iter()
        .map(|question| {
            let answer = repo.ask(question);
            assert_eq!(answer.layer, 1);
            eprintln!("{:>8.2} ms  {:<28} {}", answer.elapsed.as_secs_f64() * 1000.0, answer.intent, answer.text);
            answer.elapsed
        })
        .collect();
    let slowest = slowest(times);
    assert!(slowest < Duration::from_millis(10), "slowest answer: {slowest:?}");
}

/// Every harness's MCP config, as in a repository set up for all of them.
fn mcp_repo() -> TempDir {
    let dir = TempDir::new();
    let mut json = String::from("{\n  \"mcpServers\": {\n");
    let mut opencode = String::from("{\n  \"mcp\": {\n");
    let mut toml = String::new();
    for n in 0..20 {
        opencode.push_str(&format!(
            "    \"server{n}\": {{ \"type\": \"local\", \"command\": [\"npx\", \"-y\", \"server{n}@1.0.0\"], \"environment\": {{ \"KEY\": \"{{env:KEY}}\" }} }},\n"
        ));
        json.push_str(&format!(
            "    \"server{n}\": {{ \"command\": \"npx\", \"args\": [\"-y\", \"server{n}@1.0.0\"], \"env\": {{ \"KEY\": \"${{KEY}}\" }} }},\n"
        ));
        toml.push_str(&format!("[mcp_servers.server{n}]\ncommand = \"npx\"\nargs = [\"-y\", \"server{n}@1.0.0\"]\nenv_vars = [\"KEY\"]\n\n"));
    }
    let docs = "    \"docs\": { \"url\": \"https://docs.example.com/mcp\" }\n  }\n}\n";
    json.push_str(docs);
    opencode.push_str(docs);
    dir.write(".mcp.json", &json);
    dir.write(".cursor/mcp.json", &json);
    dir.write(".codex/config.toml", toml);
    dir.write("opencode.json", opencode);
    dir
}

#[test]
#[ignore = "timing; run with --release"]
fn reads_mcp_configs_within_budget() {
    let dir = mcp_repo();
    assert_eq!(read_mcp(dir.path()).len(), 4);

    let times: Vec<Duration> = (0..200)
        .map(|_| {
            let start = Instant::now();
            let configs = read_mcp(dir.path());
            let elapsed = start.elapsed();
            assert_eq!(configs.iter().map(|config| config.servers.len()).sum::<usize>(), 83);
            elapsed
        })
        .collect();
    let slowest = slowest(times);
    eprintln!("slowest read of 4 files, 83 servers: {slowest:?}");
    assert!(slowest < Duration::from_millis(1), "slowest read: {slowest:?}");
}
