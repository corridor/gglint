"""Finds the AI agents and prompts defined in Python source code. The scan runs in Rust."""

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

@t.final
class Prompt:
    """A prompt found in the source code: text passed to an agent or a model API."""

    path: str
    """The file, relative to the scanned directory, with ``/`` separators."""
    module: str
    """The module's dotted path, such as ``myapp.agents``."""
    line: int
    """1-based line of the prompt's expression."""
    column: int
    """1-based."""
    framework: str
    """The framework or SDK, such as ``pydantic-ai`` or ``anthropic``, or ``messages`` for a ``{"role": ..., "content": ...}``
    dict."""
    role: t.Literal["system", "user", "template"]
    via: str | None
    """What the prompt was passed to, such as ``pydantic_ai.Agent``, or ``None`` for a message dict."""
    argument: str
    """The argument it was passed as, such as ``system_prompt``, or ``return`` for a function whose return value is the
    prompt (``@agent.instructions``)."""
    text: str
    """The text. Parts only known at runtime are written as ``{expression}``: ``"Today is {date}."``."""
    dynamic_parts: int
    """How many parts of ``text`` are only known at runtime."""
    agent: str | None
    """The agent the prompt belongs to, by name or variable, if it's passed to the call that creates the agent."""
    sources: list[str]
    """Other files the text came from, relative to the scanned directory: the module a constant was imported from, a text
    file read at runtime, or a crewAI YAML config."""

def find_prompts(path: str | os.PathLike[str]) -> list[Prompt]:
    """Return the prompts in the Python files under ``path``, ordered by file and position.

    ``path`` can also be a single file. Raises ``FileNotFoundError`` if it doesn't exist.
    """
