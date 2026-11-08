//! Tier 2, workflow: the parts of each graph and how work moves between them, built
//! from tier 1 alone. Every part lists the nodes it's made of, and every handoff the
//! edges it stands for, so each box can be followed down to the code.
//!
//! A node's role comes from its place in the graph and what it does:
//!
//! | Role         | The node                                                                      |
//! | ------------ | ----------------------------------------------------------------------------- |
//! | `guardrail`  | is where several nodes go next, can send their work back, and isn't the start  |
//! | `supervisor` | is the same, but the graph starts there                                       |
//! | `router`     | decides between two or more nodes                                             |
//! | `agent`      | calls a model                                                                 |
//! | `tools`      | is a `ToolNode`                                                               |
//! | `responder`  | answers without a model and ends the graph                                    |
//! | `step`       | does anything else                                                            |
//!
//! Then nodes that work as one become one part, by these rules in order:
//!
//! 1. **A tool loop**: an agent, the `ToolNode` that runs the tools bound to its model, and
//!    the steps from the tools back to it.
//! 2. **A shared resource**: agents calling the same project function, when at most half
//!    of the graph's agents call it. A function every agent calls, such as a clock, says
//!    nothing about which agents belong together.
//! 3. **A producer**: a node whose output, a state key few nodes write, is read only by
//!    one part, routers and guardrails aside, joins that part.
//!
//! A part is named after the word its nodes' names share most, as a team is after its
//! work: `triage_decision`, `post_triage` and `pre_approval` make the "Triage agent". Ties
//! go to the agent that runs the part's tools.

use rustc_hash::FxHashMap;

use crate::discovery::FileResult;
use crate::flow::{END, NodeKind, START, StateGraph, Step};
use crate::json::Json;

use super::code::{graphs, model_name};
use super::mermaid::{Flowchart, Shape, id};
use super::{last_segment, qualify, relative_path};

/// What a part does in the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Start,
    End,
    Guardrail,
    Supervisor,
    Router,
    Agent,
    Tools,
    Responder,
    Subgraph,
    Step,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Start => "start",
            Role::End => "end",
            Role::Guardrail => "guardrail",
            Role::Supervisor => "supervisor",
            Role::Router => "router",
            Role::Agent => "agent",
            Role::Tools => "tools",
            Role::Responder => "responder",
            Role::Subgraph => "graph",
            Role::Step => "step",
        }
    }

    /// What a part with this role and a single node is called when it's the only one.
    fn title(self) -> Option<&'static str> {
        match self {
            Role::Guardrail => Some("Guardrail"),
            Role::Supervisor => Some("Supervisor"),
            Role::Router => Some("Router"),
            _ => None,
        }
    }
}

/// The parts of one graph and the handoffs between them.
pub struct Workflow<'g> {
    pub graph: &'g StateGraph,
    pub parts: Vec<Part>,
    pub handoffs: Vec<Handoff>,
}

pub struct Part {
    pub name: String,
    pub role: Role,
    /// The nodes of tier 1 it's made of, in the order they're added to the graph.
    pub members: Vec<String>,
    /// The models its nodes call.
    pub models: Vec<String>,
    /// The tools bound to its models and run by its tool nodes.
    pub tools: Vec<String>,
    /// The project functions its nodes call that most nodes don't, by qualified name,
    /// those called by more of its nodes first.
    pub uses: Vec<String>,
    /// The functions it runs in parallel.
    pub parallel: Vec<String>,
    /// Whether it loops through its own tools.
    pub tool_loop: bool,
}

pub struct Handoff {
    /// Indices into [`Workflow::parts`].
    pub from: usize,
    pub to: usize,
    pub conditional: bool,
    /// A guardrail sending a part's work back to it.
    pub retry: bool,
    /// What it's taken on: the enum values and quoted constants its conditions test,
    /// and the `path_map` keys that lead to it.
    pub when: Vec<String>,
    /// The edges of tier 1 it stands for, as indices into the graph's edges.
    pub edges: Vec<usize>,
}

/// The workflow of every graph in `results`, or of the one at index `only`, with the file
/// each is built in.
pub(crate) fn workflows(results: &[FileResult], only: Option<usize>) -> Vec<(&FileResult, Workflow<'_>)> {
    graphs(results, only).map(|(result, graph)| (result, build(graph))).collect()
}

