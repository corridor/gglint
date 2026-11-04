//! The `gglint` Python package, built with PyO3 and maturin.
//!
//! ```python
//! import gglint
//!
//! for agent in gglint.find_agents("path/to/repo"):
//!     print(agent.path, agent.line, agent.framework, agent.via)
//! ```
//!
//! The scan runs in Rust, on every core, with Python's GIL released. Python only
//! receives the finished list.

use std::path::{Path, PathBuf};

use pyo3::exceptions::{PyFileNotFoundError, PyValueError};
use pyo3::prelude::*;

use crate::ask;
use crate::discovery::{FileResult, scan};
use crate::graph;
use crate::views::{self, Format, Tier};

/// An agent found in the source code.
#[pyclass(frozen, get_all, module = "gglint")]
struct Agent {
    /// The file, relative to the scanned directory, with `/` separators.
    path: String,
    /// The module's dotted path, such as `myapp.agents`, as if the scanned directory
    /// were on `sys.path`.
    module: String,
    /// 1-based line and column.
    line: usize,
    column: usize,
    /// The framework, such as `langchain`, `deepagents` or `crewai`.
    framework: &'static str,
    /// `"agent"`, `"agent_class"` or `"agent_factory"`.
    kind: &'static str,
    /// The qualified name that was called, subclassed or used as a decorator, such as
    /// `langchain.agents.create_agent`.
    via: String,
    /// The agent's name, if it's written as a literal (the `name=` or `role=`
    /// argument, or the class or function name).
    name: Option<String>,
    /// The model, if it's written as a literal.
    model: Option<String>,
    /// The module-level variable the agent is assigned to, which is the name other code
    /// imports it by: `agent` in `agent = create_agent(...)`.
    variable: Option<String>,
    /// The function decorated as an agent factory that returns the agent: `researcher`
    /// in crewAI's `@agent def researcher(self): return Agent(...)`.
    factory: Option<String>,
}

#[pymethods]
impl Agent {
    fn __repr__(&self) -> String {
        let name = self.name.as_deref().map_or("None".to_string(), |name| format!("{name:?}"));
        format!(
            "Agent(path={:?}, line={}, framework={:?}, kind={:?}, via={:?}, name={name})",
            self.path, self.line, self.framework, self.kind, self.via
        )
    }
}

/// A prompt found in the source code.
#[pyclass(frozen, get_all, module = "gglint")]
struct Prompt {
    /// The file, relative to the scanned directory, with `/` separators.
    path: String,
    module: String,
    /// 1-based line and column of the prompt's expression.
    line: usize,
    column: usize,
    /// The framework or SDK, such as `pydantic-ai` or `anthropic`, or `messages` for
    /// a `{"role": ..., "content": ...}` dict.
    framework: &'static str,
    /// `"system"`, `"user"` or `"template"`.
    role: &'static str,
    /// What the prompt was passed to, such as `pydantic_ai.Agent`, or `None` for a
    /// message dict.
    via: Option<String>,
    /// The argument it was passed as, such as `system_prompt`, or `return` for a
    /// function whose return value is the prompt.
    argument: &'static str,
    /// The text. Parts only known at runtime are written as `{expression}`.
    text: String,
    /// How many parts of the text are only known at runtime.
    dynamic_parts: usize,
    /// The agent the prompt is given to, by name or variable: the one created by the
    /// call it's passed to, or the one it's passed to otherwise (`agent.run_sync(...)`,
    /// `Task(agent=...)`, `@agent.instructions`), in this file or imported.
    agent: Option<String>,
    /// Other files the text came from, relative to the scanned directory: the module
    /// a constant was imported from, a text file, or a crewAI YAML config.
    sources: Vec<String>,
}

#[pymethods]
impl Prompt {
    fn __repr__(&self) -> String {
        let via = self.via.as_deref().map_or("None".to_string(), |via| format!("{via:?}"));
        format!(
            "Prompt(path={:?}, line={}, role={:?}, via={via}, text={:?})",
            self.path, self.line, self.role, self.text
        )
    }
}

