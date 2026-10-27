"""Finds the AI agents defined in Python source code. The scan runs in Rust."""

import os
import typing as t

__version__: str

@t.final
class Agent:
    """An agent found in the source code."""

    path: str
    """The file, relative to the scanned directory, with ``/`` separators."""
    module: str
    """The module's dotted path, such as ``myapp.agents``, as if the scanned directory were on ``sys.path``."""
    line: int
    """1-based."""
    column: int
    """1-based."""
    framework: str
    """The framework, such as ``langchain``, ``deepagents`` or ``crewai``."""
    kind: t.Literal["agent", "agent_class", "agent_factory"]
    via: str
    """The qualified name that was called, subclassed or used as a decorator, such as ``langchain.agents.create_agent``."""
    name: str | None
    """The agent's name, if it's written as a literal (``name=`` or ``role=``, or the class or function name)."""
    model: str | None
    """The model, if it's written as a literal."""
    variable: str | None
    """The module-level variable the agent is assigned to, which is the name other code imports it by: ``agent`` in
    ``agent = create_agent(...)``."""

def find_agents(path: str | os.PathLike[str]) -> list[Agent]:
    """Return the agents defined in the Python files under ``path``, ordered by file and position.

    ``path`` can also be a single file. Raises ``FileNotFoundError`` if it doesn't exist.
    """
