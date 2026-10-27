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

use std::path::PathBuf;

use pyo3::exceptions::PyFileNotFoundError;
use pyo3::prelude::*;

use crate::discovery::scan;

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
        let relative_path = result
            .relative_path
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
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
    module.add_function(wrap_pyfunction!(find_agents, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