fn relative_path(result: &FileResult) -> String {
    slash_path(&result.relative_path)
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Returns the prompts in the Python files under `path`, ordered by file and position.
#[pyfunction]
fn find_prompts(py: Python<'_>, path: PathBuf) -> PyResult<Vec<Prompt>> {
    if !path.exists() {
        return Err(PyFileNotFoundError::new_err(path.display().to_string()));
    }
    let results = py.detach(|| scan(std::slice::from_ref(&path), false));

    let mut prompts = Vec::new();
    for result in results {
        let relative_path = relative_path(&result);
        let root = result.path.ancestors().nth(result.relative_path.components().count());
        for (location, prompt) in result.prompts {
            let sources = prompt
                .sources
                .iter()
                .map(|source| {
                    let source = root.and_then(|root| source.strip_prefix(root).ok()).unwrap_or(source);
                    slash_path(source)
                })
                .collect();
            prompts.push(Prompt {
                path: relative_path.clone(),
                module: result.module.clone(),
                line: location.line,
                column: location.column,
                framework: prompt.framework,
                role: prompt.role.as_str(),
                via: prompt.via,
                argument: prompt.argument,
                text: prompt.text,
                dynamic_parts: prompt.dynamic_parts,
                agent: prompt.agent,
                sources,
            });
        }
    }
    Ok(prompts)
}

/// Returns the agents defined in the Python files under `path`, ordered by file and
/// position. `path` can also be a single file.
#[pyfunction]
fn find_agents(py: Python<'_>, path: PathBuf) -> PyResult<Vec<Agent>> {
    if !path.exists() {
        return Err(PyFileNotFoundError::new_err(path.display().to_string()));
    }
    let results = py.detach(|| scan(std::slice::from_ref(&path), false));

    let mut agents = Vec::new();
    for result in results {
        let relative_path = relative_path(&result);
        for (location, agent) in result.agents {
            agents.push(Agent {
                path: relative_path.clone(),
                module: result.module.clone(),
                line: location.line,
                column: location.column,
                framework: agent.framework,
                kind: agent.kind.as_str(),
                via: agent.via,
                name: agent.name,
                model: agent.model,
                variable: agent.variable,
                factory: agent.factory,
            });
        }
    }
    Ok(agents)
}

/// Returns the lineage graph of the Python files under `path` as JSON text, in
/// networkx's node-link format. Use it to pass the graph on without parsing it.
#[pyfunction]
fn lineage_json(py: Python<'_>, path: PathBuf) -> PyResult<String> {
    if !path.exists() {
        return Err(PyFileNotFoundError::new_err(path.display().to_string()));
    }
    Ok(py.detach(|| graph::build(scan(std::slice::from_ref(&path), false)).to_json()))
}

/// Returns the lineage graph of the Python files under `path` as a dict, in networkx's
/// node-link format: `networkx.node_link_graph(gglint.lineage_graph(path), edges="edges")`.
#[pyfunction]
fn lineage_graph<'py>(py: Python<'py>, path: PathBuf) -> PyResult<Bound<'py, PyAny>> {
    let json = lineage_json(py, path)?;
    // CPython's `json` module, written in C, builds the dicts faster than building
    // them one at a time through PyO3.
    py.import("json")?.call_method1("loads", (json,))
}

/// Returns the diagram of the agent graphs in the Python files under `path`, as Mermaid
/// text, or as JSON text with `format="json"`. `tier` is the level of detail: `"code"`.
#[pyfunction]
#[pyo3(signature = (path, tier = "code", format = "mermaid"))]
fn diagram(py: Python<'_>, path: PathBuf, tier: &str, format: &str) -> PyResult<String> {
    if !path.exists() {
        return Err(PyFileNotFoundError::new_err(path.display().to_string()));
    }
    let Some(tier) = Tier::parse(tier) else {
        let tiers: Vec<_> = Tier::ALL.iter().map(|tier| tier.as_str()).collect();
        return Err(PyValueError::new_err(format!("tier must be one of {}", tiers.join(", "))));
    };
    let format = match format {
        "mermaid" => Format::Mermaid,
        "json" => Format::Json,
        _ => return Err(PyValueError::new_err("format must be \"mermaid\" or \"json\"")),
    };
    Ok(py.detach(|| views::render(&scan(std::slice::from_ref(&path), false), tier, format)))
}

