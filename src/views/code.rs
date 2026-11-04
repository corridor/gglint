//! Tier 1, code: every LangGraph graph as LangGraph draws it, and under each node what
//! it does: the models it calls, with their structured output and tools, the prompts
//! it uses, the project functions it calls and what it runs in parallel.

use crate::discovery::{DiscoveredPrompt, FileResult};
use crate::flow::{END, Model, Node, NodeKind, START, StateGraph, Step};
use crate::json::Json;
use crate::project::{Part, SourceKind};

use super::mermaid::{Flowchart, id};
use super::{last_segment, prompt_at, prompt_id, qualify, relative_path};

/// Every graph, with the file it's built in, in file order.
pub(crate) fn graphs(results: &[FileResult]) -> impl Iterator<Item = (&FileResult, &StateGraph)> {
    results
        .iter()
        .flat_map(|result| result.graphs.iter().map(move |graph| (result, graph)))
}

/// The graphs as one Mermaid flowchart. Several graphs are each drawn in a box.
pub fn mermaid(results: &[FileResult]) -> String {
    let graphs: Vec<_> = graphs(results).collect();
    let boxed = graphs.len() > 1;
    let mut chart = Flowchart::new();
    for (index, (result, graph)) in graphs.into_iter().enumerate() {
        let prefix = if boxed {
            format!("g{}_", index + 1)
        } else {
            String::new()
        };
        if boxed {
            let title = format!("{} · {}:{}", graph.name(), relative_path(result), graph.location.line);
            chart.begin_group(&format!("g{}", index + 1), &title);
        }
        draw(&mut chart, result, graph, &prefix);
        if boxed {
            chart.end_group();
        }
    }
    chart.finish()
}

fn draw(chart: &mut Flowchart, result: &FileResult, graph: &StateGraph, prefix: &str) {
    if graph.edges.iter().any(|edge| edge.from == START) {
        chart.node(&id(prefix, START), START, &[]);
    }
    for node in &graph.nodes {
        chart.node(&id(prefix, &node.name), &node.name, &details(result, node));
    }
    if graph.edges.iter().any(|edge| edge.to == END) {
        chart.node(&id(prefix, END), END, &[]);
    }
    for edge in &graph.edges {
        let label = match (&edge.label, edge.reachable) {
            (label, true) => label.clone(),
            // A `path_map` entry the router never returns.
            (label, false) => Some(format!("{} (never returned)", label.as_deref().unwrap_or(&edge.to))),
        };
        chart.edge(
            &id(prefix, &edge.from),
            &id(prefix, &edge.to),
            label.as_deref(),
            edge.conditional,
        );
    }
}

/// The lines shown under a node's name.
fn details(result: &FileResult, node: &Node) -> Vec<String> {
    match &node.kind {
        NodeKind::Tools(tools) => return vec![format!("tools: {}", tools.join(", "))],
        NodeKind::Subgraph(graph) => return vec![format!("graph: {graph}")],
        NodeKind::Agent(via) => return vec![format!("agent: {}", last_segment(via))],
        NodeKind::Function { .. } | NodeKind::Other(_) => {}
    }
    let mut lines = Vec::new();
    let mut prompts = Vec::new();
    // Prompts written where they're used, often one-line instructions added to a
    // retry, are counted rather than listed.
    let mut inline = 0;
    let mut calls = Vec::new();
    let mut runs = Vec::new();
    for step in &node.steps {
        match step {
            Step::Model(call) => {
                let mut line = format!("model: {}", model_name(&call.model));
                if let Some(output) = &call.output {
                    line.push_str(" → ");
                    line.push_str(output);
                }
                if !call.tools.is_empty() {
                    line.push_str(" + tools: ");
                    line.push_str(&call.tools.join(", "));
                }
                lines.push(line);
            }
            Step::Prompt(location) => match prompt_at(result, *location).map(prompt_source) {
                Some(Some(source)) => push_new(&mut prompts, source),
                Some(None) => inline += 1,
                None => {}
            },
            Step::Call { name, .. } => push_new(&mut calls, last_segment(name).to_string()),
            Step::Node { name, .. } => push_new(&mut runs, name.clone()),
            Step::Parallel { functions, .. } => lines.push(format!("in parallel: {}", functions.join(", "))),
        }
    }
    match (prompts.is_empty(), inline) {
        (true, 0) => {}
        (true, inline) => lines.push(format!("prompts: {inline} inline")),
        (false, 0) => lines.push(format!("prompts: {}", prompts.join(", "))),
        (false, inline) => lines.push(format!("prompts: {} + {inline} inline", prompts.join(", "))),
    }
    if !calls.is_empty() {
        lines.push(format!("calls: {}", calls.join(", ")));
    }
    if !runs.is_empty() {
        lines.push(format!("runs: {}", runs.join(", ")));
    }
    lines
}