/// Groups a graph's nodes into parts.
pub fn build(graph: &StateGraph) -> Workflow<'_> {
    let nodes = &graph.nodes;
    let index: FxHashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.name.as_str(), i))
        .collect();
    // The edges the graph can take, by node index. `None` is the start or the end.
    let edges: Vec<(usize, Option<usize>, Option<usize>)> = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| edge.reachable)
        .map(|(i, edge)| {
            (
                i,
                index.get(edge.from.as_str()).copied(),
                index.get(edge.to.as_str()).copied(),
            )
        })
        .collect();
    let roles: Vec<Role> = (0..nodes.len()).map(|node| role(graph, node, &index)).collect();

    let mut parts = Sets::new(nodes.len());
    join_tool_loops(graph, &roles, &index, &mut parts);
    join_shared_resources(graph, &roles, &mut parts);
    join_producers(graph, &roles, &mut parts);

    // Parts in the order of their first node; the start and the end around them.
    let mut part_of_root: FxHashMap<usize, usize> = FxHashMap::default();
    let mut members: Vec<Vec<usize>> = Vec::new();
    for node in 0..nodes.len() {
        let root = parts.find(node);
        let part = *part_of_root.entry(root).or_insert_with(|| {
            members.push(Vec::new());
            members.len() - 1
        });
        members[part].push(node);
    }
    let uses = narrow_calls(graph);
    let mut built: Vec<Part> = Vec::with_capacity(members.len() + 2);
    built.push(endpoint(START, Role::Start));
    for members in &members {
        built.push(part(graph, members, &roles, &uses));
    }
    built.push(endpoint(END, Role::End));
    name_parts(graph, &members, &mut built[1..members.len() + 1]);

    // A node's part, offset by the start.
    let part_of = |node: Option<usize>, start: bool| match node {
        Some(node) => part_of_root[&parts.find(node)] + 1,
        None if start => 0,
        None => built.len() - 1,
    };
    let mut handoffs: Vec<Handoff> = Vec::new();
    for &(edge, from, to) in &edges {
        let (from, to) = (part_of(from, true), part_of(to, false));
        if from == to {
            continue;
        }
        let tier1 = &graph.edges[edge];
        let handoff = match handoffs
            .iter_mut()
            .position(|handoff| handoff.from == from && handoff.to == to)
        {
            Some(existing) => &mut handoffs[existing],
            None => {
                handoffs.push(Handoff {
                    from,
                    to,
                    conditional: false,
                    retry: false,
                    when: Vec::new(),
                    edges: Vec::new(),
                });
                handoffs.last_mut().expect("pushed")
            }
        };
        handoff.conditional |= tier1.conditional;
        handoff.edges.push(edge);
        if let Some(label) = &tier1.label {
            push_new(&mut handoff.when, label.clone());
        }
        for condition in tier1.conditions.iter().flatten() {
            for value in tested_values(condition) {
                push_new(&mut handoff.when, value.to_string());
            }
        }
    }
    // A guardrail sending work back where it came from.
    let returns: Vec<(usize, usize)> = handoffs.iter().map(|handoff| (handoff.from, handoff.to)).collect();
    for handoff in &mut handoffs {
        handoff.retry = built[handoff.from].role == Role::Guardrail && returns.contains(&(handoff.to, handoff.from));
    }
    Workflow {
        graph,
        parts: built,
        handoffs,
    }
}

fn endpoint(name: &str, role: Role) -> Part {
    Part {
        name: name.to_string(),
        role,
        members: vec![name.to_string()],
        models: Vec::new(),
        tools: Vec::new(),
        uses: Vec::new(),
        parallel: Vec::new(),
        tool_loop: false,
    }
}

