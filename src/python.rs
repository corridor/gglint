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

use pyo3::exceptions::PyFileNotFoundError;
use pyo3::prelude::*;

use crate::discovery::{FileResult, scan};

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
    /// The agent the prompt belongs to, by name or variable, if it's passed to the call
    /// that creates the agent.
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
            });
        }
    }
    Ok(agents)
}

#[pymodule]
fn gglint(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Agent>()?;
    module.add_class::<Prompt>()?;
    module.add_function(wrap_pyfunction!(find_agents, module)?)?;
    module.add_function(wrap_pyfunction!(find_prompts, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
