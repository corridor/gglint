//! The lineage graph: which text reaches which agent.
//!
//! ```text
//! constant TONE ──feeds──▶ prompt A.system_prompt ──given_to──▶ agent A
//!        └──────feeds──▶ prompt B.system_prompt ──given_to──▶ agent B
//! file system.md ─feeds──▶ prompt C.instructions ──given_to──▶ agent B
//! ```
//!
//! Three kinds of node:
//! - **agents**, as discovered;
//! - **prompts**, as discovered: text passed to a sink, where it's passed;
//! - **sources**: the named definitions prompt text comes from: module-level constants,
//!   functions whose return value is used, files read at runtime and crewAI config
//!   fields. Text written inline at the sink is part of the prompt.
//!
//! A source used by several prompts is one node with several edges, which is where
//! the many-to-many comes from (sqlglot's lineage caches its nodes the same way). A
//! source built from other sources keeps them: `RULES = BASE + "..."` gives
//! `BASE ─▶ RULES ─▶ prompt`.
//!
//! Nothing here reads source code. During the scan, each prompt's text keeps markers
//! around the parts that come from a source ([`Part::Begin`]), and records the agent it
//! is given to ([`AgentLink`]). [`build`] consumes the scan results and walks those
//! once, so it costs time proportional to the number of prompt parts, not to the size
//! of the code. This is the structure of Ruff's `analyze graph`: facts collected per
//! file in parallel, linked in one pass afterwards. Agents imported from another file
//! are linked by the scan (`discovery::link_imported_agents`), by module and variable,
//! as stack-graphs links files through names rather than syntax trees.
//!
//! Nodes and edges are plain vectors indexed by `u32`, as in Ruff's semantic model. Ids
//! are strings derived from names, such as `constant:app.prompts.TONE`, so they stay the
//! same between scans and a viewer can match nodes across them. The JSON is the
//! node-link format networkx and most graph viewers read.

use std::ffi::OsStr;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::discovery::{AgentLink, DiscoveredAgent, DiscoveredPrompt, FileResult, Location};
use crate::project::{Part, SourceKind, SourceRef};
use crate::prompts::Link;

/// The version of the JSON format, in `graph.schema`.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug)]
pub struct Node {
    /// Unique in the graph, and the same between scans of the same code.
    pub id: String,
    /// The file, relative to the directory it was found under, with `/` separators.
    /// Shared by the nodes of a file.
    pub path: Arc<str>,
    /// The module's dotted path. Empty for files that aren't Python.
    pub module: Arc<str>,
    pub location: Option<Location>,
    pub data: NodeData,
}

#[derive(Debug)]
pub enum NodeData {
    Agent(DiscoveredAgent),
    Prompt(DiscoveredPrompt),
    Source(Source),
}

#[derive(Debug)]
pub struct Source {
    pub kind: SourceKind,
    /// `app.prompts.TONE` for a constant or function, the entry and field for a config
    /// (`writer.role`), the path for a file.
    pub name: String,
    /// The source's text. Parts only known at runtime are written as `{expression}`.
    pub text: String,
    pub dynamic_parts: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    /// Indices into [`Graph::nodes`].
    pub from: u32,
    pub to: u32,
    pub kind: EdgeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// A source's text is part of a prompt's or another source's. `position` is the
    /// order the target uses its sources in, from 0.
    Feeds { position: u32 },
    /// A prompt is given to an agent.
    GivenTo(Link),
}

impl Node {
    pub fn kind(&self) -> &'static str {
        match self.data {
            NodeData::Agent(_) => "agent",
            NodeData::Prompt(_) => "prompt",
            NodeData::Source(_) => "source",
        }
    }

    /// A short name to show: the agent's name, the argument a prompt is passed as, or
    /// the source's own name.
    pub fn label(&self) -> &str {
        match &self.data {
            NodeData::Agent(agent) => agent.display_name().unwrap_or_else(|| last_segment(&agent.via)),
            NodeData::Prompt(prompt) => prompt.argument,
            NodeData::Source(source) => match source.kind {
                SourceKind::Constant | SourceKind::Function => last_segment(&source.name),
                SourceKind::File => source.name.rsplit('/').next().unwrap_or(&source.name),
                SourceKind::Config => &source.name,
            },
        }
    }
}

fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// Builds the lineage graph from the results of [`crate::discovery::scan`].
pub fn build(results: Vec<FileResult>) -> Graph {
    let mut graph = Graph::default();
    // The node of each file's first agent, by the file's index in the results.
    let mut first_agents = Vec::with_capacity(results.len());
    let mut prompts = Vec::new();
    let mut leftovers = Vec::with_capacity(results.len());

    // Agents first, so a prompt can be linked to an agent in any file.
    for mut result in results {
        first_agents.push(graph.nodes.len() as u32);
        // Most files of a large tree have neither: the prefilter skipped them.
        if result.agents.is_empty() && result.prompts.is_empty() {
            leftovers.push(result);
            continue;
        }
        let file = Rc::new(FileInfo {
            path: slash_path(&result.relative_path).into(),
            module: std::mem::take(&mut result.module).into(),
            root: root_of(&result).to_path_buf(),
        });
        // Two agents can only have the same id if they're in the same file.
        let mut ids: FxHashMap<String, u32> = FxHashMap::default();
        for (location, agent) in std::mem::take(&mut result.agents) {
            // The names other code reaches the agent by come first: they're the most
            // stable.
            let qualifier = match agent.variable.as_ref().or(agent.factory.as_ref()).or(agent.name.as_ref()) {
                Some(name) => name.clone(),
                None => format!("{}:{}", location.line, location.column),
            };
            let id = unique(&mut ids, format!("agent:{}", qualify(&file.module, &file.path, &qualifier)));
            push_node(&mut graph, id, file.path.clone(), file.module.clone(), Some(location), NodeData::Agent(agent));
        }
        let first_agent = *first_agents.last().expect("pushed above");
        for (location, prompt) in std::mem::take(&mut result.prompts) {
            prompts.push((file.clone(), first_agent, location, prompt));
        }
        leftovers.push(result);
    }
    // What's left of the results is only freed memory, which takes a while for a large
    // tree, and nothing waits on it.
    rayon::spawn(move || drop(leftovers));

    // The parts are set aside while the graph is built, so the sources found in them
    // can be looked up by reference, without copying their names.
    let parts: Vec<Vec<Part>> = prompts
        .iter_mut()
        .map(|(_, _, _, prompt)| std::mem::take(&mut prompt.parts))
        .collect();
    let mut builder = Builder::default();
    let mut prompt_nodes = Vec::with_capacity(prompts.len());
    let mut previous: Option<(*const FileInfo, Location, u32)> = None;
    for ((file, first_agent, location, prompt), parts) in prompts.into_iter().zip(&parts) {
        // Prompts are in source order, so two at the same place, the fields of one
        // crewAI config, are next to each other.
        let repeat = match previous {
            Some((previous_file, previous_location, count))
                if std::ptr::eq(previous_file, Rc::as_ptr(&file)) && previous_location == location =>
            {
                count + 1
            }
            _ => 1,
        };
        previous = Some((Rc::as_ptr(&file), location, repeat));
        let mut id = format!("prompt:{}:{}:{}", file.path, location.line, location.column);
        if repeat > 1 {
            let _ = write!(id, "#{repeat}");
        }
        let target = match &prompt.link {
            Some((AgentLink::Local(index), link)) => Some((first_agent + *index as u32, *link)),
            Some((AgentLink::Elsewhere { file, agent }, link)) => Some((first_agents[*file] + *agent as u32, *link)),
            Some((AgentLink::Import { .. }, _)) | None => None,
        };
        let node = push_node(&mut graph, id, file.path.clone(), file.module.clone(), Some(location), NodeData::Prompt(prompt));
        prompt_nodes.push(node);
        if let Some((agent, link)) = target {
            builder.add_edge(&mut graph, node, agent, EdgeKind::GivenTo(link));
        }
        builder.add_sources(&mut graph, node, parts, &file.root);
    }
    drop(builder);

    // Hand the parts back: they're part of each prompt.
    for (node, parts) in prompt_nodes.into_iter().zip(parts) {
        if let NodeData::Prompt(prompt) = &mut graph.nodes[node as usize].data {
            prompt.parts = parts;
        }
    }
    graph
}