/// The role of `node` on its own.
fn role(graph: &StateGraph, node: usize, index: &FxHashMap<&str, usize>) -> Role {
    let this = &graph.nodes[node];
    match this.kind {
        NodeKind::Tools(_) => return Role::Tools,
        NodeKind::Subgraph(_) => return Role::Subgraph,
        NodeKind::Agent(_) => return Role::Agent,
        NodeKind::Function { .. } | NodeKind::Other(_) => {}
    }
    let name = this.name.as_str();
    let reachable = || graph.edges.iter().filter(|edge| edge.reachable);
    // The nodes that go here directly, and whether this sends work back to one of them.
    let mut sources: Vec<&str> = reachable()
        .filter(|edge| edge.to == name && !edge.conditional && edge.from != START && edge.from != name)
        .map(|edge| edge.from.as_str())
        .collect();
    sources.sort_unstable();
    sources.dedup();
    let sends_back =
        reachable().any(|edge| edge.from == name && edge.conditional && sources.contains(&edge.to.as_str()));
    if sources.len() >= 2 && sends_back {
        let entered_from_start = reachable().any(|edge| edge.from == START && edge.to == name);
        return if entered_from_start {
            Role::Supervisor
        } else {
            Role::Guardrail
        };
    }
    let mut choices: Vec<&str> = reachable()
        .filter(|edge| edge.from == name && edge.conditional && edge.to != END && index.contains_key(edge.to.as_str()))
        .map(|edge| edge.to.as_str())
        .collect();
    choices.sort_unstable();
    choices.dedup();
    if choices.len() >= 2 {
        return Role::Router;
    }
    if this.steps.iter().any(|step| matches!(step, Step::Model(_))) {
        return Role::Agent;
    }
    if reachable().any(|edge| edge.from == name && edge.to == END) {
        return Role::Responder;
    }
    Role::Step
}

/// Rule 1: an agent, the tool node running its model's tools, and the steps from the
/// tools back to the agent.
fn join_tool_loops(graph: &StateGraph, roles: &[Role], index: &FxHashMap<&str, usize>, parts: &mut Sets) {
    for (tools_node, node) in graph.nodes.iter().enumerate() {
        let NodeKind::Tools(tools) = &node.kind else { continue };
        let owner = graph.nodes.iter().enumerate().find_map(|(agent, node)| {
            let bound = node.steps.iter().any(|step| match step {
                Step::Model(call) => call.tools.iter().any(|tool| tools.contains(tool)),
                _ => false,
            });
            (bound && roles[agent] == Role::Agent).then_some(agent)
        });
        let Some(owner) = owner else { continue };
        parts.union(tools_node, owner);
        // Steps that only pass the tools' results on, until the agent.
        let mut path = Vec::new();
        let mut current = tools_node;
        while path.len() < graph.nodes.len() {
            let next: Vec<usize> = graph
                .edges
                .iter()
                .filter(|edge| edge.reachable && edge.from == graph.nodes[current].name)
                .filter_map(|edge| index.get(edge.to.as_str()).copied())
                .collect();
            let [next] = next.as_slice() else { break };
            if *next == owner {
                for step in path {
                    parts.union(step, owner);
                }
                break;
            }
            if roles[*next] != Role::Step {
                break;
            }
            path.push(*next);
            current = *next;
        }
    }
}

/// Rule 2: agents that call the same project function, when at most half of the
/// graph's agents call it.
fn join_shared_resources(graph: &StateGraph, roles: &[Role], parts: &mut Sets) {
    let agents: Vec<usize> = (0..graph.nodes.len())
        .filter(|&node| roles[node] == Role::Agent)
        .collect();
    let mut callers: FxHashMap<&str, Vec<usize>> = FxHashMap::default();
    for &agent in &agents {
        for step in &graph.nodes[agent].steps {
            if let Step::Call { name, .. } = step {
                let callers = callers.entry(name.as_str()).or_default();
                if callers.last() != Some(&agent) {
                    callers.push(agent);
                }
            }
        }
    }
    let mut shared: Vec<&Vec<usize>> = callers
        .values()
        .filter(|callers| callers.len() >= 2 && callers.len() * 2 <= agents.len() + 1)
        .collect();
    // The same groups whatever order the map holds them in.
    shared.sort();
    for callers in shared {
        for &caller in &callers[1..] {
            parts.union(callers[0], caller);
        }
    }
}

/// Rule 3: a node whose output only one part reads joins it.
fn join_producers(graph: &StateGraph, roles: &[Role], parts: &mut Sets) {
    let nodes = &graph.nodes;
    let deciders = |node: usize| matches!(roles[node], Role::Router | Role::Guardrail | Role::Supervisor);
    let mut writers: FxHashMap<&str, usize> = FxHashMap::default();
    for node in nodes {
        for key in &node.writes {
            *writers.entry(key.as_str()).or_default() += 1;
        }
    }
    for producer in 0..nodes.len() {
        if deciders(producer) || !matches!(roles[producer], Role::Agent | Role::Step) || parts.size(producer) > 1 {
            continue;
        }
        let mut joined = None;
        for key in &nodes[producer].writes {
            if writers[key.as_str()] * 2 > nodes.len() {
                continue;
            }
            let mut readers = (0..nodes.len())
                .filter(|&reader| reader != producer && !deciders(reader) && nodes[reader].reads.contains(key))
                .map(|reader| parts.find(reader));
            let Some(first) = readers.next() else { continue };
            if readers.all(|part| part == first) && parts.size(first) > 1 {
                joined = Some(first);
                break;
            }
        }
        if let Some(part) = joined {
            parts.union(producer, part);
        }
    }
}

