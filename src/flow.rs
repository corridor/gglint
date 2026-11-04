//! Tier 1 of the diagrams: the graphs built with LangGraph, read from the code that
//! builds them.
//!
//! LangGraph draws a graph at runtime: `graph.get_graph().draw_mermaid()` steps the
//! compiled graph through an empty run and records where each node can go next. This
//! finds the same graph without running anything, from the builder's calls:
//!
//! ```python
//! workflow = StateGraph(State)
//! workflow.add_node("triage", triage_node)
//! workflow.add_edge(START, "triage")
//! workflow.add_conditional_edges("triage", route, {"retry": "triage", END: END})
//! ```
//!
//! and adds what the runtime drawing can't show:
//! - when a router takes each branch, from the `if`s around its `return`s;
//! - the `path_map` entries a router never returns (`reachable` is false);
//! - what each node does: the models it calls, with their structured output and tools,
//!   the prompts it uses, the project functions it calls, the functions it runs in
//!   parallel, and the state keys it reads and writes.
//!
//! As in LangGraph, a conditional edge's targets come from the `path_map`, then from the
//! router's `Literal[...]` return annotation, and here also from the values it returns.
//! A node routes by itself with `Command[Literal[...]]`, `destinations=` or the
//! `Command(goto=...)` it returns.
//!
//! Extraction runs once a file has been walked, from the [`References`] the checker
//! keeps for files that mention `StateGraph`, so other files cost nothing. Models made
//! by a function in another file, `def claude(): return ChatBedrockConverse(...)`, are
//! filled in by [`link`] once every file has been checked.

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::checker::{CandidateKind, Checker, References, collect_dotted_name};
use crate::discovery::{FileResult, LineIndex, Location};
use crate::prompts;
use crate::semantic::{BindingId, BindingKind, QualifiedName, ScopeId};

/// The name LangGraph gives the start of every graph.
pub const START: &str = "__start__";
/// The name LangGraph gives the end of every graph.
pub const END: &str = "__end__";

// ---------------------------------------------------------------------------
// The graph
// ---------------------------------------------------------------------------

/// A graph built with `StateGraph`.
#[derive(Debug)]
pub struct StateGraph {
    /// The variable the builder is assigned to: `workflow` in `workflow = StateGraph(S)`.
    pub variable: Option<String>,
    /// The function the graph is built in, if any.
    pub function: Option<String>,
    /// The state schema, as written: `S`.
    pub state: Option<String>,
    pub location: Location,
    /// In the order they're added.
    pub nodes: Vec<Node>,
    /// In the order they're added, then the routes nodes take by themselves.
    pub edges: Vec<Edge>,
}

impl StateGraph {
    /// What to call the graph: the function it's built in, or its variable.
    pub fn name(&self) -> &str {
        self.function.as_deref().or(self.variable.as_deref()).unwrap_or("graph")
    }
}

