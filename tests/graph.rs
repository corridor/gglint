//! The lineage graph built from real directories: which sources feed which prompts,
//! and which agents the prompts are given to.

mod common;

use common::TempDir;
use gglint::discovery::scan;
use gglint::graph::{EdgeKind, Graph, NodeData, build};

fn graph(dir: &TempDir) -> Graph {
    build(scan(&[dir.path().to_path_buf()], false))
}

/// Each edge as `(source id, target id, label)`, in order.
fn edges(graph: &Graph) -> Vec<(String, String, String)> {
    graph
        .edges
        .iter()
        .map(|edge| {
            let label = match edge.kind {
                EdgeKind::Feeds { position } => format!("feeds {position}"),
                EdgeKind::GivenTo(link) => format!("given_to {}", link.as_str()),
            };
            (
                graph.nodes[edge.from as usize].id.clone(),
                graph.nodes[edge.to as usize].id.clone(),
                label,
            )
        })
        .collect()
}

fn edge(from: &str, to: &str, label: &str) -> (String, String, String) {
    (from.to_string(), to.to_string(), label.to_string())
}

fn source_text<'g>(graph: &'g Graph, id: &str) -> &'g str {
    let node = graph.nodes.iter().find(|node| node.id == id).unwrap_or_else(|| panic!("no node {id}"));
    match &node.data {
        NodeData::Source(source) => &source.text,
        _ => panic!("{id} isn't a source"),
    }
}

#[test]
fn a_shared_constant_is_one_node_feeding_every_agent_that_uses_it() {
    let dir = TempDir::new();
    dir.write("app/__init__.py", "");
    dir.write("app/prompts.py", "TONE = \"Be brief.\"\n");
    dir.write(
        "app/a.py",
        "from pydantic_ai import Agent\n\
         from app.prompts import TONE\n\
         \n\
         a = Agent(\"openai:gpt-4o\", instructions=f\"{TONE} You are A.\")\n\
         a.run_sync(\"Summarise X\")\n",
    );
    dir.write(
        "app/b.py",
        "from pydantic_ai import Agent\n\
         from .prompts import TONE\n\
         \n\
         b = Agent(\"openai:gpt-4o\", system_prompt=TONE + \" You are B.\")\n",
    );

    let graph = graph(&dir);

    assert_eq!(
        edges(&graph),
        vec![
            edge("prompt:app/a.py:4:41", "agent:app.a.a", "given_to constructor"),
            edge("constant:app.prompts.TONE", "prompt:app/a.py:4:41", "feeds 0"),
            edge("prompt:app/a.py:5:12", "agent:app.a.a", "given_to method"),
            edge("prompt:app/b.py:4:42", "agent:app.b.b", "given_to constructor"),
            edge("constant:app.prompts.TONE", "prompt:app/b.py:4:42", "feeds 0"),
        ]
    );
    assert_eq!(source_text(&graph, "constant:app.prompts.TONE"), "Be brief.");
    let tone = graph.nodes.iter().find(|node| node.id == "constant:app.prompts.TONE").unwrap();
    assert_eq!(&*tone.path, "app/prompts.py");
    assert_eq!(tone.location.map(|location| (location.line, location.column)), Some((1, 1)));
}

#[test]
fn sources_built_from_other_sources_keep_them_in_order() {
    let dir = TempDir::new();
    dir.write(
        "bot.py",
        "from pydantic_ai import Agent\n\
         BASE = \"You review code.\"\n\
         STYLE = \"Be kind.\"\n\
         RULES = BASE + \" \" + STYLE\n\
         def today():\n    return \"Today is Monday.\"\n\
         agent = Agent(\"x\", instructions=f\"{RULES} {today()}\")\n",
    );

    let graph = graph(&dir);

    assert_eq!(
        edges(&graph),
        vec![
            edge("prompt:bot.py:7:33", "agent:bot.agent", "given_to constructor"),
            edge("constant:bot.RULES", "prompt:bot.py:7:33", "feeds 0"),
            edge("constant:bot.BASE", "constant:bot.RULES", "feeds 0"),
            edge("constant:bot.STYLE", "constant:bot.RULES", "feeds 1"),
            edge("function:bot.today", "prompt:bot.py:7:33", "feeds 1"),
        ]
    );
    assert_eq!(source_text(&graph, "constant:bot.RULES"), "You review code. Be kind.");
    assert_eq!(source_text(&graph, "function:bot.today"), "Today is Monday.");
}