/// The project functions called by at most half of the graph's nodes: the ones that say
/// something about a part.
fn narrow_calls(graph: &StateGraph) -> Vec<&str> {
    let mut callers: FxHashMap<&str, usize> = FxHashMap::default();
    for node in &graph.nodes {
        let mut seen: Vec<&str> = Vec::new();
        for step in &node.steps {
            if let Step::Call { name, .. } = step {
                if !seen.contains(&name.as_str()) {
                    seen.push(name);
                    *callers.entry(name).or_default() += 1;
                }
            }
        }
    }
    let mut narrow: Vec<&str> = callers
        .into_iter()
        .filter(|&(_, callers)| callers * 2 <= graph.nodes.len())
        .map(|(name, _)| name)
        .collect();
    narrow.sort_unstable();
    narrow
}

fn part(graph: &StateGraph, members: &[usize], roles: &[Role], narrow: &[&str]) -> Part {
    let role = match members {
        [node] => roles[*node],
        _ => Role::Agent,
    };
    let mut part = Part {
        name: String::new(),
        role,
        members: members.iter().map(|&node| graph.nodes[node].name.clone()).collect(),
        models: Vec::new(),
        tools: Vec::new(),
        uses: Vec::new(),
        parallel: Vec::new(),
        tool_loop: members.len() > 1 && members.iter().any(|&node| roles[node] == Role::Tools),
    };
    // How many of the part's nodes call each function.
    let mut uses: Vec<(&str, usize)> = Vec::new();
    for &node in members {
        let node = &graph.nodes[node];
        let first_use = uses.len();
        if let NodeKind::Tools(tools) = &node.kind {
            for tool in tools {
                push_new(&mut part.tools, tool.clone());
            }
        }
        for step in &node.steps {
            match step {
                Step::Model(call) => {
                    push_new(&mut part.models, model_name(&call.model).to_string());
                    for tool in &call.tools {
                        push_new(&mut part.tools, tool.clone());
                    }
                }
                Step::Call { name, .. } if narrow.binary_search(&name.as_str()).is_ok() => {
                    match uses.iter_mut().position(|(used, _)| *used == name) {
                        Some(seen) if seen < first_use => uses[seen].1 += 1,
                        Some(_) => {}
                        None => uses.push((name, 1)),
                    }
                }
                Step::Parallel { functions, .. } => {
                    for function in functions {
                        push_new(&mut part.parallel, function.clone());
                    }
                }
                _ => {}
            }
        }
    }
    // A stable sort keeps the order they're first used in among equals.
    uses.sort_by_key(|(_, callers)| std::cmp::Reverse(*callers));
    part.uses = uses.into_iter().map(|(name, _)| name.to_string()).collect();
    part
}

/// Names each part: a router or guardrail by its role when it's the only one, a part
/// of several nodes after the word their names share most, any other after its node.
fn name_parts(graph: &StateGraph, members: &[Vec<usize>], parts: &mut [Part]) {
    let mut roles: FxHashMap<Role, usize> = FxHashMap::default();
    for (part, members) in parts.iter().zip(members) {
        if members.len() == 1 {
            *roles.entry(part.role).or_default() += 1;
        }
    }
    for (part, members) in parts.iter_mut().zip(members) {
        let node = &graph.nodes[members[0]];
        part.name = match (members.len(), part.role.title()) {
            (1, Some(title)) if roles[&part.role] == 1 => title.to_string(),
            (1, _) => humanize(&node.name),
            _ => {
                let word = shared_word(graph, members).unwrap_or(&node.name);
                format!("{} agent", humanize(word))
            }
        };
        if part.role == Role::Agent && members.len() == 1 && !part.name.to_lowercase().contains("agent") {
            part.name.push_str(" agent");
        }
    }
}

/// Words that say how a node is built rather than what it's for.
const PLUMBING: &[&str] = &[
    "after", "agent", "agents", "and", "call", "handle", "in", "node", "of", "run", "state", "step", "the", "to",
    "tool", "tools", "update", "updates",
];

