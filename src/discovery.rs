//! Agent discovery: matches what the checker found against a registry of agent
//! constructors, and scans directories of Python files in parallel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use ruff_python_ast::{Expr, ExprCall, PySourceType};
use ruff_python_parser::parse_unchecked_source;
use ruff_text_size::{Ranged, TextSize};

use crate::checker::{CandidateKind, Checker, check_module};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignatureKind {
    /// Calling it creates an agent, and subclassing it defines an agent class.
    Constructor,
    /// Decorating a function with it marks the function as an agent factory.
    Decorator,
}

struct AgentSignature {
    framework: &'static str,
    qualified_name: &'static str,
    kind: SignatureKind,
}

const fn constructor(framework: &'static str, qualified_name: &'static str) -> AgentSignature {
    AgentSignature {
        framework,
        qualified_name,
        kind: SignatureKind::Constructor,
    }
}

const fn decorator(framework: &'static str, qualified_name: &'static str) -> AgentSignature {
    AgentSignature {
        framework,
        qualified_name,
        kind: SignatureKind::Decorator,
    }
}

/// Public import paths of agent constructors. Frameworks re-export classes from
/// several modules, so each path a user might import from is listed.
///
/// This belongs in a data file eventually, so adding a framework doesn't need a rebuild.
/// Check the paths against current framework releases before relying on them.
const REGISTRY: &[AgentSignature] = &[
    constructor("pydantic-ai", "pydantic_ai.Agent"),
    constructor("pydantic-ai", "pydantic_ai.agent.Agent"),
    constructor("openai-agents", "agents.Agent"),
    constructor("openai-agents", "agents.agent.Agent"),
    constructor("crewai", "crewai.Agent"),
    constructor("crewai", "crewai.agent.Agent"),
    constructor("crewai", "crewai.Crew"),
    constructor("crewai", "crewai.crew.Crew"),
    decorator("crewai", "crewai.project.agent"),
    decorator("crewai", "crewai.project.crew"),
    constructor("google-adk", "google.adk.agents.Agent"),
    constructor("google-adk", "google.adk.agents.LlmAgent"),
    constructor("google-adk", "google.adk.agents.llm_agent.LlmAgent"),
    constructor("google-adk", "google.adk.agents.SequentialAgent"),
    constructor("google-adk", "google.adk.agents.ParallelAgent"),
    constructor("google-adk", "google.adk.agents.LoopAgent"),
    constructor("langgraph", "langgraph.prebuilt.create_react_agent"),
    constructor("langchain", "langchain.agents.create_agent"),
    constructor("deepagents", "deepagents.create_deep_agent"),
    constructor("autogen", "autogen_agentchat.agents.AssistantAgent"),
];

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    /// `Agent(...)`, or a call to a class that subclasses an agent.
    Instance,
    /// `class Reviewer(Agent): ...`
    Class,
    /// `@agent def researcher(self): ...`
    Factory,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Instance => "agent",
            AgentKind::Class => "agent_class",
            AgentKind::Factory => "agent_factory",
        }
    }
}

#[derive(Debug)]
pub struct DiscoveredAgent {
    pub framework: &'static str,
    pub kind: AgentKind,
    /// What was called, subclassed or used as a decorator.
    pub via: String,
    /// The agent's own name: the class or function name, or the assigned variable.
    pub name: Option<String>,
    /// The model, if it's written as a literal.
    pub model: Option<String>,
    /// The module-level variable the agent is assigned to, which is the name other
    /// code imports it by: `agent` in `agent = create_agent(...)`.
    pub variable: Option<String>,
    pub offset: TextSize,
}