/// What the nodes of one file share.
struct FileInfo {
    path: Arc<str>,
    module: Arc<str>,
    /// The directory the file was found under, which source paths are shown relative to.
    root: PathBuf,
}

fn push_node(graph: &mut Graph, id: String, path: Arc<str>, module: Arc<str>, location: Option<Location>, data: NodeData) -> u32 {
    let index = graph.nodes.len() as u32;
    graph.nodes.push(Node {
        id,
        path,
        module,
        location,
        data,
    });
    index
}

/// `id`, or `id#2`, `id#3`, ... if it's already in `ids`.
fn unique(ids: &mut FxHashMap<String, u32>, id: String) -> String {
    match ids.get_mut(&id) {
        Some(count) => {
            *count += 1;
            format!("{id}#{count}")
        }
        None => {
            ids.insert(id.clone(), 1);
            id
        }
    }
}

/// Links prompts to their sources. `'p` is the lifetime of the prompts' parts, which
/// the source index borrows its keys from.
#[derive(Default)]
struct Builder<'p> {
    /// Paths are compared as bytes: hashing a `Path` parses it into components.
    sources: FxHashMap<(SourceKind, &'p str, &'p OsStr), u32>,
    edges: FxHashSet<(u32, u32)>,
    /// How many sources each node has so far, which is the position of the next one, by
    /// node.
    positions: Vec<u32>,
}

impl<'p> Builder<'p> {
    fn add_edge(&mut self, graph: &mut Graph, from: u32, to: u32, kind: EdgeKind) -> bool {
        if !self.edges.insert((from, to)) {
            return false;
        }
        graph.edges.push(Edge { from, to, kind });
        true
    }

    /// Adds the sources in `parts`, the parts of the prompt `prompt`'s text, and the
    /// edges between them.
    fn add_sources(&mut self, graph: &mut Graph, prompt: u32, parts: &'p [Part], root: &Path) {
        if !parts.iter().any(|part| matches!(part, Part::Begin(_))) {
            return;
        }
        // The node each open source feeds, and the text of the sources seen for the
        // first time, which is collected as the parts go by.
        let mut stack: Vec<Frame> = vec![Frame {
            node: prompt,
            text: None,
        }];
        for part in parts {
            match part {
                Part::Begin(source) => {
                    let parent = stack.last().map_or(prompt, |frame| frame.node);
                    let (node, new) = self.source_node(graph, source, root);
                    if self.positions.len() < graph.nodes.len() {
                        self.positions.resize(graph.nodes.len(), 0);
                    }
                    let position = self.positions[parent as usize];
                    if self.add_edge(graph, node, parent, EdgeKind::Feeds { position }) {
                        self.positions[parent as usize] += 1;
                    }
                    stack.push(Frame {
                        node,
                        text: new.then(|| (String::new(), 0)),
                    });
                }
                Part::End => {
                    if stack.len() > 1 {
                        finish(graph, stack.pop().expect("checked"));
                    }
                }
                Part::Text(text) => {
                    for (captured, _) in stack.iter_mut().filter_map(|frame| frame.text.as_mut()) {
                        captured.push_str(text);
                    }
                }
                Part::Dynamic(source)
                | Part::Import { source, .. }
                | Part::File { source, .. }
                | Part::Config { source, .. } => {
                    for (captured, dynamic_parts) in stack.iter_mut().filter_map(|frame| frame.text.as_mut()) {
                        captured.push('{');
                        captured.push_str(source);
                        captured.push('}');
                        *dynamic_parts += 1;
                    }
                }
            }
        }
        // Markers cut off by the limit on a prompt's parts.
        while stack.len() > 1 {
            finish(graph, stack.pop().expect("checked"));
        }
    }