/// One thing an answer rests on, and where it is in the code.
#[pyclass(frozen, get_all, module = "gglint")]
struct Fact {
    /// The file, relative to the scanned directory, with `/` separators.
    path: String,
    /// 1-based.
    line: usize,
    label: String,
}

#[pymethods]
impl Fact {
    fn __repr__(&self) -> String {
        format!("Fact(path={:?}, line={}, label={:?})", self.path, self.line, self.label)
    }
}

/// An answer to a question about the code.
#[pyclass(frozen, module = "gglint")]
struct Answer {
    inner: ask::Answer,
}

#[pymethods]
impl Answer {
    /// The answer, from the graph of agents and prompts.
    #[getter]
    fn text(&self) -> &str {
        &self.inner.text
    }

    /// What the answer rests on.
    #[getter]
    fn facts(&self) -> Vec<Fact> {
        self.inner
            .facts
            .iter()
            .map(|fact| Fact {
                path: fact.path.clone(),
                line: fact.line,
                label: fact.label.clone(),
            })
            .collect()
    }

    /// The kind of question, such as `"agents_by_model"`.
    #[getter]
    fn intent(&self) -> &'static str {
        self.inner.intent
    }

    /// 0 when keywords understood the question, 1 when the embedding model did.
    #[getter]
    fn layer(&self) -> u8 {
        self.inner.layer
    }

    #[getter]
    fn elapsed_ms(&self) -> f64 {
        self.inner.elapsed.as_secs_f64() * 1000.0
    }

    fn __repr__(&self) -> String {
        format!("Answer(text={:?}, intent={:?}, facts={})", self.inner.text, self.inner.intent, self.inner.facts.len())
    }
}

/// A scanned repository that answers questions about its agents and prompts.
#[pyclass(frozen, module = "gglint")]
struct Repo {
    inner: ask::Repo,
}

#[pymethods]
impl Repo {
    /// Scans `path` and loads the embedding model, downloading it on first use unless
    /// `download` is false. The model is loaded once per process and shared.
    #[new]
    #[pyo3(signature = (path, *, embed = true, download = true))]
    fn new(py: Python<'_>, path: PathBuf, embed: bool, download: bool) -> PyResult<Repo> {
        if !path.exists() {
            return Err(PyFileNotFoundError::new_err(path.display().to_string()));
        }
        let options = ask::Options { embed, download };
        let inner = py.detach(|| ask::Repo::open(std::slice::from_ref(&path), &options));
        Ok(Repo { inner })
    }

    /// Why the embedding model couldn't be loaded, if it couldn't.
    #[getter]
    fn warnings(&self) -> Vec<String> {
        self.inner.warnings.clone()
    }

    /// Answers `question` in milliseconds.
    fn ask(&self, py: Python<'_>, question: &str) -> Answer {
        Answer {
            inner: py.detach(|| self.inner.ask(question)),
        }
    }
}

/// Downloads the embedding model `Repo` uses, if it isn't cached yet, and checks every
/// file's SHA-256. For building it into an image ahead of time.
#[cfg(feature = "embed")]
#[pyfunction]
fn pull_models(py: Python<'_>) -> PyResult<Vec<PathBuf>> {
    use crate::ask::models::{EMBED_FILES, ensure, verify};

    py.detach(|| {
        EMBED_FILES
            .iter()
            .map(|file| ensure(file, true).and_then(|path| verify(file).map(|()| path)))
            .collect::<Result<Vec<_>, _>>()
    })
    .map_err(pyo3::exceptions::PyRuntimeError::new_err)
}

#[pymodule]
fn gglint(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Agent>()?;
    module.add_class::<Prompt>()?;
    module.add_class::<Repo>()?;
    module.add_class::<Answer>()?;
    module.add_class::<Fact>()?;
    module.add_function(wrap_pyfunction!(find_agents, module)?)?;
    module.add_function(wrap_pyfunction!(find_prompts, module)?)?;
    module.add_function(wrap_pyfunction!(lineage_graph, module)?)?;
    module.add_function(wrap_pyfunction!(lineage_json, module)?)?;
    module.add_function(wrap_pyfunction!(diagram, module)?)?;
    #[cfg(feature = "embed")]
    module.add_function(wrap_pyfunction!(pull_models, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