/// Matches a module's candidates against the registry.
pub fn discover(checker: &Checker<'_>) -> Vec<DiscoveredAgent> {
    // Qualified names that create agents when called, mapped to their framework. Starts
    // with the registry, then grows with this module's own subclasses.
    let mut agent_classes: HashMap<String, &'static str> = REGISTRY
        .iter()
        .filter(|signature| signature.kind == SignatureKind::Constructor)
        .map(|signature| (signature.qualified_name.to_string(), signature.framework))
        .collect();

    let mut agents = Vec::new();

    // Find agent subclasses, including subclasses of subclasses. Each base was resolved
    // where the class is written, so a base must be defined before the class that uses
    // it, just as Python requires.
    for candidate in &checker.candidates {
        let CandidateKind::Base(class) = candidate.kind else {
            continue;
        };
        let Some(base) = &candidate.qualified_name else {
            continue;
        };
        let Some(&framework) = agent_classes.get(&base.to_string()) else {
            continue;
        };
        let class_name: Vec<&str> = checker
            .semantic
            .module_path()
            .iter()
            .copied()
            .chain(std::iter::once(class.name.as_str()))
            .collect();
        let class_name = class_name.join(".");
        // Only module-level classes can be called by name from elsewhere.
        if candidate.scope.is_global() {
            agent_classes.insert(class_name, framework);
        }
        agents.push(DiscoveredAgent {
            framework,
            kind: AgentKind::Class,
            via: base.to_string(),
            name: Some(class.name.to_string()),
            model: None,
            variable: None,
            offset: class.name.range.start(),
        });
    }

    for candidate in &checker.candidates {
        let Some(qualified_name) = &candidate.qualified_name else {
            continue;
        };
        let qualified_name = qualified_name.to_string();

        match &candidate.kind {
            CandidateKind::Call(call) => {
                if let Some(&framework) = agent_classes.get(&qualified_name) {
                    agents.push(DiscoveredAgent {
                        framework,
                        kind: AgentKind::Instance,
                        via: qualified_name,
                        name: literal_keyword(call, &["name", "role"]),
                        model: literal_keyword(call, &["model", "llm"])
                            .or_else(|| first_literal_argument(call)),
                        variable: candidate.assigned_to.map(str::to_string),
                        offset: candidate.start(),
                    });
                }
            }
            CandidateKind::Decorator { function, .. } => {
                let signature = REGISTRY.iter().find(|signature| {
                    signature.kind == SignatureKind::Decorator
                        && signature.qualified_name == qualified_name
                });
                if let (Some(signature), Some(function)) = (signature, function) {
                    agents.push(DiscoveredAgent {
                        framework: signature.framework,
                        kind: AgentKind::Factory,
                        via: qualified_name,
                        name: Some(function.name.to_string()),
                        model: None,
                        variable: None,
                        offset: function.name.range.start(),
                    });
                }
            }
            CandidateKind::Base(_) => {}
        }
    }

    agents.sort_by_key(|agent| agent.offset);
    agents
}

/// The value of the first matching keyword argument, if it's a string literal.
fn literal_keyword(call: &ExprCall, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        let keyword = call.arguments.find_keyword(name)?;
        string_value(&keyword.value)
    })
}

/// The first positional argument, if it's a string literal: `Agent("openai:gpt-4o")`.
fn first_literal_argument(call: &ExprCall) -> Option<String> {
    call.arguments.args.first().and_then(string_value)
}