#[test]
fn files_and_crewai_config_fields_are_sources() {
    let dir = TempDir::new();
    dir.write("app/system.md", "You summarise tickets.");
    dir.write("app/config/agents.yaml", "writer:\n  role: Writer\n  goal: Write {topic}\n");
    dir.write(
        "app/bots.py",
        "from pathlib import Path\n\
         from pydantic_ai import Agent\n\
         from crewai import Agent as CrewAgent\n\
         \n\
         summary = Agent(\"x\", instructions=(Path(__file__).parent / \"system.md\").read_text())\n\
         \n\
         class Crew:\n    def writer(self):\n        return CrewAgent(config=self.agents_config[\"writer\"])\n",
    );

    let graph = graph(&dir);

    let found = edges(&graph);
    assert!(found.contains(&edge("file:app/system.md", "prompt:app/bots.py:5:35", "feeds 0")), "{found:#?}");
    assert!(found.contains(&edge("config:app/config/agents.yaml#writer.role", "prompt:app/bots.py:9:33", "feeds 0")), "{found:#?}");
    assert!(found.contains(&edge("config:app/config/agents.yaml#writer.goal", "prompt:app/bots.py:9:33#2", "feeds 0")), "{found:#?}");
    assert_eq!(source_text(&graph, "file:app/system.md"), "You summarise tickets.");
    assert_eq!(source_text(&graph, "config:app/config/agents.yaml#writer.goal"), "Write {topic}");
}

#[test]
fn prompts_reach_agents_through_runners_tasks_decorators_and_prompt_objects() {
    let dir = TempDir::new();
    dir.write(
        "runner.py",
        "from agents import Agent, Runner\n\
         helper = Agent(name=\"helper\", instructions=\"Help.\")\n\
         Runner.run_sync(helper, \"Hi there\")\n",
    );
    dir.write(
        "decorated.py",
        "from pydantic_ai import Agent\n\
         bot = Agent(\"x\")\n\
         @bot.instructions\n\
         def rules():\n    return \"Be precise.\"\n",
    );
    dir.write(
        "crew.py",
        "from crewai import Agent, Task\n\
         from crewai.project import agent, task\n\
         class Crew:\n\
         \x20   @agent\n\
         \x20   def researcher(self):\n        return Agent(role=\"Researcher\", goal=\"Find\", backstory=\"Curious\")\n\
         \x20   @task\n\
         \x20   def research(self):\n        return Task(description=\"Research it\", expected_output=\"Notes\", agent=self.researcher())\n",
    );
    dir.write(
        "react.py",
        "from langgraph.prebuilt import create_react_agent\n\
         from langchain_core.messages import SystemMessage\n\
         graph = create_react_agent(\"x\", tools=[], prompt=SystemMessage(\"Be terse.\"))\n",
    );

    let graph = graph(&dir);

    let found = edges(&graph);
    for expected in [
        edge("prompt:runner.py:3:25", "agent:runner.helper", "given_to argument"),
        edge("prompt:decorated.py:5:12", "agent:decorated.bot", "given_to decorator"),
        edge("prompt:crew.py:9:33", "agent:crew.researcher", "given_to argument"),
        edge("prompt:react.py:3:64", "agent:react.graph", "given_to nested"),
    ] {
        assert!(found.contains(&expected), "missing {expected:?} in {found:#?}");
    }
}