/// The word that appears most in the names of a part's nodes and of their functions.
/// Ties go to a word of the agent that runs the part's tools, its core, and then to the
/// earliest node.
fn shared_word<'g>(graph: &'g StateGraph, members: &[usize]) -> Option<&'g str> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for &node in members {
        let node = &graph.nodes[node];
        let function = match &node.kind {
            NodeKind::Function { name, .. } => last_segment(name),
            _ => "",
        };
        for word in node.name.split('_').chain(function.split('_')) {
            let word = word.trim();
            if word.len() < 3 || PLUMBING.contains(&word.to_ascii_lowercase().as_str()) {
                continue;
            }
            match counts.iter_mut().find(|(seen, _)| seen.eq_ignore_ascii_case(word)) {
                Some((_, count)) => *count += 1,
                None => counts.push((word, 1)),
            }
        }
    }
    let most = counts.iter().map(|(_, count)| *count).max()?;
    let mut tied = counts
        .into_iter()
        .filter(|(_, count)| *count == most)
        .map(|(word, _)| word);
    let first = tied.next()?;
    let core = members.iter().map(|&node| &graph.nodes[node]).find(|node| {
        node.steps
            .iter()
            .any(|step| matches!(step, Step::Model(call) if !call.tools.is_empty()))
    });
    let in_core =
        |word: &str| core.is_some_and(|core| core.name.split('_').any(|part| part.eq_ignore_ascii_case(word)));
    Some(
        std::iter::once(first)
            .chain(tied)
            .find(|word| in_core(word))
            .unwrap_or(first),
    )
}

/// `general_medical` → `General medical`.
fn humanize(name: &str) -> String {
    let words: Vec<String> = name
        .split('_')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let mut text = words.join(" ");
    if let Some(first) = text.get(..1) {
        let upper = first.to_ascii_uppercase();
        text.replace_range(..1, &upper);
    }
    text
}

/// The enum values and quoted upper-case constants a condition tests:
/// `intent == MessageIntent.NEW_CASE` → `NEW_CASE`.
fn tested_values(condition: &str) -> impl Iterator<Item = &str> {
    condition
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '"' || c == '\''))
        .filter_map(|token| {
            let quoted = token.len() > 2
                && (token.starts_with('"') && token.ends_with('"') || token.starts_with('\'') && token.ends_with('\''));
            let value = if quoted {
                &token[1..token.len() - 1]
            } else {
                let (owner, member) = token.rsplit_once('.')?;
                (!owner.is_empty()).then_some(member)?
            };
            is_constant(value).then_some(value)
        })
}

