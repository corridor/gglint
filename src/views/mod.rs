//! Diagrams of the agent systems in a codebase, at three levels of detail. Each level
//! is built from the one below it:
//!
//! 1. **code** ([`code`]): every graph as LangGraph draws it, and inside each node the
//!    models, prompts, tools and functions it uses.
//! 2. **workflow** ([`workflow`]): the parts of each graph, such as a router, an agent
//!    with its tools or a guardrail, and how work moves between them. Each part is a group
//!    of nodes from the code tier.
//!
//! Every diagram is printed as Mermaid, in the style LangGraph's `draw_mermaid()` uses,
//! or as JSON for other tools to draw.

pub mod code;
mod mermaid;
pub mod workflow;

use crate::discovery::{FileResult, Location};

/// How much detail a diagram shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Graphs, nodes and edges, and what each node does in the code.
    Code,
    /// The parts of each graph and the handoffs between them.
    Workflow,
}

impl Tier {
    pub const ALL: [Tier; 2] = [Tier::Code, Tier::Workflow];

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Code => "code",
            Tier::Workflow => "workflow",
        }
    }

    pub fn parse(name: &str) -> Option<Tier> {
        Tier::ALL.into_iter().find(|tier| tier.as_str() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mermaid,
    Json,
}

/// The diagram of `results`, from [`crate::discovery::scan`], at `tier`.
pub fn render(results: &[FileResult], tier: Tier, format: Format) -> String {
    match (tier, format) {
        (Tier::Code, Format::Mermaid) => code::mermaid(results),
        (Tier::Code, Format::Json) => code::json(results),
        (Tier::Workflow, Format::Mermaid) => workflow::mermaid(results),
        (Tier::Workflow, Format::Json) => workflow::json(results),
    }
}

/// The file of a result, relative to the directory it was found under, with `/`
/// separators.
fn relative_path(result: &FileResult) -> String {
    crate::graph::slash_path(&result.relative_path)
}

/// The prompt written at `location` in `result`, if prompt discovery kept it.
fn prompt_at(result: &FileResult, location: Location) -> Option<&crate::discovery::DiscoveredPrompt> {
    result
        .prompts
        .iter()
        .find(|(at, _)| *at == location)
        .map(|(_, prompt)| prompt)
}

/// `a.b.c` → `c`.
fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// The id of a prompt, the same as the lineage graph's.
fn prompt_id(path: &str, location: Location) -> String {
    format!("prompt:{path}:{}:{}", location.line, location.column)
}

/// `module.name`, or `path:name` when the module has no name (a single file scanned).
fn qualify(result: &FileResult, path: &str, name: &str) -> String {
    if result.module.is_empty() {
        format!("{path}:{name}")
    } else {
        format!("{}.{name}", result.module)
    }
}
