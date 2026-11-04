//! Diagrams of LangGraph graphs read from real directories.

mod common;

use common::TempDir;
use gglint::discovery::{FileResult, scan};
use gglint::flow::{Node, Step};
use gglint::views::{self, Format, Tier};

fn results(dir: &TempDir) -> Vec<FileResult> {
    scan(&[dir.path().to_path_buf()], false)
}

fn node<'r>(results: &'r [FileResult], name: &str) -> &'r Node {
    results
        .iter()
        .flat_map(|result| &result.graphs)
        .flat_map(|graph| &graph.nodes)
        .find(|node| node.name == name)
        .unwrap_or_else(|| panic!("no node {name}"))
}

/// A graph whose node calls a model made in another module, a project function and a
/// library function.
fn triage_project(dir: &TempDir) {
    dir.write(
        "app/models.py",
        "from langchain_aws import ChatBedrockConverse\n\
         \n\
         def claude(thinking='none'):\n\
         \x20   llm = ChatBedrockConverse(model_id='us.anthropic.claude-sonnet-5-5')\n\
         \x20   return llm\n",
    );
    dir.write("app/rag.py", "def protocol(name):\n    return open(name).read()\n");
    dir.write(
        "app/flow.py",
        "import json\n\
         from langgraph.graph import StateGraph, START, END\n\
         from app.models import claude\n\
         from app.rag import protocol\n\
         \n\
         def triage(state):\n\
         \x20   llm = claude().with_structured_output(Decision)\n\
         \x20   decision = llm.invoke(protocol(state['protocol']))\n\
         \x20   return {'disposition': json.dumps(decision)}\n\
         \n\
         def route(state):\n\
         \x20   if state['disposition'] == 'EMERGENCY':\n\
         \x20       return 'escalate'\n\
         \x20   return END\n\
         \n\
         def escalate(state):\n\
         \x20   return {}\n\
         \n\
         workflow = StateGraph(State)\n\
         workflow.add_node('triage', triage)\n\
         workflow.add_node('escalate', escalate)\n\
         workflow.add_edge(START, 'triage')\n\
         workflow.add_conditional_edges('triage', route, {'escalate': 'escalate', 'done': END})\n\
         workflow.add_edge('escalate', END)\n\
         graph = workflow.compile()\n",
    );
}

#[test]
fn a_model_made_in_another_module_is_looked_up_and_only_project_calls_are_kept() {
    let dir = TempDir::new();
    triage_project(&dir);
    let results = results(&dir);

    let triage = node(&results, "triage");
    let [Step::Model(call), Step::Call { name, .. }] = triage.steps.as_slice() else {
        panic!("{:?}", triage.steps);
    };
    assert_eq!(call.model.id.as_deref(), Some("us.anthropic.claude-sonnet-5-5"));
    assert_eq!(call.model.client.as_deref(), Some("langchain_aws.ChatBedrockConverse"));
    assert_eq!(call.model.factory.as_deref(), Some("app.models.claude"));
    assert_eq!(call.output.as_deref(), Some("Decision"));
    // `json.dumps` is a library's, and `claude` is the model's.
    assert_eq!(name, "app.rag.protocol");
    assert_eq!(triage.reads, ["protocol"]);
    assert_eq!(triage.writes, ["disposition"]);
}

#[test]
fn a_call_that_is_not_known_to_reach_a_model_is_not_a_model_call() {
    let dir = TempDir::new();
    dir.write("app/runner.py", "def build():\n    return object()\n");
    dir.write(
        "app/flow.py",
        "from langgraph.graph import StateGraph\n\
         from app.runner import build\n\
         def run(state):\n\
         \x20   return build().invoke(state)\n\
         g = StateGraph(dict)\n\
         g.add_node('run', run)\n",
    );
    let results = results(&dir);
    assert_eq!(
        node(&results, "run").steps,
        [Step::Call {
            name: "app.runner.build".to_string(),
            location: gglint::discovery::Location { line: 4, column: 12 },
        }]
    );
}