    /// The node for `source`, and whether it was just added.
    fn source_node(&mut self, graph: &mut Graph, source: &'p SourceRef, root: &Path) -> (u32, bool) {
        let key = (source.kind, source.name.as_str(), source.path.as_os_str());
        if let Some(&node) = self.sources.get(&key) {
            return (node, false);
        }
        let path = slash_path(source.path.strip_prefix(root).unwrap_or(&source.path));
        let (id, name, module) = match source.kind {
            SourceKind::Constant | SourceKind::Function => {
                let module = source.name.rsplit_once('.').map_or("", |(module, _)| module);
                (format!("{}:{}", source.kind.as_str(), source.name), source.name.clone(), module.into())
            }
            SourceKind::File => (format!("file:{path}"), path.clone(), Arc::from("")),
            SourceKind::Config => (format!("config:{path}#{}", source.name), source.name.clone(), Arc::from("")),
        };
        let node = push_node(
            graph,
            id,
            path.into(),
            module,
            source.location,
            NodeData::Source(Source {
                kind: source.kind,
                name,
                text: String::new(),
                dynamic_parts: 0,
            }),
        );
        self.sources.insert(key, node);
        (node, true)
    }
}

fn finish(graph: &mut Graph, frame: Frame) {
    if let Some((text, dynamic_parts)) = frame.text {
        if let NodeData::Source(source) = &mut graph.nodes[frame.node as usize].data {
            source.text = text;
            source.dynamic_parts = dynamic_parts;
        }
    }
}

struct Frame {
    node: u32,
    /// The source's text and its number of dynamic parts, collected the first time the
    /// source is seen.
    text: Option<(String, usize)>,
}

/// `module.name`, or `path:name` when the module has no name (a single file scanned).
fn qualify(module: &str, path: &str, name: &str) -> String {
    if module.is_empty() { format!("{path}:{name}") } else { format!("{module}.{name}") }
}

/// The directory the file was found under: `path` without `relative_path`.
fn root_of(result: &FileResult) -> &Path {
    let path = result.path.as_os_str().as_encoded_bytes();
    let relative = result.relative_path.as_os_str().as_encoded_bytes();
    if relative.is_empty() || !path.ends_with(relative) {
        return result.path.ancestors().nth(result.relative_path.components().count()).unwrap_or(Path::new(""));
    }
    // SAFETY: the bytes come from an `OsStr` and are cut just before a separator or
    // at the end, where an encoded `OsStr` can be split.
    let root = unsafe { OsStr::from_encoded_bytes_unchecked(&path[..path.len() - relative.len()]) };
    Path::new(root)
}

pub(crate) fn slash_path(path: &Path) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        return path.to_string_lossy().into_owned();
    }
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

impl Graph {
    /// The graph in networkx's node-link format, which most graph tools read:
    ///
    /// ```json
    /// {"directed": true, "multigraph": false, "graph": {"schema": 1},
    ///  "nodes": [{"id": "agent:app.bot.agent", "kind": "agent", ...}],
    ///  "edges": [{"source": "prompt:app/bot.py:7:19", "target": "agent:app.bot.agent",
    ///             "kind": "given_to", "link": "constructor"}]}
    /// ```
    ///
    /// Written by hand rather than through serde: the format is small and fixed, and
    /// the crate has no other use for serde.
    pub fn to_json(&self) -> String {
        let text: usize = self
            .nodes
            .iter()
            .map(|node| match &node.data {
                NodeData::Prompt(prompt) => prompt.text.len(),
                NodeData::Source(source) => source.text.len(),
                NodeData::Agent(_) => 0,
            })
            .sum();
        let mut out = Json(String::with_capacity(text + 256 * self.nodes.len() + 128 * self.edges.len()));
        let _ = write!(
            out.0,
            "{{\"directed\":true,\"multigraph\":false,\"graph\":{{\"schema\":{SCHEMA_VERSION}}},\"nodes\":["
        );
        for (index, node) in self.nodes.iter().enumerate() {
            if index > 0 {
                out.0.push(',');
            }
            out.node(node);
        }
        out.0.push_str("],\"edges\":[");
        for (index, edge) in self.edges.iter().enumerate() {
            if index > 0 {
                out.0.push(',');
            }
            out.0.push('{');
            out.string("source", &self.nodes[edge.from as usize].id);
            out.string("target", &self.nodes[edge.to as usize].id);
            match edge.kind {
                EdgeKind::Feeds { position } => {
                    out.string("kind", "feeds");
                    out.number("position", position as usize);
                }
                EdgeKind::GivenTo(link) => {
                    out.string("kind", "given_to");
                    out.string("link", link.as_str());
                }
            }
            out.end_object();
        }
        out.0.push_str("]}");
        out.0
    }
}