fn push_new(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// What to call a model: its name, or else the class or function that makes it.
pub(crate) fn model_name(model: &Model) -> &str {
    model
        .id
        .as_deref()
        .or(model.client.as_deref().map(last_segment))
        .or(model.factory.as_deref().map(last_segment))
        .unwrap_or("model")
}

/// The name of what a prompt's text comes from: a constant, a function, a file or a
/// config entry.
pub(crate) fn prompt_source(prompt: &DiscoveredPrompt) -> Option<String> {
    let source = prompt.parts.iter().find_map(|part| match part {
        Part::Begin(source) => Some(source),
        _ => None,
    })?;
    Some(match source.kind {
        SourceKind::Constant | SourceKind::Function => last_segment(&source.name).to_string(),
        SourceKind::File => source.path.file_name()?.to_string_lossy().into_owned(),
        SourceKind::Config => source.name.clone(),
    })
}

/// What to call a prompt: where its text comes from, or the start of its text.
pub(crate) fn prompt_label(prompt: &DiscoveredPrompt) -> String {
    if let Some(source) = prompt_source(prompt) {
        return source;
    }
    const PREVIEW: usize = 32;
    let flat = prompt.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview: String = flat.chars().take(PREVIEW).collect();
    let ellipsis = if flat.chars().count() > PREVIEW { "…" } else { "" };
    format!("\"{preview}{ellipsis}\"")
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// The graphs as JSON:
///
/// ```json
/// {"schema": 1, "tier": "code", "graphs": [{
///   "id": "graph:app.flow.build", "name": "build", "path": "app/flow.py", "line": 12, ...
///   "nodes": [{"name": "triage", "kind": "function", "function": "triage_node",
///              "steps": [{"kind": "model", "model": "gpt-4o", "output": "Triage", ...}],
///              "reads": ["messages"], "writes": ["disposition"]}],
///   "edges": [{"source": "triage", "target": "__end__", "conditional": true,
///              "label": null, "router": "route", "conditions": ["done"], "reachable": true}]}]}
/// ```
pub fn json(results: &[FileResult]) -> String {
    let mut out = Json::with_capacity(4096);
    out.0.push_str("{\"schema\":1,\"tier\":\"code\",");
    out.begin_array("graphs");
    for (result, graph) in graphs(results) {
        write_graph(&mut out, result, graph);
    }
    out.end_array();
    out.end_object();
    out.0
}

fn write_graph(out: &mut Json, result: &FileResult, graph: &StateGraph) {
    let path = relative_path(result);
    out.begin_object();
    out.string("id", &format!("graph:{}", qualify(result, &path, graph.name())));
    out.string("name", graph.name());
    out.string("path", &path);
    out.string("module", &result.module);
    out.number("line", graph.location.line);
    out.number("column", graph.location.column);
    out.optional("function", graph.function.as_deref());
    out.optional("variable", graph.variable.as_deref());
    out.optional("state", graph.state.as_deref());
    out.begin_array("nodes");
    for node in &graph.nodes {
        write_node(out, result, &path, node);
    }
    out.end_array();
    out.begin_array("edges");
    for edge in &graph.edges {
        out.begin_object();
        out.string("source", &edge.from);
        out.string("target", &edge.to);
        out.boolean("conditional", edge.conditional);
        out.optional("label", edge.label.as_deref());
        out.optional("router", edge.router.as_deref());
        out.begin_array("conditions");
        for condition in &edge.conditions {
            match condition {
                Some(condition) => out.element(condition),
                None => out.0.push_str("null,"),
            }
        }
        out.end_array();
        out.boolean("reachable", edge.reachable);
        out.number("line", edge.location.line);
        out.close();
    }
    out.end_array();
    out.close();
}

fn write_node(out: &mut Json, result: &FileResult, path: &str, node: &Node) {
    out.begin_object();
    out.string("name", &node.name);
    out.number("line", node.location.line);
    match &node.kind {
        NodeKind::Function { name, location } => {
            out.string("kind", "function");
            out.string("function", name);
            match location {
                Some(location) => out.number("function_line", location.line),
                None => out.null("function_line"),
            }
        }
        NodeKind::Tools(tools) => {
            out.string("kind", "tools");
            out.strings("tools", tools.iter().map(String::as_str));
        }
        NodeKind::Subgraph(graph) => {
            out.string("kind", "graph");
            out.string("graph", graph);
        }
        NodeKind::Agent(via) => {
            out.string("kind", "agent");
            out.string("via", via);
        }
        NodeKind::Other(text) => {
            out.string("kind", "other");
            out.string("expression", text);
        }
    }
    out.begin_array("steps");
    for step in &node.steps {
        // A prompt prompt discovery dropped, made only of values known at runtime.
        if let Step::Prompt(location) = step {
            if prompt_at(result, *location).is_none() {
                continue;
            }
        }
        out.begin_object();
        match step {
            Step::Model(call) => {
                out.string("kind", "model");
                out.optional("model", call.model.id.as_deref());
                out.optional("client", call.model.client.as_deref());
                out.optional("factory", call.model.factory.as_deref());
                out.optional("output", call.output.as_deref());
                out.strings("tools", call.tools.iter().map(String::as_str));
                out.number("line", call.location.line);
            }
            Step::Prompt(location) => {
                out.string("kind", "prompt");
                out.string("id", &prompt_id(path, *location));
                if let Some(prompt) = prompt_at(result, *location) {
                    out.string("label", &prompt_label(prompt));
                }
                out.number("line", location.line);
            }
            Step::Call { name, location } => {
                out.string("kind", "call");
                out.string("name", name);
                out.number("line", location.line);
            }
            Step::Node { name, location } => {
                out.string("kind", "node");
                out.string("name", name);
                out.number("line", location.line);
            }
            Step::Parallel { functions, location } => {
                out.string("kind", "parallel");
                out.strings("functions", functions.iter().map(String::as_str));
                out.number("line", location.line);
            }
        }
        out.close();
    }
    out.end_array();
    out.strings("reads", node.reads.iter().map(String::as_str));
    out.strings("writes", node.writes.iter().map(String::as_str));
    out.close();
}