fn string_value(expr: &Expr) -> Option<String> {
    match expr {
        Expr::StringLiteral(literal) => Some(literal.value.to_str().to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Finds and checks every Python file under `roots`, on every core at once, as Ruff
/// does. Results are ordered by path.
///
/// With `parse_all`, files that can't contain an agent are parsed anyway, which is
/// only useful for measuring the parser.
pub fn scan(roots: &[PathBuf], parse_all: bool) -> Vec<FileResult> {
    let mut files = Vec::new();
    for root in roots {
        let mut paths = Vec::new();
        collect_python_files(root, &mut paths);
        files.extend(paths.into_iter().map(|path| (root.as_path(), path)));
    }
    files.sort_by(|(_, a), (_, b)| a.cmp(b));

    let prefilter = Prefilter::new();
    files
        .par_iter()
        .filter_map(|(root, path)| check_file(root, path, &prefilter, parse_all))
        .collect()
}

/// Skips files that can't contain an agent before spending time parsing them.
///
/// A file can only construct a framework's agent if it imports that framework, so its
/// text must contain the framework's top-level module name. One fast byte search per
/// name (`memchr` uses SIMD) is far cheaper than lexing and parsing. False positives,
/// such as the word "agents" in a comment, only cost a parse.
///
/// This holds while discovery is per-file. Once agents can be found through imports
/// of the project's own modules, those module names must be searched for too.
struct Prefilter {
    finders: Vec<memchr::memmem::Finder<'static>>,
}

impl Prefilter {
    fn new() -> Self {
        let mut roots: Vec<&'static str> = REGISTRY
            .iter()
            .filter_map(|signature| signature.qualified_name.split('.').next())
            .collect();
        roots.sort_unstable();
        roots.dedup();
        Self {
            finders: roots.into_iter().map(memchr::memmem::Finder::new).collect(),
        }
    }

    fn may_contain_agents(&self, source: &str) -> bool {
        self.finders
            .iter()
            .any(|finder| finder.find(source.as_bytes()).is_some())
    }
}

/// A 1-based line and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

pub struct FileResult {
    pub path: PathBuf,
    /// `path` relative to the root it was found under.
    pub relative_path: PathBuf,
    /// The module's dotted path, such as `myapp.agents`.
    pub module: String,
    pub bytes: usize,
    pub parsed: bool,
    pub warning: Option<String>,
    pub agents: Vec<(Location, DiscoveredAgent)>,
}

/// Reads, parses and checks one file.
fn check_file(root: &Path, path: &Path, prefilter: &Prefilter, parse_all: bool) -> Option<FileResult> {
    let Ok(source) = std::fs::read_to_string(path) else {
        eprintln!("warning: couldn't read {}", path.display());
        return None;
    };

    let module_path = module_path(root, path);
    let mut result = FileResult {
        path: path.to_path_buf(),
        relative_path: path.strip_prefix(root).unwrap_or(path).to_path_buf(),
        module: module_path.join("."),
        bytes: source.len(),
        parsed: false,
        warning: None,
        agents: Vec::new(),
    };
    if !parse_all && !prefilter.may_contain_agents(&source) {
        return Some(result);
    }

    // Ruff's parser recovers from syntax errors, so a file with a mistake in one place
    // still yields a tree for the rest of it.
    let parsed = parse_unchecked_source(&source, PySourceType::Python);
    result.parsed = true;
    if let Some(error) = parsed.errors().first() {
        result.warning = Some(format!(
            "{} syntax error(s), first at {}: {}",
            parsed.errors().len(),
            line_column(&source, error.location.start()),
            error.error
        ));
    }

    let module_path: Vec<&str> = module_path.iter().map(String::as_str).collect();
    let checker = check_module(parsed.syntax(), module_path);
    result.agents = discover(&checker)
        .into_iter()
        .map(|agent| (line_column(&source, agent.offset), agent))
        .collect();
    Some(result)
}

/// Recursively finds `.py` files, skipping hidden, virtualenv and build directories.
fn collect_python_files(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|extension| extension == "py") {
            files.push(path.to_path_buf());
        }
        return;
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir()
            && (name.starts_with('.')
                || matches!(
                    name.as_ref(),
                    "venv" | "node_modules" | "__pycache__" | "target" | "site-packages"
                ))
        {
            continue;
        }
        collect_python_files(&path, files);
    }
}

/// `root/myapp/agents.py` → `["myapp", "agents"]`; `root/myapp/__init__.py` → `["myapp"]`.
fn module_path(root: &Path, path: &Path) -> Vec<String> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let mut segments: Vec<String> = relative
        .with_extension("")
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if segments.last().is_some_and(|last| last == "__init__") {
        segments.pop();
    }
    segments
}