#[test]
fn draws_the_graph_as_langgraph_does() {
    let dir = TempDir::new();
    triage_project(&dir);

    let mermaid = views::render(&results(&dir), Tier::Code, Format::Mermaid);

    assert_eq!(
        mermaid,
        "---\nconfig:\n  flowchart:\n    curve: linear\n---\ngraph TD;\n\
         \t__start__([<p>__start__</p>]):::first\n\
         \ttriage(\"triage<hr/><small><em>model: us.anthropic.claude-sonnet-5-5 → Decision<br>calls: protocol</em></small>\")\n\
         \tescalate(\"escalate\")\n\
         \t__end__([<p>__end__</p>]):::last\n\
         \t__start__ --> triage;\n\
         \ttriage -.-> escalate;\n\
         \ttriage -. &nbsp;done (never returned)&nbsp; .-> __end__;\n\
         \tescalate --> __end__;\n\
         \tclassDef default fill:#f2f0ff,line-height:1.2\n\
         \tclassDef first fill-opacity:0\n\
         \tclassDef last fill:#bfb6fc\n"
    );
}

#[test]
fn json_has_the_conditions_and_what_each_node_does() {
    let dir = TempDir::new();
    triage_project(&dir);

    let json = views::render(&results(&dir), Tier::Code, Format::Json);

    assert!(json.starts_with("{\"schema\":1,\"tier\":\"code\",\"graphs\":[{\"id\":\"graph:app.flow.workflow\",\"name\":\"workflow\",\"path\":\"app/flow.py\""), "{json}");
    assert!(json.contains("{\"source\":\"triage\",\"target\":\"escalate\",\"conditional\":true,\"label\":null,\"router\":\"route\",\"conditions\":[\"state['disposition'] == 'EMERGENCY'\"],\"reachable\":true,\"line\":23}"), "{json}");
    assert!(
        json.contains("\"label\":\"done\",\"router\":\"route\",\"conditions\":[],\"reachable\":false"),
        "{json}"
    );
    assert!(json.contains("{\"kind\":\"model\",\"model\":\"us.anthropic.claude-sonnet-5-5\",\"client\":\"langchain_aws.ChatBedrockConverse\",\"factory\":\"app.models.claude\",\"output\":\"Decision\",\"tools\":[],\"line\":8}"), "{json}");
    assert!(json.ends_with("]}"), "{json}");
}

#[test]
fn several_graphs_are_each_drawn_in_a_box() {
    let dir = TempDir::new();
    let graph = |name: &str| {
        format!(
            "from langgraph.graph import StateGraph, START\n\
             def {name}(state): return {{}}\n\
             g = StateGraph(dict)\n\
             g.add_node('{name}', {name})\n\
             g.add_edge(START, '{name}')\n"
        )
    };
    dir.write("a.py", graph("first"));
    dir.write("b.py", graph("end"));

    let mermaid = views::render(&results(&dir), Tier::Code, Format::Mermaid);

    assert!(
        mermaid.contains(
            "\tsubgraph g1[\"g · a.py:3\"]\n\
             \tg1___start__([<p>__start__</p>]):::first\n\
             \tg1_first(\"first\")\n\
             \tg1___end__([<p>__end__</p>]):::last\n\
             \tg1___start__ --> g1_first;\n\
             \tg1_first --> g1___end__;\n\
             \tend\n"
        ),
        "{mermaid}"
    );
    // A node named `end` doesn't end the box.
    assert!(mermaid.contains("\tg2_end(\"end\")\n"), "{mermaid}");
}

#[test]
fn the_workflow_tier_groups_nodes_into_parts_and_lists_what_they_use() {
    let dir = TempDir::new();
    triage_project(&dir);

    let results = results(&dir);
    let mermaid = views::render(&results, Tier::Workflow, Format::Mermaid);
    let json = views::render(&results, Tier::Workflow, Format::Json);

    assert!(
        mermaid.contains(
            "\ttriage(\"Triage agent<hr/><small><em>triage<br>model: us.anthropic.claude-sonnet-5-5<br>uses: protocol</em></small>\"):::agent\n"
        ),
        "{mermaid}"
    );
    assert!(
        mermaid.contains("\ttriage -. &nbsp;EMERGENCY&nbsp; .-> escalate;\n"),
        "{mermaid}"
    );
    assert!(
        mermaid.contains("\tclassDef agent fill:#e3f4ec,stroke:#2e9d6a\n"),
        "{mermaid}"
    );
    assert!(
        json.starts_with("{\"schema\":1,\"tier\":\"workflow\",\"graphs\":[{\"id\":\"graph:app.flow.workflow\""),
        "{json}"
    );
    assert!(
        json.contains(
            "{\"source\":\"part:app.flow.workflow.triage\",\"target\":\"part:app.flow.workflow.escalate\",\
             \"conditional\":true,\"retry\":false,\"when\":[\"EMERGENCY\"],\"edges\":[\"triage -> escalate\"]}"
        ),
        "{json}"
    );
}