#[derive(Debug)]
pub struct Node {
    pub name: String,
    /// Where it's added to the graph.
    pub location: Location,
    pub kind: NodeKind,
    /// What the node does, in source order.
    pub steps: Vec<Step>,
    /// The state keys it reads: `state["k"]`, `state.get("k")`.
    pub reads: Vec<String>,
    /// The state keys it writes: the keys of the dicts it returns.
    pub writes: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A function, and where it's defined when that's in this file.
    Function { name: String, location: Option<Location> },
    /// `ToolNode([...])`, with its tools.
    Tools(Vec<String>),
    /// A graph compiled in this file, by its name.
    Subgraph(String),
    /// A prebuilt agent, by what was called: `langgraph.prebuilt.create_react_agent`.
    Agent(String),
    /// Anything else, as written.
    Other(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// A call to a chat model.
    Model(ModelCall),
    /// A prompt found by prompt discovery, where it's written.
    Prompt(Location),
    /// A call to a function in another module of the project, by its qualified name.
    Call { name: String, location: Location },
    /// A call to the function of another node.
    Node { name: String, location: Location },
    /// Functions started together: `executor.submit(f)`, `asyncio.gather(f(), g())`.
    Parallel { functions: Vec<String>, location: Location },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCall {
    pub location: Location,
    pub model: Model,
    /// The schema of a structured output: `Triage` in `with_structured_output(Triage)`.
    pub output: Option<String>,
    /// The tools bound to the model: `bind_tools([search])`.
    pub tools: Vec<String>,
    /// Whether the call is known to be a model's from this file: the model is created
    /// here, or configured as only a model is (`bind_tools`, `with_structured_output`).
    /// Otherwise it's a model's only if [`link`] finds that `factory` makes one.
    pub(crate) certain: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Model {
    /// The chat model class or function: `langchain_aws.ChatBedrockConverse`.
    pub client: Option<String>,
    /// The model, when it's written as a literal: `us.anthropic.claude-sonnet-5-5`.
    pub id: Option<String>,
    /// The function in another module that creates it: `models.claude.claude_sonnet`.
    pub factory: Option<String>,
}

#[derive(Debug)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// Where the edge is added.
    pub location: Location,
    /// Whether a router or the node decides to take it.
    pub conditional: bool,
    /// The `path_map` key, when it isn't the target's name.
    pub label: Option<String>,
    /// The router function.
    pub router: Option<String>,
    /// When it's taken: the conditions of the `if`s around each `return` that leads
    /// here, in order. `None` for a `return` that no `if` guards.
    pub conditions: Vec<Option<String>>,
    /// False for a `path_map` entry the router never returns.
    pub reachable: bool,
}

impl Edge {
    fn direct(from: String, to: String, location: Location) -> Self {
        Self {
            from,
            to,
            location,
            conditional: false,
            label: None,
            router: None,
            conditions: Vec::new(),
            reachable: true,
        }
    }

    fn conditional(from: String, to: String, location: Location) -> Self {
        Self {
            conditional: true,
            ..Self::direct(from, to, location)
        }
    }
}

/// A module-level function that returns a chat model:
/// `def claude(): return ChatBedrockConverse(model_id="...")`.
#[derive(Debug)]
pub(crate) struct ModelFactory {
    /// Its qualified name.
    pub function: String,
    pub model: Model,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

const STATE_GRAPHS: &[&str] = &[
    "langgraph.graph.StateGraph",
    "langgraph.graph.state.StateGraph",
    "langgraph.graph.MessageGraph",
    "langgraph.graph.message.MessageGraph",
];
const STARTS: &[&str] = &[
    "langgraph.graph.START",
    "langgraph.graph.state.START",
    "langgraph.constants.START",
];
const ENDS: &[&str] = &[
    "langgraph.graph.END",
    "langgraph.graph.state.END",
    "langgraph.constants.END",
];
const TOOL_NODES: &[&str] = &["langgraph.prebuilt.ToolNode", "langgraph.prebuilt.tool_node.ToolNode"];
/// Routes to `tools` when the last message calls a tool, and to the end otherwise.
const TOOLS_CONDITIONS: &[&str] = &[
    "langgraph.prebuilt.tools_condition",
    "langgraph.prebuilt.tool_node.tools_condition",
];
const TOOL_DECORATORS: &[&str] = &[
    "langchain_core.tools.tool",
    "langchain_core.tools.convert.tool",
    "langchain.tools.tool",
    "langchain.agents.tool",
];
const SENDS: &[&str] = &["langgraph.types.Send", "langgraph.constants.Send"];
const COMMANDS: &[&str] = &["langgraph.types.Command"];
const LITERALS: &[&str] = &["typing.Literal", "typing_extensions.Literal"];
const GATHERS: &[&str] = &["asyncio.gather"];
/// Agents that can be a node of their own.
const PREBUILT_AGENTS: &[&str] = &[
    "langgraph.prebuilt.create_react_agent",
    "langchain.agents.create_agent",
    "deepagents.create_deep_agent",
];

/// Chat model classes, and the function that makes one from a model's name.
const CHAT_MODELS: &[&str] = &[
    "langchain.chat_models.init_chat_model",
    "langchain_anthropic.ChatAnthropic",
    "langchain_aws.ChatBedrock",
    "langchain_aws.ChatBedrockConverse",
    "langchain_community.chat_models.ChatOllama",
    "langchain_community.chat_models.ChatOpenAI",
    "langchain_google_genai.ChatGoogleGenerativeAI",
    "langchain_google_vertexai.ChatVertexAI",
    "langchain_groq.ChatGroq",
    "langchain_mistralai.ChatMistralAI",
    "langchain_ollama.ChatOllama",
    "langchain_openai.AzureChatOpenAI",
    "langchain_openai.ChatOpenAI",
];
/// The keyword arguments a chat model's name is passed as.
const MODEL_KEYWORDS: &[&str] = &["model", "model_id", "model_name", "deployment_name", "azure_deployment"];

/// Methods that call a model.
const INVOKES: &[&str] = &["invoke", "ainvoke", "stream", "astream", "batch", "abatch"];
/// Methods that return the model they're called on, configured: `llm.bind_tools([...])`.
const CONFIGURES: &[&str] = &[
    "with_structured_output",
    "bind_tools",
    "bind",
    "with_retry",
    "with_config",
    "with_fallbacks",
    "configurable_fields",
];

/// How many variables and functions to follow to find what a value is.
const MAX_DEPTH: usize = 6;

fn matches(name: &QualifiedName<'_>, dotted: &[&str]) -> bool {
    dotted.iter().any(|dotted| {
        let mut parts = dotted.split('.');
        name.segments().iter().all(|segment| parts.next() == Some(segment)) && parts.next().is_none()
    })
}

/// A cheap test for whether a file can build a graph, made before it's walked.
pub(crate) fn may_build_graph(source: &str) -> bool {
    let source = source.as_bytes();
    memchr::memmem::find(source, b"StateGraph").is_some() || memchr::memmem::find(source, b"MessageGraph").is_some()
}

/// A cheap test for whether a file can create a chat model.
pub(crate) fn may_create_model(source: &str) -> bool {
    let source = source.as_bytes();
    memchr::memmem::find(source, b"Chat").is_some() || memchr::memmem::find(source, b"init_chat_model").is_some()
}

fn string_literal(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::StringLiteral(literal) => Some(literal.value.to_str()),
        _ => None,
    }
}

fn elements(expr: &Expr) -> Option<&[Expr]> {
    match expr {
        Expr::List(ast::ExprList { elts, .. })
        | Expr::Tuple(ast::ExprTuple { elts, .. })
        | Expr::Set(ast::ExprSet { elts, .. }) => Some(elts),
        _ => None,
    }
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// The model a chat model is created with, when it's a literal.
fn model_id(call: &ast::ExprCall) -> Option<String> {
    MODEL_KEYWORDS
        .iter()
        .find_map(|name| call.arguments.find_keyword(name).map(|keyword| &keyword.value))
        .or_else(|| call.arguments.args.first())
        .and_then(string_literal)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Model factories
// ---------------------------------------------------------------------------

/// The module-level functions of a file that return a chat model.
pub(crate) fn model_factories(checker: &Checker<'_>) -> Vec<ModelFactory> {
    let mut models: FxHashMap<TextSize, (&ast::ExprCall, String)> = FxHashMap::default();
    for candidate in &checker.candidates {
        if let (CandidateKind::Call(call), Some(name)) = (&candidate.kind, &candidate.qualified_name) {
            if matches(name, CHAT_MODELS) {
                models.insert(call.start(), (call, name.to_string()));
            }
        }
    }
    if models.is_empty() {
        return Vec::new();
    }
    let module = checker.semantic.module_path();
    let mut factories = Vec::new();
    for (name, binding) in checker.semantic.global_scope().bindings() {
        let binding = checker.semantic.binding(binding);
        if !matches!(binding.kind, BindingKind::FunctionDefinition) {
            continue;
        }
        let returned = checker.returned_call(binding.range.start());
        let Some((call, client)) = returned.and_then(|start| models.get(&start)) else {
            continue;
        };
        factories.push(ModelFactory {
            function: module.iter().copied().chain([name]).collect::<Vec<_>>().join("."),
            model: Model {
                client: Some(client.clone()),
                id: model_id(call),
                factory: None,
            },
        });
    }
    factories.sort_by(|a, b| a.function.cmp(&b.function));
    factories
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// The graphs built in a file, from a checker run with references.
pub(crate) fn extract(checker: &Checker<'_>, source: &str, lines: &LineIndex) -> Vec<StateGraph> {
    let Some(references) = &checker.references else {
        return Vec::new();
    };
    Extractor::new(checker, references, source, lines).graphs()
}

/// A graph while it's being read.
struct Building {
    graph: StateGraph,
    /// The function of each node, for nodes that are functions in this file, in the
    /// order of `graph.nodes`.
    functions: Vec<Option<BindingId>>,
    /// The strings the nodes write to each state key: what a router that returns
    /// `state["key"]` can return.
    written: FxHashMap<String, Vec<String>>,
    /// Whether every node and edge could be read. A name only known at runtime means
    /// edges may be missing, so no node can be said to have nowhere to go.
    complete: bool,
}

/// One call to a builder.
struct Operation<'a> {
    graph: usize,
    method: &'a str,
    call: &'a ast::ExprCall,
}

/// What a router or a node can return, and when.
struct Return {
    target: Target,
    condition: Option<String>,
    /// Whether it's a `Command(goto=...)` or a `Send`, which route from a node.
    routes: bool,
}

enum Target {
    Node(String),
    /// Whatever the state holds under this key: `return state["next"]`.
    State(String),
    Unknown,
}

/// A router: the function `add_conditional_edges` is given.
struct Router {
    name: Option<String>,
    returns: Vec<Return>,
    /// The targets its `Literal[...]` return annotation declares.
    declared: Vec<String>,
    /// Whether every value it returns is known.
    known: bool,
}

/// What a value is, as far as model calls go.
#[derive(Default)]
struct Traced {
    model: Model,
    output: Option<String>,
    tools: Vec<String>,
    certain: bool,
}

struct Extractor<'c, 'a> {
    checker: &'c Checker<'a>,
    references: &'c References<'a>,
    source: &'c str,
    lines: &'c LineIndex,
    /// `tools.append(x)` and `tools.extend([...])`, by the list's binding.
    appended: FxHashMap<BindingId, Vec<(&'a str, &'a Expr)>>,
}

impl<'c, 'a> Extractor<'c, 'a> {
    fn new(checker: &'c Checker<'a>, references: &'c References<'a>, source: &'c str, lines: &'c LineIndex) -> Self {
        let mut appended: FxHashMap<BindingId, Vec<(&'a str, &'a Expr)>> = FxHashMap::default();
        for candidate in &checker.candidates {
            let CandidateKind::Call(call) = candidate.kind else {
                continue;
            };
            let Expr::Attribute(attribute) = call.func.as_ref() else {
                continue;
            };
            let method = attribute.attr.as_str();
            if !matches!(method, "append" | "extend") {
                continue;
            }
            let Expr::Name(list) = attribute.value.as_ref() else {
                continue;
            };
            if let (Some(&binding), Some(argument)) =
                (references.names.get(&list.range.start()), call.arguments.args.first())
            {
                appended.entry(binding).or_default().push((method, argument));
            }
        }
        Self {
            checker,
            references,
            source,
            lines,
            appended,
        }
    }

    fn graphs(&self) -> Vec<StateGraph> {
        // Each variable, by the start of the value it's assigned: how a builder or a
        // compiled graph is named.
        let assigned: FxHashMap<TextSize, BindingId> = self
            .references
            .values
            .iter()
            .map(|(&binding, value)| (value.start(), binding))
            .collect();

        let mut building = Vec::new();
        let mut builders: FxHashMap<BindingId, usize> = FxHashMap::default();
        for candidate in &self.checker.candidates {
            let (CandidateKind::Call(call), Some(name)) = (&candidate.kind, &candidate.qualified_name) else {
                continue;
            };
            if !matches(name, STATE_GRAPHS) {
                continue;
            }
            let variable = assigned.get(&call.start()).copied();
            if let Some(variable) = variable {
                builders.insert(variable, building.len());
            }
            building.push(Building {
                graph: StateGraph {
                    variable: variable.map(|binding| self.binding_name(binding).to_string()),
                    function: self
                        .function_of(candidate.scope)
                        .map(|function| function.name.to_string()),
                    state: call
                        .arguments
                        .find_argument_value("state_schema", 0)
                        .map(|state| self.text(state)),
                    location: self.location(call.start()),
                    nodes: Vec::new(),
                    edges: Vec::new(),
                },
                functions: Vec::new(),
                written: FxHashMap::default(),
                complete: true,
            });
        }
        if building.is_empty() {
            return Vec::new();
        }

        // The builders' calls, in the order the walk met them: source order within a
        // function.
        let mut operations = Vec::new();
        for candidate in &self.checker.candidates {
            let CandidateKind::Call(call) = candidate.kind else {
                continue;
            };
            let Expr::Attribute(attribute) = call.func.as_ref() else {
                continue;
            };
            if let Some(&graph) = self
                .binding(&attribute.value)
                .and_then(|binding| builders.get(&binding))
            {
                operations.push(Operation {
                    graph,
                    method: attribute.attr.as_str(),
                    call,
                });
            }
        }
        // A graph used as a node of another is the call that compiles it.
        let names: Vec<String> = building.iter().map(|graph| graph.graph.name().to_string()).collect();
        let compiled: FxHashMap<TextSize, &str> = operations
            .iter()
            .filter(|operation| operation.method == "compile")
            .map(|operation| (operation.call.start(), names[operation.graph].as_str()))
            .collect();

        for operation in &operations {
            let graph = &mut building[operation.graph];
            match operation.method {
                "add_node" => self.add_node(graph, operation.call, &compiled),
                "add_sequence" => self.add_sequence(graph, operation.call, &compiled),
                _ => {}
            }
        }
        // What each node does, once every node's function is known.
        for graph in &mut building {
            self.fill_nodes(graph);
        }
        for operation in &operations {
            let call = operation.call;
            let arguments = &call.arguments;
            let location = self.location(call.start());
            let graph = &mut building[operation.graph];
            match operation.method {
                "add_edge" => {
                    let (Some(from), Some(to)) = (
                        arguments.find_argument_value("start_key", 0),
                        arguments.find_argument_value("end_key", 1),
                    ) else {
                        continue;
                    };
                    // `add_edge(["a", "b"], "c")` waits for both.
                    let sources = elements(from).unwrap_or(std::slice::from_ref(from));
                    let from: Vec<String> = sources.iter().filter_map(|from| self.node_name(from)).collect();
                    let to = self.node_name(to);
                    if to.is_none() || from.len() < sources.len() {
                        graph.complete = false;
                    }
                    let Some(to) = to else { continue };
                    for from in from {
                        graph.graph.edges.push(Edge::direct(from, to.clone(), location));
                    }
                }
                "set_entry_point" => match arguments
                    .find_argument_value("key", 0)
                    .and_then(|key| self.node_name(key))
                {
                    Some(to) => graph.graph.edges.push(Edge::direct(START.to_string(), to, location)),
                    None => graph.complete = false,
                },
                "set_finish_point" => match arguments
                    .find_argument_value("key", 0)
                    .and_then(|key| self.node_name(key))
                {
                    Some(from) => graph.graph.edges.push(Edge::direct(from, END.to_string(), location)),
                    None => graph.complete = false,
                },
                "add_conditional_edges" => {
                    match arguments
                        .find_argument_value("source", 0)
                        .and_then(|from| self.node_name(from))
                    {
                        Some(from) => {
                            let path = arguments.find_argument_value("path", 1);
                            let path_map = arguments.find_argument_value("path_map", 2);
                            self.add_branch(graph, from, path, path_map, location);
                        }
                        None => graph.complete = false,
                    }
                }
                "set_conditional_entry_point" => {
                    let path = arguments.find_argument_value("path", 0);
                    let path_map = arguments.find_argument_value("path_map", 1);
                    self.add_branch(graph, START.to_string(), path, path_map, location);
                }
                _ => {}
            }
        }
        for graph in &mut building {
            self.add_routes(graph);
            if graph.complete {
                finish(&mut graph.graph);
            }
        }
        building.into_iter().map(|graph| graph.graph).collect()
    }

    // -----------------------------------------------------------------------
    // Nodes
    // -----------------------------------------------------------------------

    fn add_node(&self, graph: &mut Building, call: &'a ast::ExprCall, compiled: &FxHashMap<TextSize, &str>) {
        let arguments = &call.arguments;
        let Some(first) = arguments.find_argument_value("node", 0) else {
            return;
        };
        let (name, action) = match (arguments.find_argument_value("action", 1), self.string(first)) {
            (Some(action), Some(name)) => (name, Some(action)),
            // A name only known at runtime is written as the expression, as in prompts.
            (Some(action), None) => {
                graph.complete = false;
                (format!("{{{}}}", self.text(first)), Some(action))
            }
            (None, Some(name)) => (name, None),
            // `add_node(triage)` is named after the function.
            (None, None) => match self.callable_name(first) {
                Some(name) => (name, Some(first)),
                None => {
                    graph.complete = false;
                    return;
                }
            },
        };
        let location = self.location(call.start());
        self.push_node(graph, name.clone(), location, action, compiled);

        // `destinations=("a", "b")`, or `{"a": "label"}`: where the node can go, for
        // drawing it.
        let Some(destinations) = arguments.find_keyword("destinations") else {
            return;
        };
        let targets: Vec<(String, Option<String>)> = match &destinations.value {
            Expr::Dict(dict) => dict
                .items
                .iter()
                .filter_map(|item| {
                    let target = self.node_name(item.key.as_ref()?)?;
                    Some((target, self.string(&item.value)))
                })
                .collect(),
            value => elements(value)
                .unwrap_or_default()
                .iter()
                .filter_map(|target| Some((self.node_name(target)?, None)))
                .collect(),
        };
        for (to, label) in targets {
            graph.graph.edges.push(Edge {
                label,
                ..Edge::conditional(name.clone(), to, location)
            });
        }
    }

    /// `add_sequence([a, b])` or `add_sequence([("a", a), ("b", b)])`: nodes, each
    /// followed by the next.
    fn add_sequence(&self, graph: &mut Building, call: &'a ast::ExprCall, compiled: &FxHashMap<TextSize, &str>) {
        let Some(sequence) = call.arguments.find_argument_value("nodes", 0).and_then(elements) else {
            return;
        };
        let location = self.location(call.start());
        let mut previous: Option<String> = None;
        for element in sequence {
            let (name, action) = match element {
                Expr::Tuple(ast::ExprTuple { elts, .. }) if elts.len() == 2 => (self.node_name(&elts[0]), &elts[1]),
                _ => (self.callable_name(element), element),
            };
            let Some(name) = name else { continue };
            self.push_node(graph, name.clone(), location, Some(action), compiled);
            if let Some(previous) = previous.replace(name.clone()) {
                graph.graph.edges.push(Edge::direct(previous, name, location));
            }
        }
    }

    fn push_node(
        &self,
        graph: &mut Building,
        name: String,
        location: Location,
        action: Option<&'a Expr>,
        compiled: &FxHashMap<TextSize, &str>,
    ) {
        let (kind, function) = match action {
            Some(action) => self.node_kind(action, compiled),
            None => (NodeKind::Other(String::new()), None),
        };
        graph.graph.nodes.push(Node {
            name,
            location,
            kind,
            steps: Vec::new(),
            reads: Vec::new(),
            writes: Vec::new(),
        });
        graph.functions.push(function);
    }

    /// What a node runs, and its function when that's defined in this file.
    fn node_kind(&self, action: &'a Expr, compiled: &FxHashMap<TextSize, &str>) -> (NodeKind, Option<BindingId>) {
        if let Some((binding, function)) = self.function(action) {
            let location = self.location(function.name.range.start());
            let kind = NodeKind::Function {
                name: function.name.to_string(),
                location: Some(location),
            };
            return (kind, Some(binding));
        }
        let call = match action {
            Expr::Call(call) => Some(call),
            _ => match self
                .binding(action)
                .and_then(|binding| self.references.values.get(&binding))
            {
                Some(Expr::Call(call)) => Some(call),
                _ => None,
            },
        };
        if let Some(call) = call {
            if let Some(graph) = compiled.get(&call.start()) {
                return (NodeKind::Subgraph((*graph).to_string()), None);
            }
            if let Some(callee) = self.resolve(&call.func) {
                if matches(&callee, TOOL_NODES) {
                    let tools = call.arguments.find_argument_value("tools", 0);
                    return (
                        NodeKind::Tools(tools.map(|tools| self.tools(tools, 0)).unwrap_or_default()),
                        None,
                    );
                }
                if matches(&callee, PREBUILT_AGENTS) {
                    return (NodeKind::Agent(callee.to_string()), None);
                }
            }
        }
        // A function imported from another module.
        if let Some(name) = self.resolve(action) {
            let kind = NodeKind::Function {
                name: name.to_string(),
                location: None,
            };
            return (kind, None);
        }
        (NodeKind::Other(self.text(action)), None)
    }

    /// Fills in what each node whose function is in this file does, and what it
    /// writes to the state.
    fn fill_nodes(&self, graph: &mut Building) {
        let nodes: FxHashMap<BindingId, &str> = graph
            .functions
            .iter()
            .zip(&graph.graph.nodes)
            .filter_map(|(function, node)| Some(((*function)?, node.name.as_str())))
            .collect();
        let mut filled = Vec::with_capacity(graph.graph.nodes.len());
        for function in &graph.functions {
            filled.push(function.map(|function| self.inside(function, &nodes)));
        }
        for (node, inside) in graph.graph.nodes.iter_mut().zip(filled) {
            let Some(inside) = inside else { continue };
            node.steps = inside.steps;
            node.reads = inside.reads;
            for (key, value) in inside.written {
                if let Some(value) = value {
                    let values = graph.written.entry(key.clone()).or_default();
                    push_unique(values, value);
                }
                push_unique(&mut node.writes, key);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Branches
    // -----------------------------------------------------------------------

    fn add_branch(
        &self,
        graph: &mut Building,
        from: String,
        path: Option<&'a Expr>,
        path_map: Option<&'a Expr>,
        location: Location,
    ) {
        let router = path.map(|path| self.router(path));
        let (returns, known, router_name, declared) = match &router {
            Some(router) => (
                router.returns.as_slice(),
                router.known,
                router.name.clone(),
                router.declared.as_slice(),
            ),
            None => (&[][..], false, None, &[][..]),
        };
        // What the router returns, with state lookups replaced by what the nodes write.
        let mut produced: Vec<(String, Option<String>)> = Vec::new();
        let mut known = known;
        for value in returns {
            match &value.target {
                Target::Node(name) => produced.push((name.clone(), value.condition.clone())),
                Target::State(key) => match graph.written.get(key) {
                    Some(written) => {
                        for name in written {
                            produced.push((name.clone(), value.condition.clone()));
                        }
                    }
                    None => known = false,
                },
                Target::Unknown => known = false,
            }
        }
        let conditions = |key: &str| -> Vec<Option<String>> {
            produced
                .iter()
                .filter(|(name, _)| name == key)
                .map(|(_, condition)| condition.clone())
                .collect()
        };

        let mut edges = Vec::new();
        match path_map {
            Some(Expr::Dict(dict)) => {
                for item in &dict.items {
                    let (Some(key), Some(to)) = (
                        item.key.as_ref().and_then(|key| self.node_name(key)),
                        self.node_name(&item.value),
                    ) else {
                        graph.complete = false;
                        continue;
                    };
                    let conditions = conditions(&key);
                    let reachable = !known || !conditions.is_empty() || declared.contains(&key);
                    edges.push(Edge {
                        label: (key != to).then_some(key),
                        conditions,
                        reachable,
                        ..Edge::conditional(from.clone(), to, location)
                    });
                }
            }
            Some(path_map) => {
                let targets = elements(path_map).unwrap_or_default();
                let names: Vec<String> = targets.iter().filter_map(|to| self.node_name(to)).collect();
                if names.is_empty() || names.len() < targets.len() {
                    graph.complete = false;
                }
                for to in names {
                    let conditions = conditions(&to);
                    let reachable = !known || !conditions.is_empty() || declared.contains(&to);
                    edges.push(Edge {
                        conditions,
                        reachable,
                        ..Edge::conditional(from.clone(), to, location)
                    });
                }
            }
            None => {
                if !known && declared.is_empty() {
                    graph.complete = false;
                }
                // Without a path map, the targets are what the router can return. A
                // state lookup can hold anything, so only nodes of the graph count.
                let mut targets: Vec<String> = declared.to_vec();
                for (name, _) in &produced {
                    let is_node = name == END || graph.graph.nodes.iter().any(|node| &node.name == name);
                    if is_node {
                        push_unique(&mut targets, name.clone());
                    }
                }
                for to in targets {
                    let conditions = conditions(&to);
                    edges.push(Edge {
                        conditions,
                        ..Edge::conditional(from.clone(), to, location)
                    });
                }
            }
        }
        for edge in &mut edges {
            edge.router = router_name.clone();
        }
        graph.graph.edges.extend(edges);
    }

    fn router(&self, path: &'a Expr) -> Router {
        if let Some(name) = self.resolve(path) {
            if matches(&name, TOOLS_CONDITIONS) {
                let returns = vec![
                    Return {
                        target: Target::Node("tools".to_string()),
                        condition: Some("the last message calls a tool".to_string()),
                        routes: false,
                    },
                    Return {
                        target: Target::Node(END.to_string()),
                        condition: None,
                        routes: false,
                    },
                ];
                return Router {
                    name: Some("tools_condition".to_string()),
                    returns,
                    declared: Vec::new(),
                    known: true,
                };
            }
        }
        if let Some((_, function)) = self.function(path) {
            let mut returns = Vec::new();
            self.returns_in(&function.body, &mut Vec::new(), &mut returns);
            return Router {
                name: Some(function.name.to_string()),
                returns,
                declared: function
                    .returns
                    .as_deref()
                    .map(|returns| self.literal_targets(returns))
                    .unwrap_or_default(),
                known: true,
            };
        }
        if let Expr::Lambda(lambda) = path {
            let mut returns = Vec::new();
            self.returned(&lambda.body, &mut Vec::new(), false, &mut returns);
            return Router {
                name: Some("lambda".to_string()),
                returns,
                declared: Vec::new(),
                known: true,
            };
        }
        Router {
            name: Some(self.text(path)),
            returns: Vec::new(),
            declared: Vec::new(),
            known: false,
        }
    }

    /// Edges for the nodes that route themselves: by `Command[Literal[...]]`, or by
    /// returning `Command(goto=...)` or `Send(...)`.
    fn add_routes(&self, graph: &mut Building) {
        let mut routes: Vec<Edge> = Vec::new();
        for (node, function) in graph.graph.nodes.iter().zip(&graph.functions) {
            let Some(function) = function.and_then(|function| self.references.functions.get(&function)) else {
                continue;
            };
            let mut targets: Vec<(String, Option<String>)> = function
                .returns
                .as_deref()
                .map(|returns| self.literal_targets(returns))
                .unwrap_or_default()
                .into_iter()
                .map(|target| (target, None))
                .collect();
            let mut returns = Vec::new();
            self.returns_in(&function.body, &mut Vec::new(), &mut returns);
            for value in returns {
                if let (true, Target::Node(name)) = (value.routes, value.target) {
                    targets.push((name, value.condition));
                }
            }
            for (to, condition) in targets {
                if let Some(edge) = routes.iter_mut().find(|edge| edge.from == node.name && edge.to == to) {
                    edge.conditions.push(condition);
                } else if !graph
                    .graph
                    .edges
                    .iter()
                    .any(|edge| edge.from == node.name && edge.to == to)
                {
                    routes.push(Edge {
                        conditions: vec![condition],
                        ..Edge::conditional(node.name.clone(), to, node.location)
                    });
                }
            }
        }
        graph.graph.edges.extend(routes);
    }

    /// The node names a `Literal[...]` annotation lists, also inside `Command[...]`,
    /// `Union[...]` and `A | B`.
    fn literal_targets(&self, annotation: &'a Expr) -> Vec<String> {
        let mut targets = Vec::new();
        self.push_literal_targets(annotation, &mut targets);
        targets
    }

    fn push_literal_targets(&self, annotation: &'a Expr, targets: &mut Vec<String>) {
        match annotation {
            Expr::Subscript(subscript) => {
                let Some(name) = self.resolve(&subscript.value) else {
                    return;
                };
                if matches(&name, LITERALS) {
                    let values = elements(&subscript.slice).unwrap_or(std::slice::from_ref(subscript.slice.as_ref()));
                    for value in values {
                        if let Some(target) = self.node_name(value) {
                            push_unique(targets, target);
                        }
                    }
                } else {
                    // `Command[...]`, `Union[...]`, `Optional[...]`.
                    let values = elements(&subscript.slice).unwrap_or(std::slice::from_ref(subscript.slice.as_ref()));
                    for value in values {
                        self.push_literal_targets(value, targets);
                    }
                }
            }
            Expr::BinOp(ast::ExprBinOp {
                left,
                op: ast::Operator::BitOr,
                right,
                ..
            }) => {
                self.push_literal_targets(left, targets);
                self.push_literal_targets(right, targets);
            }
            _ => {}
        }
    }

    /// Collects the values `body` returns and the conditions under which it returns
    /// them. Nested functions and classes are left out: their `return`s are their own.
    fn returns_in(&self, body: &'a [Stmt], conditions: &mut Vec<String>, out: &mut Vec<Return>) {
        for stmt in body {
            match stmt {
                Stmt::Return(ast::StmtReturn { value: Some(value), .. }) => {
                    self.returned(value, conditions, false, out)
                }
                Stmt::If(if_) => {
                    conditions.push(self.text(&*if_.test));
                    self.returns_in(&if_.body, conditions, out);
                    conditions.pop();
                    let only_else = matches!(if_.elif_else_clauses.as_slice(), [clause] if clause.test.is_none());
                    for clause in &if_.elif_else_clauses {
                        match &clause.test {
                            Some(test) => conditions.push(self.text(test)),
                            None if only_else => conditions.push(format!("not ({})", self.text(&*if_.test))),
                            // The `else` after `elif`s: none of the conditions above.
                            None => {
                                self.returns_in(&clause.body, conditions, out);
                                continue;
                            }
                        }
                        self.returns_in(&clause.body, conditions, out);
                        conditions.pop();
                    }
                }
                Stmt::Match(match_) => {
                    let subject = self.text(&*match_.subject);
                    for case in &match_.cases {
                        conditions.push(format!("{subject} matches {}", self.text(&case.pattern)));
                        self.returns_in(&case.body, conditions, out);
                        conditions.pop();
                    }
                }
                Stmt::For(ast::StmtFor { body, orelse, .. }) | Stmt::While(ast::StmtWhile { body, orelse, .. }) => {
                    self.returns_in(body, conditions, out);
                    self.returns_in(orelse, conditions, out);
                }
                Stmt::With(with) => self.returns_in(&with.body, conditions, out),
                Stmt::Try(try_) => {
                    self.returns_in(&try_.body, conditions, out);
                    for handler in &try_.handlers {
                        let ast::ExceptHandler::ExceptHandler(handler) = handler;
                        self.returns_in(&handler.body, conditions, out);
                    }
                    self.returns_in(&try_.orelse, conditions, out);
                    self.returns_in(&try_.finalbody, conditions, out);
                }
                _ => {}
            }
        }
    }

    /// What returning `value` routes to.
    fn returned(&self, value: &'a Expr, conditions: &mut Vec<String>, routes: bool, out: &mut Vec<Return>) {
        let condition = |conditions: &[String]| (!conditions.is_empty()).then(|| conditions.join(" and "));
        match value {
            Expr::If(if_) => {
                conditions.push(self.text(&*if_.test));
                self.returned(&if_.body, conditions, routes, out);
                conditions.pop();
                conditions.push(format!("not ({})", self.text(&*if_.test)));
                self.returned(&if_.orelse, conditions, routes, out);
                conditions.pop();
            }
            Expr::List(_) | Expr::Tuple(_) => {
                for element in elements(value).unwrap_or_default() {
                    self.returned(element, conditions, routes, out);
                }
            }
            Expr::ListComp(comprehension) => self.returned(&comprehension.elt, conditions, routes, out),
            Expr::Call(call) => {
                let callee = self.resolve(&call.func);
                if callee.as_ref().is_some_and(|callee| matches(callee, SENDS)) {
                    let target = call
                        .arguments
                        .find_argument_value("node", 0)
                        .and_then(|node| self.node_name(node));
                    out.push(Return {
                        target: target.map_or(Target::Unknown, Target::Node),
                        condition: condition(conditions),
                        routes: true,
                    });
                } else if callee.as_ref().is_some_and(|callee| matches(callee, COMMANDS)) {
                    if let Some(goto) = call.arguments.find_keyword("goto") {
                        self.returned(&goto.value, conditions, true, out);
                    }
                } else {
                    out.push(Return {
                        target: Target::Unknown,
                        condition: condition(conditions),
                        routes,
                    });
                }
            }
            _ => {
                let target = match (self.node_name(value), self.state_key(value)) {
                    (Some(name), _) => Target::Node(name),
                    (None, Some(key)) => Target::State(key.to_string()),
                    (None, None) => Target::Unknown,
                };
                out.push(Return {
                    target,
                    condition: condition(conditions),
                    routes,
                });
            }
        }
    }

    /// `state["key"]` or `state.get("key")` → `key`.
    fn state_key(&self, expr: &'a Expr) -> Option<&'a str> {
        match expr {
            Expr::Subscript(subscript) if subscript.value.is_name_expr() => string_literal(&subscript.slice),
            Expr::Call(call) => match call.func.as_ref() {
                Expr::Attribute(attribute) if attribute.attr.as_str() == "get" && attribute.value.is_name_expr() => {
                    call.arguments.args.first().and_then(string_literal)
                }
                _ => None,
            },
            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Inside a node
    // -----------------------------------------------------------------------

    /// What the function `function` does, with the functions in this file it calls.
    fn inside(&self, function: BindingId, nodes: &FxHashMap<BindingId, &str>) -> Inside {
        let definition = self.references.functions[&function];
        let state = definition
            .parameters
            .iter()
            .next()
            .map(|parameter| parameter.name().range);
        let mut visit = InsideVisitor {
            extractor: self,
            nodes,
            state,
            steps: Vec::new(),
            reads: Vec::new(),
            queue: Vec::new(),
            visited: FxHashSet::from_iter([function]),
            ranges: vec![definition.range],
            executors: FxHashMap::default(),
        };
        visit.visit_body(&definition.body);
        // Helpers in this file are part of the node: their models, prompts and calls
        // are the node's.
        while let Some(helper) = visit.queue.pop() {
            let helper = self.references.functions[&helper];
            visit.ranges.push(helper.range);
            visit.visit_body(&helper.body);
        }

        // Prompts are found by prompt discovery; these are the ones written in the node.
        for prompt in &self.checker.prompts {
            let start = prompt.start();
            if visit.ranges.iter().any(|range| range.contains(start)) {
                visit.steps.push((start, Step::Prompt(self.location(start))));
            }
        }
        visit.steps.sort_by_key(|(start, _)| *start);
        let mut steps: Vec<Step> = Vec::with_capacity(visit.steps.len());
        for (_, step) in visit.steps {
            if !steps.iter().any(|seen| same_step(seen, &step)) {
                steps.push(step);
            }
        }

        let mut written = Vec::new();
        for value in self.references.returns.get(&function).into_iter().flatten() {
            self.push_written(value, 0, &mut written);
        }
        Inside {
            steps,
            reads: visit.reads,
            written,
        }
    }

    /// The state keys a node writes by returning `value`, and the string written to
    /// each when it's a literal.
    fn push_written(&self, value: &'a Expr, depth: usize, written: &mut Vec<(String, Option<String>)>) {
        if depth > MAX_DEPTH {
            return;
        }
        match value {
            Expr::Dict(dict) => {
                for item in &dict.items {
                    match &item.key {
                        Some(key) => {
                            if let Some(key) = string_literal(key) {
                                written.push((key.to_string(), self.string(&item.value)));
                            }
                        }
                        // `{**update, ...}`
                        None => self.push_written(&item.value, depth + 1, written),
                    }
                }
            }
            Expr::Name(_) => {
                if let Some(value) = self
                    .binding(value)
                    .and_then(|binding| self.references.values.get(&binding))
                {
                    self.push_written(value, depth + 1, written);
                }
            }
            Expr::Call(call)
                if self
                    .resolve(&call.func)
                    .is_some_and(|callee| matches(&callee, COMMANDS)) =>
            {
                if let Some(update) = call.arguments.find_keyword("update") {
                    self.push_written(&update.value, depth + 1, written);
                }
            }
            Expr::If(if_) => {
                self.push_written(&if_.body, depth + 1, written);
                self.push_written(&if_.orelse, depth + 1, written);
            }
            _ => {}
        }
    }

    /// The model a call to a method of `receiver` reaches, if `receiver` is a model.
    fn trace(&self, receiver: &'a Expr, depth: usize) -> Option<Traced> {
        if depth > MAX_DEPTH {
            return None;
        }
        match receiver {
            Expr::Call(call) => {
                if let Expr::Attribute(attribute) = call.func.as_ref() {
                    let method = attribute.attr.as_str();
                    if CONFIGURES.contains(&method) {
                        // A model configured as only models are is a model, even when
                        // where it came from isn't known.
                        let mut traced = self.trace(&attribute.value, depth + 1).unwrap_or_default();
                        let argument = call.arguments.args.first();
                        match method {
                            "with_structured_output" => {
                                traced.output = argument
                                    .map(|schema| self.callable_name(schema).unwrap_or_else(|| self.text(schema)));
                                traced.certain = true;
                            }
                            "bind_tools" => {
                                traced.tools = argument.map(|tools| self.tools(tools, 0)).unwrap_or_default();
                                traced.certain = true;
                            }
                            _ => {}
                        }
                        return Some(traced);
                    }
                }
                if let Some((function, _)) = self.function(&call.func) {
                    // A function in this file: whatever it returns. The first `return`
                    // that's a model wins, as with `return cached or create()`.
                    let returns = self.references.returns.get(&function)?;
                    return returns.iter().find_map(|value| self.trace(value, depth + 1));
                }
                let callee = self.resolve(&call.func)?;
                if matches(&callee, CHAT_MODELS) {
                    return Some(Traced {
                        model: Model {
                            client: Some(callee.to_string()),
                            id: model_id(call),
                            factory: None,
                        },
                        certain: true,
                        ..Traced::default()
                    });
                }
                // A function from another module, which `link` looks up.
                let binding = self.binding_of_root(&call.func)?;
                matches!(
                    self.checker.semantic.binding(binding).kind,
                    BindingKind::Import(_) | BindingKind::FromImport(_) | BindingKind::SubmoduleImport(_)
                )
                .then(|| Traced {
                    model: Model {
                        factory: Some(callee.to_string()),
                        ..Model::default()
                    },
                    ..Traced::default()
                })
            }
            Expr::Name(_) => {
                let value = self.references.values.get(&self.binding(receiver)?)?;
                self.trace(value, depth + 1)
            }
            // `prompt | llm | parser`
            Expr::BinOp(ast::ExprBinOp {
                left,
                op: ast::Operator::BitOr,
                right,
                ..
            }) => self.trace(left, depth + 1).or_else(|| self.trace(right, depth + 1)),
            _ => None,
        }
    }

    /// The names of the tools in `expr`: a list of tools, or a variable holding one.
    fn tools(&self, expr: &'a Expr, depth: usize) -> Vec<String> {
        let mut tools = Vec::new();
        self.push_tools(expr, depth, &mut tools);
        tools
    }

    fn push_tools(&self, expr: &'a Expr, depth: usize, tools: &mut Vec<String>) {
        if depth > MAX_DEPTH {
            return;
        }
        match expr {
            Expr::Name(_) => {
                let Some(binding) = self.binding(expr) else {
                    return;
                };
                if self.references.functions.contains_key(&binding) {
                    push_unique(tools, self.tool_name(expr));
                    return;
                }
                if let Some(value) = self.references.values.get(&binding) {
                    self.push_tools(value, depth + 1, tools);
                }
                for (method, argument) in self.appended.get(&binding).into_iter().flatten() {
                    match *method {
                        "append" => push_unique(tools, self.tool_name(argument)),
                        _ => self.push_tools(argument, depth + 1, tools),
                    }
                }
            }
            Expr::If(if_) => {
                self.push_tools(&if_.body, depth + 1, tools);
                self.push_tools(&if_.orelse, depth + 1, tools);
            }
            Expr::BinOp(ast::ExprBinOp {
                left,
                op: ast::Operator::Add,
                right,
                ..
            }) => {
                self.push_tools(left, depth + 1, tools);
                self.push_tools(right, depth + 1, tools);
            }
            Expr::Starred(starred) => self.push_tools(&starred.value, depth + 1, tools),
            _ => {
                for element in elements(expr).unwrap_or_default() {
                    match element {
                        Expr::Starred(starred) => self.push_tools(&starred.value, depth + 1, tools),
                        _ => push_unique(tools, self.tool_name(element)),
                    }
                }
            }
        }
    }

    /// A tool's name: the name given to `@tool("name")`, or its function's.
    fn tool_name(&self, expr: &'a Expr) -> String {
        if let Some((_, function)) = self.function(expr) {
            for decorator in &function.decorator_list {
                let Expr::Call(call) = &decorator.expression else {
                    continue;
                };
                let is_tool = self
                    .resolve(&call.func)
                    .is_some_and(|name| matches(&name, TOOL_DECORATORS));
                if let (true, Some(name)) = (is_tool, call.arguments.args.first().and_then(string_literal)) {
                    return name.to_string();
                }
            }
            return function.name.to_string();
        }
        self.callable_name(expr).unwrap_or_else(|| self.text(expr))
    }

    // -----------------------------------------------------------------------
    // Names
    // -----------------------------------------------------------------------

    /// The binding a name read refers to.
    fn binding(&self, expr: &Expr) -> Option<BindingId> {
        match expr {
            Expr::Name(name) => self.references.names.get(&name.range.start()).copied(),
            _ => None,
        }
    }

    /// The binding the first name of `a.b.c` refers to.
    fn binding_of_root(&self, expr: &Expr) -> Option<BindingId> {
        match expr {
            Expr::Attribute(attribute) => self.binding_of_root(&attribute.value),
            _ => self.binding(expr),
        }
    }

    /// What `a.b.c` resolves to, where it's written.
    fn resolve(&self, expr: &'a Expr) -> Option<QualifiedName<'a>> {
        let segments = collect_dotted_name(expr)?;
        let binding = self.binding_of_root(expr)?;
        self.checker.semantic.resolve_through(binding, &segments)
    }

    /// The function defined in this file that `expr` names.
    fn function(&self, expr: &Expr) -> Option<(BindingId, &'a ast::StmtFunctionDef)> {
        let binding = self.binding(expr)?;
        Some((binding, self.references.functions.get(&binding)?))
    }

    /// The innermost function `scope` is in.
    fn function_of(&self, scope: ScopeId) -> Option<&'a ast::StmtFunctionDef> {
        self.checker
            .semantic
            .ancestor_ids(scope)
            .find_map(|scope| self.references.scopes.get(&scope))
            .and_then(|binding| self.references.functions.get(binding).copied())
    }

    /// The name of a node, as `add_edge` and `path_map` refer to it: a string, `START`
    /// or `END`, or a constant holding a string.
    fn node_name(&self, expr: &'a Expr) -> Option<String> {
        if let Some(name) = self.resolve(expr) {
            if matches(&name, STARTS) {
                return Some(START.to_string());
            }
            if matches(&name, ENDS) {
                return Some(END.to_string());
            }
        }
        self.string(expr)
    }

    /// A string literal, or a variable holding one.
    fn string(&self, expr: &'a Expr) -> Option<String> {
        if let Some(text) = string_literal(expr) {
            return Some(text.to_string());
        }
        let binding = self.binding(expr)?;
        prompts::single_text(self.checker.binding_text(binding)?).map(str::to_string)
    }

    /// The name a function or class is known by: `triage` for `triage` or `nodes.triage`.
    fn callable_name(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(name) => Some(name.id.to_string()),
            Expr::Attribute(attribute) => Some(attribute.attr.to_string()),
            _ => None,
        }
    }

    fn binding_name(&self, binding: BindingId) -> &str {
        &self.source[self.checker.semantic.binding(binding).range]
    }

    /// `expr` as written, on one line.
    fn text(&self, expr: &impl Ranged) -> String {
        self.source[expr.range()]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn location(&self, offset: TextSize) -> Location {
        self.lines.location(offset)
    }
}

/// A node with nowhere to go ends the graph, and LangGraph draws an edge from it to the
/// end. Only for graphs whose every edge is known.
fn finish(graph: &mut StateGraph) {
    let mut finished = Vec::new();
    for node in &graph.nodes {
        if !graph.edges.iter().any(|edge| edge.from == node.name) {
            finished.push(Edge::direct(node.name.clone(), END.to_string(), node.location));
        }
    }
    graph.edges.extend(finished);
}

/// Whether two steps of a node are the same thing done twice, such as a model called
/// again to retry.
fn same_step(a: &Step, b: &Step) -> bool {
    match (a, b) {
        (Step::Model(a), Step::Model(b)) => a.model == b.model && a.output == b.output && a.tools == b.tools,
        (Step::Call { name: a, .. }, Step::Call { name: b, .. })
        | (Step::Node { name: a, .. }, Step::Node { name: b, .. }) => a == b,
        (Step::Prompt(a), Step::Prompt(b)) => a == b,
        (Step::Parallel { functions: a, .. }, Step::Parallel { functions: b, .. }) => a == b,
        _ => false,
    }
}

/// What a node does.
struct Inside {
    steps: Vec<Step>,
    reads: Vec<String>,
    /// Each key written, and the string written when it's a literal.
    written: Vec<(String, Option<String>)>,
}

/// Walks a node's function, and the functions in this file it calls.
struct InsideVisitor<'e, 'c, 'a> {
    extractor: &'e Extractor<'c, 'a>,
    /// The functions of the graph's nodes.
    nodes: &'e FxHashMap<BindingId, &'e str>,
    /// Where the node's state parameter is bound.
    state: Option<TextRange>,
    steps: Vec<(TextSize, Step)>,
    reads: Vec<String>,
    /// Functions in this file called by the node, to visit.
    queue: Vec<BindingId>,
    visited: FxHashSet<BindingId>,
    /// The functions visited, where prompts in them are found.
    ranges: Vec<TextRange>,
    /// Each executor functions are submitted to, and where it's first used.
    executors: FxHashMap<BindingId, TextSize>,
}

impl InsideVisitor<'_, '_, '_> {
    /// Whether `expr` is the node's state parameter.
    fn is_state(&self, expr: &Expr) -> bool {
        let Some(binding) = self.extractor.binding(expr) else {
            return false;
        };
        Some(self.extractor.checker.semantic.binding(binding).range) == self.state
    }

    /// A call at `start` to the function `binding` in this file: another node's, or a
    /// helper whose body is visited as part of this node.
    fn call_function(&mut self, binding: BindingId, start: TextSize) {
        if let Some(&node) = self.nodes.get(&binding) {
            let location = self.extractor.location(start);
            self.steps.push((
                start,
                Step::Node {
                    name: node.to_string(),
                    location,
                },
            ));
        } else if self.visited.insert(binding) {
            self.queue.push(binding);
        }
    }

    fn read(&mut self, key: &str) {
        push_unique(&mut self.reads, key.to_string());
    }

    /// Adds `function` to the functions started together at `start`.
    fn parallel(&mut self, start: TextSize, function: String) {
        let step = self
            .steps
            .iter_mut()
            .find(|(at, step)| *at == start && matches!(step, Step::Parallel { .. }));
        if let Some((_, Step::Parallel { functions, .. })) = step {
            push_unique(functions, function);
            return;
        }
        let location = self.extractor.location(start);
        self.steps.push((
            start,
            Step::Parallel {
                functions: vec![function],
                location,
            },
        ));
    }
}

impl<'a> Visitor<'a> for InsideVisitor<'_, '_, 'a> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Call(call) => self.visit_call(call),
            Expr::Subscript(subscript) if self.is_state(&subscript.value) => {
                if let Some(key) = string_literal(&subscript.slice) {
                    self.read(key);
                }
            }
            _ => {}
        }
        visitor::walk_expr(self, expr);
    }
}

impl<'a> InsideVisitor<'_, '_, 'a> {
    fn visit_call(&mut self, call: &'a ast::ExprCall) {
        let extractor = self.extractor;
        let start = call.start();
        if let Expr::Attribute(attribute) = call.func.as_ref() {
            let method = attribute.attr.as_str();
            if INVOKES.contains(&method) {
                if let Some(traced) = extractor.trace(&attribute.value, 0) {
                    let call = ModelCall {
                        location: extractor.location(start),
                        model: traced.model,
                        output: traced.output,
                        tools: traced.tools,
                        certain: traced.certain,
                    };
                    self.steps.push((start, Step::Model(call)));
                    return;
                }
            }
            if method == "get" && self.is_state(&attribute.value) {
                if let Some(key) = call.arguments.args.first().and_then(string_literal) {
                    self.read(key);
                }
                return;
            }
            // `executor.submit(f, ...)`: the functions submitted to one executor run
            // together, from its first `submit`.
            if method == "submit" {
                let submitted = call.arguments.args.first();
                if let Some(function) = submitted.and_then(|function| extractor.callable_name(function)) {
                    let first = match extractor.binding(&attribute.value) {
                        Some(executor) => *self.executors.entry(executor).or_insert(start),
                        None => start,
                    };
                    self.parallel(first, function);
                    // What it runs is part of the node, as if it were called here.
                    if let Some((binding, _)) = submitted.and_then(|function| extractor.function(function)) {
                        self.call_function(binding, start);
                    }
                    return;
                }
            }
        }
        if let Some((binding, _)) = extractor.function(&call.func) {
            self.call_function(binding, start);
            return;
        }
        let Some(callee) = extractor.resolve(&call.func) else {
            return;
        };
        if matches(&callee, GATHERS) {
            for argument in &call.arguments.args {
                if let Expr::Call(inner) = argument {
                    if let Some(function) = extractor.callable_name(&inner.func) {
                        self.parallel(start, function);
                    }
                }
            }
            return;
        }
        // Calls to other modules: `link` keeps the ones to the project's own.
        let imported = extractor.binding_of_root(&call.func).is_some_and(|binding| {
            matches!(
                extractor.checker.semantic.binding(binding).kind,
                BindingKind::Import(_) | BindingKind::FromImport(_) | BindingKind::SubmoduleImport(_)
            )
        });
        if imported {
            let location = extractor.location(start);
            self.steps.push((
                start,
                Step::Call {
                    name: callee.to_string(),
                    location,
                },
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Linking
// ---------------------------------------------------------------------------

/// Looks up the models made by functions in other modules, and keeps only the calls
/// to the project's own modules.
pub(crate) fn link(results: &mut [FileResult]) {
    if results.iter().all(|result| result.graphs.is_empty()) {
        return;
    }
    // A module's name depends on which directory was scanned, so each is indexed by
    // every suffix of its name, as imported agents are.
    let mut factories: FxHashMap<String, Model> = FxHashMap::default();
    for result in results.iter() {
        for factory in &result.model_factories {
            for suffix in suffixes(&factory.function).filter(|suffix| suffix.contains('.')) {
                factories
                    .entry(suffix.to_string())
                    .or_insert_with(|| factory.model.clone());
            }
        }
    }
    let mut modules: FxHashSet<String> = FxHashSet::default();
    for result in results.iter() {
        for suffix in suffixes(&result.module) {
            modules.insert(suffix.to_string());
        }
    }
    let factory = |name: &str| {
        suffixes(name)
            .filter(|suffix| suffix.contains('.'))
            .find_map(|suffix| factories.get(suffix))
    };

    for result in results.iter_mut() {
        for graph in &mut result.graphs {
            for node in &mut graph.nodes {
                node.steps.retain_mut(|step| match step {
                    Step::Model(call) => {
                        if let Some(model) = call.model.factory.as_deref().and_then(factory) {
                            call.model.client.clone_from(&model.client);
                            call.model.id.clone_from(&model.id);
                            call.certain = true;
                        }
                        call.certain
                    }
                    Step::Call { name, .. } => {
                        let module = name.rsplit_once('.').map_or("", |(module, _)| module);
                        modules.contains(module) && factory(name).is_none()
                    }
                    _ => true,
                });
            }
        }
    }
}

/// `a.b.c`, `b.c`, `c`.
fn suffixes(name: &str) -> impl Iterator<Item = &str> {
    std::iter::once(name).chain(name.match_indices('.').map(|(index, _)| &name[index + 1..]))
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::PySourceType;
    use ruff_python_parser::parse_unchecked_source;

    use super::*;
    use crate::checker::check_module_with_references;

    fn graphs(source: &str) -> Vec<StateGraph> {
        let parsed = parse_unchecked_source(source, PySourceType::Python);
        assert!(parsed.has_valid_syntax(), "{:?}", parsed.errors());
        let checker = check_module_with_references(parsed.syntax(), vec!["app"]);
        extract(&checker, source, &LineIndex::new(source))
    }

    fn graph(source: &str) -> StateGraph {
        let mut graphs = graphs(source);
        assert_eq!(graphs.len(), 1);
        graphs.remove(0)
    }

    /// `from -> to`, with `-.->` for a conditional edge and the label after `|`.
    fn edges(graph: &StateGraph) -> Vec<String> {
        graph
            .edges
            .iter()
            .map(|edge| {
                let arrow = if edge.conditional { "-.->" } else { "->" };
                match &edge.label {
                    Some(label) => format!("{} {arrow} {} | {label}", edge.from, edge.to),
                    None => format!("{} {arrow} {}", edge.from, edge.to),
                }
            })
            .collect()
    }

    fn node<'g>(graph: &'g StateGraph, name: &str) -> &'g Node {
        graph
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| panic!("no node {name}"))
    }

    fn edge<'g>(graph: &'g StateGraph, from: &str, to: &str) -> &'g Edge {
        graph
            .edges
            .iter()
            .find(|edge| edge.from == from && edge.to == to)
            .unwrap_or_else(|| panic!("no edge {from} -> {to}"))
    }

    const HEADER: &str = "from typing import Literal\n\
                          from langgraph.graph import StateGraph, START, END\n\
                          from langgraph.types import Command, Send\n";

    #[test]
    fn reads_nodes_and_edges_as_langgraph_adds_them() {
        let graph = graph(&format!(
            "{HEADER}\
             def plan(state): return {{}}\n\
             def act(state): return {{}}\n\
             def check(state): return {{}}\n\
             def report(state): return {{}}\n\
             builder = StateGraph(State)\n\
             builder.add_node(\"plan\", plan)\n\
             builder.add_node(act)\n\
             builder.add_sequence([check, (\"report\", report)])\n\
             builder.set_entry_point(\"plan\")\n\
             builder.add_edge([\"plan\", \"act\"], \"check\")\n\
             builder.add_edge(\"report\", END)\n\
             graph = builder.compile()\n"
        ));
        assert_eq!(graph.variable.as_deref(), Some("builder"));
        assert_eq!(graph.state.as_deref(), Some("State"));
        let names: Vec<_> = graph.nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["plan", "act", "check", "report"]);
        assert_eq!(
            node(&graph, "act").kind,
            NodeKind::Function {
                name: "act".to_string(),
                location: Some(Location { line: 5, column: 5 }),
            }
        );
        assert_eq!(
            edges(&graph),
            [
                "check -> report",
                "__start__ -> plan",
                "plan -> check",
                "act -> check",
                "report -> __end__",
            ]
        );
    }

    #[test]
    fn a_router_branches_where_its_returns_say_and_unreturned_entries_are_unreachable() {
        let graph = graph(&format!(
            "{HEADER}\
             def route(state):\n\
             \x20   if state[\"done\"]:\n\
             \x20       return END\n\
             \x20   elif state.get(\"retries\", 0) > 3:\n\
             \x20       return \"give_up\"\n\
             \x20   return \"work\"\n\
             def work(state): return {{}}\n\
             g = StateGraph(S)\n\
             g.add_node(\"work\", work)\n\
             g.add_edge(START, \"work\")\n\
             g.add_conditional_edges(\"work\", route, {{\"work\": \"work\", \"give_up\": END, \"escalate\": \"work\", END: END}})\n"
        ));
        assert_eq!(
            edges(&graph),
            [
                "__start__ -> work",
                "work -.-> work",
                "work -.-> __end__ | give_up",
                "work -.-> work | escalate",
                "work -.-> __end__",
            ]
        );
        let give_up = graph
            .edges
            .iter()
            .find(|edge| edge.label.as_deref() == Some("give_up"))
            .unwrap();
        assert_eq!(give_up.conditions, [Some("state.get(\"retries\", 0) > 3".to_string())]);
        assert_eq!(give_up.router.as_deref(), Some("route"));
        let escalate = graph
            .edges
            .iter()
            .find(|edge| edge.label.as_deref() == Some("escalate"))
            .unwrap();
        assert!(!escalate.reachable);
        // The last `return` has no `if` around it.
        let unlabelled = |to: &str| {
            graph
                .edges
                .iter()
                .find(|edge| edge.to == to && edge.label.is_none() && edge.conditional)
        };
        assert_eq!(unlabelled("work").unwrap().conditions, [None]);
        assert_eq!(
            unlabelled(END).unwrap().conditions,
            [Some("state[\"done\"]".to_string())]
        );
    }

    #[test]
    fn without_a_path_map_the_targets_are_what_the_router_returns_or_declares() {
        let graph = graph(&format!(
            "{HEADER}\
             def route(state) -> Literal[\"a\", \"b\"]:\n\
             \x20   return \"a\" if state[\"x\"] else \"b\"\n\
             def fan_out(state):\n\
             \x20   return [Send(\"c\", {{\"item\": item}}) for item in state[\"items\"]]\n\
             def a(state): return {{}}\n\
             def b(state): return {{}}\n\
             def c(state): return {{}}\n\
             g = StateGraph(S)\n\
             g.add_node(\"a\", a)\n\
             g.add_node(\"b\", b)\n\
             g.add_node(\"c\", c)\n\
             g.add_conditional_edges(START, route)\n\
             g.add_conditional_edges(\"a\", fan_out)\n"
        ));
        assert_eq!(
            edges(&graph),
            [
                "__start__ -.-> a",
                "__start__ -.-> b",
                "a -.-> c",
                "b -> __end__",
                "c -> __end__"
            ]
        );
        assert_eq!(edge(&graph, START, "a").conditions, [Some("state[\"x\"]".to_string())]);
        assert_eq!(
            edge(&graph, START, "b").conditions,
            [Some("not (state[\"x\"])".to_string())]
        );
    }

    #[test]
    fn a_router_returning_a_state_key_goes_where_the_nodes_set_it() {
        let graph = graph(&format!(
            "{HEADER}\
             def draft(state): return {{\"last\": \"draft\", \"text\": \"...\"}}\n\
             def revise(state): return {{\"last\": \"revise\"}}\n\
             def check(state): return {{}}\n\
             def route(state):\n\
             \x20   if state.get(\"feedback\"):\n\
             \x20       return state[\"last\"]\n\
             \x20   return END\n\
             g = StateGraph(S)\n\
             g.add_node(\"draft\", draft)\n\
             g.add_node(\"revise\", revise)\n\
             g.add_node(\"check\", check)\n\
             g.add_conditional_edges(\"check\", route)\n"
        ));
        assert_eq!(
            edges(&graph),
            [
                "check -.-> draft",
                "check -.-> revise",
                "check -.-> __end__",
                "draft -> __end__",
                "revise -> __end__"
            ]
        );
        assert_eq!(
            edge(&graph, "check", "revise").conditions,
            [Some("state.get(\"feedback\")".to_string())]
        );
        assert_eq!(node(&graph, "draft").writes, ["last", "text"]);
    }

    #[test]
    fn nodes_route_themselves_with_commands_and_destinations() {
        let graph = graph(&format!(
            "{HEADER}\
             def triage(state) -> Command[Literal[\"urgent\", \"routine\"]]:\n\
             \x20   if state[\"severe\"]:\n\
             \x20       return Command(goto=\"urgent\", update={{\"level\": \"high\"}})\n\
             \x20   return Command(goto=\"routine\")\n\
             def urgent(state): return {{}}\n\
             def routine(state): return {{}}\n\
             def review(state): return {{}}\n\
             g = StateGraph(S)\n\
             g.add_node(\"triage\", triage)\n\
             g.add_node(\"urgent\", urgent, destinations=(\"review\",))\n\
             g.add_node(\"routine\", routine)\n\
             g.add_node(\"review\", review)\n"
        ));
        assert_eq!(
            edges(&graph),
            [
                "urgent -.-> review",
                "triage -.-> urgent",
                "triage -.-> routine",
                // Nodes with nowhere to go end the graph.
                "routine -> __end__",
                "review -> __end__",
            ]
        );
        assert_eq!(
            edge(&graph, "triage", "urgent").conditions,
            [None, Some("state[\"severe\"]".to_string())]
        );
        assert_eq!(node(&graph, "triage").writes, ["level"]);
    }

    #[test]
    fn tool_nodes_and_tools_condition() {
        let graph = graph(
            "from langgraph.graph import StateGraph, END\n\
             from langgraph.prebuilt import ToolNode, tools_condition\n\
             from langchain_core.tools import tool\n\
             @tool\n\
             def search(query: str): \"\"\"Search.\"\"\"\n\
             @tool(\"lookup_case\")\n\
             def lookup(case_id: str): \"\"\"Look up.\"\"\"\n\
             def notify(): pass\n\
             tools = [search]\n\
             tools.append(lookup)\n\
             def agent(state): return {}\n\
             g = StateGraph(S)\n\
             g.add_node(\"agent\", agent)\n\
             g.add_node(\"tools\", ToolNode(tools + [notify] if urgent else tools))\n\
             g.add_conditional_edges(\"agent\", tools_condition)\n\
             g.add_edge(\"tools\", \"agent\")\n",
        );
        assert_eq!(
            node(&graph, "tools").kind,
            NodeKind::Tools(vec![
                "search".to_string(),
                "lookup_case".to_string(),
                "notify".to_string()
            ])
        );
        assert_eq!(
            edges(&graph),
            ["agent -.-> tools", "agent -.-> __end__", "tools -> agent"]
        );
        assert_eq!(
            edge(&graph, "agent", "tools").router.as_deref(),
            Some("tools_condition")
        );
    }

    #[test]
    fn what_a_node_does() {
        let graph = graph(
            "import concurrent.futures\n\
             from langgraph.graph import StateGraph\n\
             from langchain_openai import ChatOpenAI\n\
             from langchain_core.messages import SystemMessage\n\
             from app.rag import lookup\n\
             def make_llm():\n\
             \x20   cached = CACHE.get(\"llm\")\n\
             \x20   if cached:\n\
             \x20       return cached\n\
             \x20   return ChatOpenAI(model=\"gpt-4o\")\n\
             def classify(state):\n\
             \x20   llm = make_llm().with_structured_output(Intent)\n\
             \x20   return llm.invoke([SystemMessage(\"Classify the message.\")])\n\
             def guard(text): return lookup(text)\n\
             def route(state):\n\
             \x20   with concurrent.futures.ThreadPoolExecutor() as executor:\n\
             \x20       a = executor.submit(classify, state)\n\
             \x20       b = executor.submit(guard, state[\"text\"])\n\
             \x20   answer = make_llm().bind_tools([search]).invoke(state.get(\"messages\"))\n\
             \x20   app.invoke(state)\n\
             \x20   return {\"intent\": a.result(), \"answer\": answer}\n\
             g = StateGraph(S)\n\
             g.add_node(\"route\", route)\n",
        );
        let route = node(&graph, "route");
        let steps: Vec<String> = route
            .steps
            .iter()
            // Calls to libraries are dropped by `link`, which this doesn't run.
            .filter(|step| !matches!(step, Step::Call { name, .. } if !name.starts_with("app.")))
            .map(|step| match step {
                Step::Model(call) => format!(
                    "model {} -> {:?} tools {:?}",
                    call.model.id.as_deref().unwrap_or("?"),
                    call.output,
                    call.tools
                ),
                Step::Prompt(location) => format!("prompt at {location}"),
                Step::Call { name, .. } => format!("call {name}"),
                Step::Node { name, .. } => format!("node {name}"),
                Step::Parallel { functions, .. } => format!("parallel {}", functions.join(", ")),
            })
            .collect();
        assert_eq!(
            steps,
            [
                // From `classify` and `guard`, the helpers `route` runs.
                "model gpt-4o -> Some(\"Intent\") tools []",
                "prompt at 13:38",
                "call app.rag.lookup",
                "parallel classify, guard",
                "model gpt-4o -> None tools [\"search\"]",
            ]
        );
        assert_eq!(route.reads, ["text", "messages"]);
        assert_eq!(route.writes, ["intent", "answer"]);
    }

    #[test]
    fn graphs_built_in_functions_with_nested_nodes() {
        let graph = graph(
            "def pipeline(message):\n\
             \x20   from langgraph.graph import StateGraph, START, END\n\
             \x20   def answer(state):\n\
             \x20       return {\"reply\": \"hi\"}\n\
             \x20   def build():\n\
             \x20       workflow = StateGraph(dict)\n\
             \x20       workflow.add_node(\"answer\", answer)\n\
             \x20       workflow.add_edge(START, \"answer\")\n\
             \x20       workflow.add_edge(\"answer\", END)\n\
             \x20       return workflow.compile()\n\
             \x20   return build().invoke({})\n",
        );
        assert_eq!(graph.function.as_deref(), Some("build"));
        assert_eq!(graph.name(), "build");
        assert_eq!(edges(&graph), ["__start__ -> answer", "answer -> __end__"]);
        assert_eq!(node(&graph, "answer").writes, ["reply"]);
    }

    #[test]
    fn a_compiled_graph_is_a_node_of_another() {
        let graphs = graphs(
            "from langgraph.graph import StateGraph, START\n\
             from langgraph.prebuilt import create_react_agent\n\
             def research():\n\
             \x20   inner = StateGraph(S)\n\
             \x20   return inner\n\
             inner = StateGraph(S)\n\
             researcher = inner.compile()\n\
             outer = StateGraph(S)\n\
             outer.add_node(\"research\", researcher)\n\
             outer.add_node(\"helper\", create_react_agent(\"gpt-4o\", tools=[]))\n",
        );
        let outer = graphs
            .iter()
            .find(|graph| graph.variable.as_deref() == Some("outer"))
            .unwrap();
        assert_eq!(node(outer, "research").kind, NodeKind::Subgraph("inner".to_string()));
        assert_eq!(
            node(outer, "helper").kind,
            NodeKind::Agent("langgraph.prebuilt.create_react_agent".to_string())
        );
    }

    #[test]
    fn names_known_only_at_runtime_leave_the_graph_without_implied_ends() {
        let graph = graph(
            "from langgraph.graph import StateGraph, START\n\
             def work(state): return {}\n\
             def build(name, next_node):\n\
             \x20   g = StateGraph(dict)\n\
             \x20   g.add_node(name, work)\n\
             \x20   g.add_node(\"done\", work)\n\
             \x20   g.add_edge(START, name)\n\
             \x20   g.add_edge(name, next_node)\n",
        );
        assert_eq!(graph.nodes[0].name, "{name}");
        // `{name}` may go anywhere, so `done` isn't known to be the last node.
        assert_eq!(edges(&graph), Vec::<String>::new());
    }

    #[test]
    fn files_without_a_graph_have_none() {
        assert!(graphs("from langchain.agents import create_agent\nagent = create_agent(model='x')\n").is_empty());
    }

    #[test]
    fn suffixes_of_a_name() {
        assert_eq!(suffixes("a.b.c").collect::<Vec<_>>(), ["a.b.c", "b.c", "c"]);
    }
}