/// Converts a byte offset to a 1-based line and column.
fn line_column(source: &str, offset: TextSize) -> Location {
    let before = &source[..usize::from(offset)];
    let line = before.matches('\n').count() + 1;
    let column = before.len() - before.rfind('\n').map_or(0, |index| index + 1) + 1;
    Location { line, column }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents(source: &str) -> Vec<(AgentKind, &'static str, String)> {
        let parsed = parse_unchecked_source(source, PySourceType::Python);
        assert!(parsed.has_valid_syntax(), "{:?}", parsed.errors());
        let checker = check_module(parsed.syntax(), vec!["app"]);
        discover(&checker)
            .into_iter()
            .map(|agent| (agent.kind, agent.framework, agent.via))
            .collect()
    }

    #[test]
    fn discovers_across_frameworks() {
        let found = agents(
            r#"
from pydantic_ai import Agent as A
import crewai
from agents import Agent

"""Example: Agent(...) in a docstring is not an agent."""
# Neither is Agent(...) in a comment.

reviewer = A("openai:gpt-4o", system_prompt="You review PRs")
writer = crewai.Agent(role="writer", goal="...")
triage = Agent(name="triage", instructions="...")
"#,
        );
        assert_eq!(
            found,
            vec![
                (AgentKind::Instance, "pydantic-ai", "pydantic_ai.Agent".to_string()),
                (AgentKind::Instance, "crewai", "crewai.Agent".to_string()),
                (AgentKind::Instance, "openai-agents", "agents.Agent".to_string()),
            ]
        );
    }

    #[test]
    fn ignores_unrelated_classes_named_agent() {
        assert!(agents("class Agent:\n    pass\n\nbot = Agent()\n").is_empty());
    }

    #[test]
    fn discovers_subclasses_and_their_instances() {
        let found = agents(
            "from pydantic_ai import Agent\n\
             class Reviewer(Agent):\n    pass\n\n\
             class Senior(Reviewer):\n    pass\n\n\
             bot = Senior()\n",
        );
        assert_eq!(
            found,
            vec![
                (AgentKind::Class, "pydantic-ai", "pydantic_ai.Agent".to_string()),
                (AgentKind::Class, "pydantic-ai", "app.Reviewer".to_string()),
                (AgentKind::Instance, "pydantic-ai", "app.Senior".to_string()),
            ]
        );
    }

    #[test]
    fn discovers_crewai_decorators() {
        let found = agents(
            "from crewai.project import CrewBase, agent\n\n\
             @CrewBase\nclass Team:\n    @agent\n    def researcher(self):\n        return build()\n",
        );
        assert_eq!(
            found,
            vec![(AgentKind::Factory, "crewai", "crewai.project.agent".to_string())]
        );
    }

    #[test]
    fn module_level_assignments_name_their_agents() {
        let parsed = parse_unchecked_source(
            "from langchain.agents import create_agent\n\
             agent: object = create_agent(model='x')\n\
             if True:\n    other = create_agent(model='y')\n\
             wrapped = wrap(create_agent(model='z'))\n\
             def build():\n    inner = create_agent(model='w')\n",
            PySourceType::Python,
        );
        let checker = check_module(parsed.syntax(), vec!["app"]);
        let variables: Vec<_> = discover(&checker).into_iter().map(|agent| agent.variable).collect();
        assert_eq!(
            variables,
            vec![Some("agent".to_string()), Some("other".to_string()), None, None]
        );
    }

    #[test]
    fn module_paths() {
        let root = Path::new("/repo");
        assert_eq!(module_path(root, Path::new("/repo/app/bots.py")), vec!["app", "bots"]);
        assert_eq!(module_path(root, Path::new("/repo/app/__init__.py")), vec!["app"]);
    }
}