struct Json(String);

impl Json {
    fn node(&mut self, node: &Node) {
        self.0.push('{');
        self.string("id", &node.id);
        self.string("kind", node.kind());
        self.string("label", node.label());
        self.string("path", &node.path);
        self.string("module", &node.module);
        match node.location {
            Some(location) => {
                self.number("line", location.line);
                self.number("column", location.column);
            }
            None => {
                self.null("line");
                self.null("column");
            }
        }
        match &node.data {
            NodeData::Agent(agent) => {
                self.string("framework", agent.framework);
                self.string("agent_kind", agent.kind.as_str());
                self.string("via", &agent.via);
                self.optional("name", agent.name.as_deref());
                self.optional("model", agent.model.as_deref());
                self.optional("variable", agent.variable.as_deref());
                self.optional("factory", agent.factory.as_deref());
            }
            NodeData::Prompt(prompt) => {
                self.string("framework", prompt.framework);
                self.string("role", prompt.role.as_str());
                self.optional("via", prompt.via.as_deref());
                self.string("argument", prompt.argument);
                self.string("text", &prompt.text);
                self.number("dynamic_parts", prompt.dynamic_parts);
            }
            NodeData::Source(source) => {
                self.string("source_kind", source.kind.as_str());
                self.string("name", &source.name);
                self.string("text", &source.text);
                self.number("dynamic_parts", source.dynamic_parts);
            }
        }
        self.end_object();
    }

    fn key(&mut self, key: &str) {
        // Keys are written here, never taken from the code, so they need no escaping.
        self.0.push('"');
        self.0.push_str(key);
        self.0.push_str("\":");
    }

    fn string(&mut self, key: &str, value: &str) {
        self.key(key);
        self.0.push('"');
        let bytes = value.as_bytes();
        // Almost all text needs no escaping, and this check compiles to a fast loop.
        if !bytes.iter().any(|&byte| byte < 0x20 || byte == b'"' || byte == b'\\') {
            self.0.push_str(value);
            self.0.push_str("\",");
            return;
        }
        let mut start = 0;
        for (index, &byte) in bytes.iter().enumerate() {
            let escape = match byte {
                b'"' => "\\\"",
                b'\\' => "\\\\",
                b'\n' => "\\n",
                b'\r' => "\\r",
                b'\t' => "\\t",
                0..=0x1f => "",
                _ => continue,
            };
            self.0.push_str(&value[start..index]);
            if escape.is_empty() {
                let _ = write!(self.0, "\\u{byte:04x}");
            } else {
                self.0.push_str(escape);
            }
            start = index + 1;
        }
        self.0.push_str(&value[start..]);
        self.0.push_str("\",");
    }

    fn optional(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(value) => self.string(key, value),
            None => self.null(key),
        }
    }

    fn number(&mut self, key: &str, value: usize) {
        self.key(key);
        let _ = write!(self.0, "{value},");
    }

    fn null(&mut self, key: &str) {
        self.key(key);
        self.0.push_str("null,");
    }

    /// Replaces the trailing comma every value is written with.
    fn end_object(&mut self) {
        if self.0.ends_with(',') {
            self.0.pop();
        }
        self.0.push('}');
    }
}