/// `NEW_CASE`, `EMERGENCY`: upper case, with at least two letters.
fn is_constant(value: &str) -> bool {
    value.chars().filter(char::is_ascii_uppercase).count() >= 2
        && value
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn push_new(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// Disjoint sets of nodes, with their sizes.
struct Sets {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl Sets {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
            size: vec![1; len],
        }
    }

    fn find(&self, mut node: usize) -> usize {
        while self.parent[node] != node {
            node = self.parent[node];
        }
        node
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        // The earlier node stays the root, so a part keeps its first node's place.
        let (root, child) = if a < b { (a, b) } else { (b, a) };
        self.parent[child] = root;
        self.size[root] += self.size[child];
    }

    fn size(&self, node: usize) -> usize {
        self.size[self.find(node)]
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// How many tested values an edge's label lists before it's cut short.
const MAX_LABEL_VALUES: usize = 4;
/// How many functions a part lists under `uses:` before it's cut short.
const MAX_USES: usize = 4;

/// The workflows as one Mermaid flowchart, or only the one of the graph at index `only`.
pub fn mermaid(results: &[FileResult], only: Option<usize>) -> String {
    let workflows = workflows(results, only);
    let boxed = workflows.len() > 1;
    let mut chart = Flowchart::new();
    for (role, style) in [
        ("router", "fill:#ece8ff,stroke:#6b5bd6"),
        ("supervisor", "fill:#ece8ff,stroke:#6b5bd6"),
        ("guardrail", "fill:#fde4ec,stroke:#c2416b"),
        ("agent", "fill:#e3f4ec,stroke:#2e9d6a"),
        ("responder", "fill:#fff1dc,stroke:#d08a1e"),
        ("tools", "fill:#e6f0fb,stroke:#2f6fdf"),
    ] {
        chart.class(role, style);
    }
    for (index, (result, workflow)) in workflows.iter().enumerate() {
        let prefix = if boxed {
            format!("g{}_", index + 1)
        } else {
            String::new()
        };
        if boxed {
            let graph = workflow.graph;
            let title = format!("{} · {}:{}", graph.name(), relative_path(result), graph.location.line);
            chart.begin_group(&format!("g{}", index + 1), &title);
        }
        draw(&mut chart, workflow, &prefix);
        if boxed {
            chart.end_group();
        }
    }
    chart.finish()
}

fn draw(chart: &mut Flowchart, workflow: &Workflow<'_>, prefix: &str) {
    let ids: Vec<String> = workflow.parts.iter().map(|part| id(prefix, &part.members[0])).collect();
    let used = |part: usize| {
        workflow
            .handoffs
            .iter()
            .any(|handoff| handoff.from == part || handoff.to == part)
    };
    for (index, part) in workflow.parts.iter().enumerate() {
        if matches!(part.role, Role::Start | Role::End) && !used(index) {
            continue;
        }
        let shape = match part.role {
            Role::Router | Role::Supervisor => Shape::Hexagon,
            Role::Guardrail => Shape::Trapezoid,
            Role::Tools => Shape::Subroutine,
            Role::Responder => Shape::Rect,
            Role::Subgraph => Shape::Stadium,
            Role::Start | Role::End | Role::Agent | Role::Step => Shape::Round,
        };
        let class = match part.role {
            Role::Start | Role::End | Role::Step | Role::Subgraph => None,
            role => Some(role.as_str()),
        };
        chart.shaped_node(&ids[index], &part.name, &details(part), shape, class);
    }
    for handoff in &workflow.handoffs {
        let label = if handoff.retry {
            Some("retry".to_string())
        } else if handoff.when.is_empty() {
            None
        } else {
            let mut label = handoff
                .when
                .iter()
                .take(MAX_LABEL_VALUES)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            if handoff.when.len() > MAX_LABEL_VALUES {
                label.push_str(", …");
            }
            Some(label)
        };
        chart.edge(
            &ids[handoff.from],
            &ids[handoff.to],
            label.as_deref(),
            handoff.conditional,
        );
    }
}

fn details(part: &Part) -> Vec<String> {
    if matches!(part.role, Role::Start | Role::End) {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if part.members.len() > 1 || part.name != part.members[0] {
        lines.push(part.members.join(" · "));
    }
    if !part.models.is_empty() {
        let mut line = format!("model: {}", part.models.join(", "));
        if part.tool_loop {
            line.push_str(" ⟳ tools");
        }
        lines.push(line);
    }
    if !part.tools.is_empty() {
        lines.push(format!("tools: {}", part.tools.join(", ")));
    }
    if !part.parallel.is_empty() {
        lines.push(format!("in parallel: {}", part.parallel.join(", ")));
    }
    if !part.uses.is_empty() {
        let uses: Vec<&str> = part.uses.iter().take(MAX_USES).map(|name| last_segment(name)).collect();
        let mut line = format!("uses: {}", uses.join(", "));
        if part.uses.len() > MAX_USES {
            line.push_str(&format!(" + {} more", part.uses.len() - MAX_USES));
        }
        lines.push(line);
    }
    lines
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// The workflows as JSON:
///
/// ```json
/// {"schema": 1, "tier": "workflow", "graphs": [{
///   "id": "graph:app.flow.build", "name": "build", "path": "app/flow.py", "line": 12,
///   "parts": [{"id": "part:...", "name": "Triage agent", "role": "agent",
///              "members": ["triage_decision", "tools"], "models": [...], ...}],
///   "handoffs": [{"source": "part:...", "target": "part:...", "conditional": true,
///                 "retry": false, "when": ["NEW_CASE"], "edges": ["router -> triage_decision"]}]}]}
/// ```
pub fn json(results: &[FileResult], only: Option<usize>) -> String {
    let mut out = Json::with_capacity(4096);
    out.0.push_str("{\"schema\":1,\"tier\":\"workflow\",");
    out.begin_array("graphs");
    for (result, workflow) in workflows(results, only) {
        write_workflow(&mut out, result, &workflow);
    }
    out.end_array();
    out.end_object();
    out.0
}

fn write_workflow(out: &mut Json, result: &FileResult, workflow: &Workflow<'_>) {
    let graph = workflow.graph;
    let path = relative_path(result);
    let graph_id = qualify(result, &path, graph.name());
    let part_id = |part: &Part| match part.role {
        Role::Start | Role::End => part.name.clone(),
        _ => format!("part:{graph_id}.{}", part.members[0]),
    };
    out.begin_object();
    out.string("id", &format!("graph:{graph_id}"));
    out.string("name", graph.name());
    out.string("path", &path);
    out.string("module", &result.module);
    out.number("line", graph.location.line);
    out.begin_array("parts");
    for part in &workflow.parts {
        out.begin_object();
        out.string("id", &part_id(part));
        out.string("name", &part.name);
        out.string("role", part.role.as_str());
        out.strings("members", part.members.iter().map(String::as_str));
        out.strings("models", part.models.iter().map(String::as_str));
        out.strings("tools", part.tools.iter().map(String::as_str));
        out.boolean("tool_loop", part.tool_loop);
        out.strings("uses", part.uses.iter().map(String::as_str));
        out.strings("parallel", part.parallel.iter().map(String::as_str));
        out.close();
    }
    out.end_array();
    out.begin_array("handoffs");
    for handoff in &workflow.handoffs {
        out.begin_object();
        out.string("source", &part_id(&workflow.parts[handoff.from]));
        out.string("target", &part_id(&workflow.parts[handoff.to]));
        out.boolean("conditional", handoff.conditional);
        out.boolean("retry", handoff.retry);
        out.strings("when", handoff.when.iter().map(String::as_str));
        out.begin_array("edges");
        for &edge in &handoff.edges {
            let edge = &graph.edges[edge];
            out.element(&format!("{} -> {}", edge.from, edge.to));
        }
        out.end_array();
        out.close();
    }
    out.end_array();
    out.close();
}

#[cfg(test)]
mod tests {
    use ruff_python_ast::PySourceType;
    use ruff_python_parser::parse_unchecked_source;

    use super::*;
    use crate::checker::check_module_with_references;
    use crate::discovery::LineIndex;
    use crate::flow;

    fn graph(source: &str) -> StateGraph {
        let parsed = parse_unchecked_source(source, PySourceType::Python);
        assert!(parsed.has_valid_syntax(), "{:?}", parsed.errors());
        let checker = check_module_with_references(parsed.syntax(), vec!["app"]);
        let mut graphs = flow::extract(&checker, source, &LineIndex::new(source));
        assert_eq!(graphs.len(), 1);
        graphs.remove(0)
    }

    /// `name (role): members`
    fn parts(workflow: &Workflow<'_>) -> Vec<String> {
        workflow
            .parts
            .iter()
            .map(|part| format!("{} ({}): {}", part.name, part.role.as_str(), part.members.join(", ")))
            .collect()
    }

    /// `from -> to [when]`, with `retry` for a retry.
    fn handoffs(workflow: &Workflow<'_>) -> Vec<String> {
        workflow
            .handoffs
            .iter()
            .map(|handoff| {
                let arrow = if handoff.retry { "-retry->" } else { "->" };
                let (from, to) = (&workflow.parts[handoff.from].name, &workflow.parts[handoff.to].name);
                format!("{from} {arrow} {to} {:?}", handoff.when)
            })
            .collect()
    }

    const TRIAGE: &str = "\
from langgraph.graph import StateGraph, START, END
from langgraph.prebuilt import ToolNode
from langchain_openai import ChatOpenAI
from app.rag import protocol
from app.checks import validate

llm = ChatOpenAI(model='gpt-4o')

def classify(state):
    return {'intent': llm.with_structured_output(Intent).invoke(state['messages'])}

def route(state):
    if state['intent'] == Intent.NEW_CASE:
        return 'identify'
    if state['intent'] in (Intent.BILLING, Intent.SCHEDULING):
        return 'template'
    return 'triage'

def identify(state):
    return {'protocol': llm.invoke(state['messages']), 'last': 'identify'}

def triage(state):
    doc = protocol(state['protocol'])
    return {'messages': llm.bind_tools([switch]).invoke(doc), 'last': 'triage'}

def apply_switch(state):
    return {'protocol': 'x'}

def template(state):
    return {'messages': ['See our billing page.']}

def guard(state):
    return {'feedback': validate(state['messages'])}

def check(state):
    if state['feedback'] and state['tries'] < 3:
        return state['last']
    if state['messages'][-1].tool_calls:
        return 'tools'
    return END

g = StateGraph(State)
g.add_node('classify', classify)
g.add_node('identify', identify)
g.add_node('triage', triage)
g.add_node('tools', ToolNode([switch]))
g.add_node('apply_switch', apply_switch)
g.add_node('template', template)
g.add_node('guard', guard)
g.add_edge(START, 'classify')
g.add_conditional_edges('classify', route, ['identify', 'template', 'triage'])
g.add_edge('identify', 'guard')
g.add_edge('triage', 'guard')
g.add_conditional_edges('guard', check, ['identify', 'triage', 'tools', END])
g.add_edge('tools', 'apply_switch')
g.add_edge('apply_switch', 'triage')
g.add_edge('template', END)
";

    #[test]
    fn groups_a_triage_graph_into_its_parts() {
        let graph = graph(TRIAGE);
        let workflow = build(&graph);
        assert_eq!(
            parts(&workflow),
            [
                "__start__ (start): __start__",
                "Router (router): classify",
                // `identify` writes `protocol`, which only the triage loop reads.
                "Triage agent (agent): identify, triage, tools, apply_switch",
                "Template (responder): template",
                "Guardrail (guardrail): guard",
                "__end__ (end): __end__",
            ]
        );
        assert_eq!(
            handoffs(&workflow),
            [
                "__start__ -> Router []",
                "Router -> Triage agent [\"NEW_CASE\"]",
                "Router -> Template [\"BILLING\", \"SCHEDULING\"]",
                "Triage agent -> Guardrail []",
                "Guardrail -retry-> Triage agent []",
                "Guardrail -> __end__ []",
                "Template -> __end__ []",
            ]
        );
        let triage = &workflow.parts[2];
        assert!(triage.tool_loop);
        assert_eq!(triage.tools, ["switch"]);
        assert_eq!(triage.models, ["gpt-4o"]);
        assert_eq!(triage.uses, ["app.rag.protocol"]);
    }

    #[test]
    fn a_hub_the_graph_starts_at_is_a_supervisor() {
        let graph = graph(
            "from langgraph.graph import StateGraph, START, END\n\
             def supervisor(state): return {}\n\
             def research(state): return {}\n\
             def write(state): return {}\n\
             def route(state):\n\
             \x20   return state['next']\n\
             g = StateGraph(State)\n\
             g.add_node('supervisor', supervisor)\n\
             g.add_node('research', research)\n\
             g.add_node('write', write)\n\
             g.add_edge(START, 'supervisor')\n\
             g.add_conditional_edges('supervisor', route, ['research', 'write', END])\n\
             g.add_edge('research', 'supervisor')\n\
             g.add_edge('write', 'supervisor')\n",
        );
        let workflow = build(&graph);
        assert_eq!(workflow.parts[1].role, Role::Supervisor);
        assert_eq!(workflow.parts[1].name, "Supervisor");
        // A supervisor sending work out isn't retrying it.
        assert!(workflow.handoffs.iter().all(|handoff| !handoff.retry));
    }

    #[test]
    fn agents_sharing_a_resource_few_agents_use_work_as_one() {
        let graph = graph(
            "from langgraph.graph import StateGraph, START\n\
             from langchain_openai import ChatOpenAI\n\
             from app.rag import cases\n\
             from app.util import now\n\
             llm = ChatOpenAI(model='gpt-4o')\n\
             def intake(state): return {'x': llm.invoke(now())}\n\
             def recall(state): return {'x': llm.invoke(cases(now()))}\n\
             def follow_up(state): return {'x': llm.invoke(cases(now()))}\n\
             def answer(state): return {'x': llm.invoke(now())}\n\
             g = StateGraph(State)\n\
             g.add_sequence([intake, recall, follow_up, answer])\n\
             g.add_edge(START, 'intake')\n",
        );
        let workflow = build(&graph);
        // `now` is called by every agent, so it groups none of them.
        let names: Vec<_> = workflow.parts.iter().map(|part| part.members.join("+")).collect();
        assert_eq!(names, ["__start__", "intake", "recall+follow_up", "answer", "__end__"]);
    }

    #[test]
    fn tested_values_are_enum_members_and_quoted_constants() {
        let values: Vec<_> = tested_values(
            "intent == MessageIntent.NEW_CASE and state['level'] == 'EMERGENCY' or x.lower == 'no' or Intent.A",
        )
        .collect();
        assert_eq!(values, ["NEW_CASE", "EMERGENCY"]);
    }

    #[test]
    fn names() {
        assert_eq!(humanize("general_medical"), "General medical");
        assert_eq!(humanize("Template"), "Template");
    }
}