#[test]
fn agents_imported_from_other_modules_are_linked_by_variable() {
    let dir = TempDir::new();
    dir.write("app/__init__.py", "");
    dir.write("app/bots.py", "from pydantic_ai import Agent\nsupport = Agent(\"x\")\n");
    dir.write(
        "app/run.py",
        "from pydantic_ai import Agent\n\
         from app.bots import support\n\
         from app import bots\n\
         support.run_sync(\"Where is my order?\")\n\
         bots.support.run_sync(\"Cancel it.\")\n",
    );

    let graph = graph(&dir);

    assert_eq!(
        edges(&graph),
        vec![
            edge("prompt:app/run.py:4:18", "agent:app.bots.support", "given_to method"),
            edge("prompt:app/run.py:5:23", "agent:app.bots.support", "given_to method"),
        ]
    );
}

#[test]
fn json_is_node_link_with_escaped_text() {
    let dir = TempDir::new();
    dir.write(
        "bot.py",
        "from pydantic_ai import Agent\n\
         PROMPT = \"Say \\\"hi\\\"\\n\\tthen\\\\stop\\x01\"\n\
         agent = Agent(\"x\", instructions=PROMPT)\n",
    );

    let json = graph(&dir).to_json();

    assert!(json.starts_with("{\"directed\":true,\"multigraph\":false,\"graph\":{\"schema\":1},\"nodes\":["), "{json}");
    assert!(json.contains("\"text\":\"Say \\\"hi\\\"\\n\\tthen\\\\stop\\u0001\""), "{json}");
    assert!(
        json.contains(
            "{\"source\":\"constant:bot.PROMPT\",\"target\":\"prompt:bot.py:3:33\",\"kind\":\"feeds\",\"position\":0}"
        ),
        "{json}"
    );
    assert!(json.ends_with("]}"), "{json}");
}

#[test]
fn methods_of_imported_names_that_are_not_agents_are_not_prompts() {
    let dir = TempDir::new();
    dir.write("app/__init__.py", "");
    dir.write("app/util.py", "class Job:\n    def run_sync(self, text): ...\njob = Job()\n");
    dir.write(
        "app/run.py",
        "from pydantic_ai import Agent\n\
         from app.util import job\n\
         job.run_sync(\"Not a prompt\")\n",
    );

    let results = scan(&[dir.path().to_path_buf()], false);

    assert!(results.iter().all(|result| result.prompts.is_empty()));
    assert!(build(results).nodes.is_empty());
}

#[test]
fn an_agent_returned_by_a_factory_is_one_agent() {
    let dir = TempDir::new();
    dir.write("crew/config/agents.yaml", "researcher:\n  role: Researcher\n");
    dir.write(
        "crew/crew.py",
        "from crewai import Agent, Crew, Task\n\
         from crewai.project import CrewBase, agent, crew, task\n\
         @CrewBase\n\
         class ResearchCrew:\n\
         \x20   @agent\n\
         \x20   def researcher(self):\n        return Agent(config=self.agents_config[\"researcher\"])\n\
         \x20   @agent\n\
         \x20   def writer(self):\n        a = Agent(role=\"Writer\", goal=\"Write\", backstory=\"b\")\n        return a\n\
         \x20   @task\n\
         \x20   def research(self):\n        return Task(description=\"Research it\", agent=self.researcher())\n\
         \x20   @crew\n\
         \x20   def crew(self):\n        return Crew(agents=self.agents, tasks=self.tasks)\n",
    );

    let results = scan(&[dir.path().to_path_buf()], false);
    let agents: Vec<_> = results[0]
        .agents
        .iter()
        .map(|(_, agent)| (agent.kind.as_str(), agent.via.as_str(), agent.factory.as_deref()))
        .collect();
    assert_eq!(
        agents,
        vec![
            ("agent", "crewai.Agent", Some("researcher")),
            ("agent", "crewai.Agent", Some("writer")),
            ("agent", "crewai.Crew", Some("crew")),
        ]
    );

    let graph = build(results);
    let found = edges(&graph);
    // The config's role and the task both reach the one researcher.
    assert!(found.contains(&edge("prompt:crew/crew.py:7:29", "agent:crew.crew.researcher", "given_to constructor")), "{found:#?}");
    assert!(found.contains(&edge("prompt:crew/crew.py:14:33", "agent:crew.crew.researcher", "given_to argument")), "{found:#?}");
}
