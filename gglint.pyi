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
    factory: str | None
    """The function decorated as an agent factory that returns the agent, which is the name the framework knows it by:
    ``researcher`` in crewAI's ``@agent def researcher(self): return Agent(...)``. A factory whose agent can't be seen in
    its body is reported as an ``agent_factory`` of its own."""

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
    """The agent the prompt is given to, by name or variable: the one created by the call it's passed to, or the one it's
    passed to otherwise (``agent.run_sync(...)``, ``Task(agent=...)``, ``@agent.instructions``), in this file or imported."""
    sources: list[str]
    """Other files the text came from, relative to the scanned directory: the module a constant was imported from, a text
    file read at runtime, or a crewAI YAML config."""

def find_prompts(path: str | os.PathLike[str]) -> list[Prompt]:
    """Return the prompts in the Python files under ``path``, ordered by file and position.

    ``path`` can also be a single file. Raises ``FileNotFoundError`` if it doesn't exist.
    """

class GraphNode(t.TypedDict):
    """A node of the lineage graph. Keys not listed for a kind are absent."""

    id: str
    """Unique, and the same between scans of the same code: ``agent:app.bots.support``, ``prompt:app/bots.py:12:19``,
    ``constant:app.prompts.TONE``, ``function:app.prompts.build``, ``file:app/system.md``,
    ``config:app/config/agents.yaml#writer.role``. A repeated id gets ``#2``, ``#3`` in source order."""
    kind: t.Literal["agent", "prompt", "source"]
    label: str
    """A short name to show: the agent's name, the argument a prompt is passed as, or the source's own name."""
    path: str
    """The file, relative to the scanned directory, with ``/`` separators."""
    module: str
    line: int | None
    column: int | None
    framework: t.NotRequired[str]
    """Agents and prompts."""
    agent_kind: t.NotRequired[t.Literal["agent", "agent_class", "agent_factory"]]
    via: t.NotRequired[str | None]
    name: t.NotRequired[str | None]
    model: t.NotRequired[str | None]
    variable: t.NotRequired[str | None]
    factory: t.NotRequired[str | None]
    role: t.NotRequired[t.Literal["system", "user", "template"]]
    argument: t.NotRequired[str]
    text: t.NotRequired[str]
    """Prompts and sources. Parts only known at runtime are written as ``{expression}``."""
    dynamic_parts: t.NotRequired[int]
    source_kind: t.NotRequired[t.Literal["constant", "function", "file", "config"]]

class GraphEdge(t.TypedDict):
    """``feeds``: a source's text is part of a prompt's or another source's, ``position`` being the order the target
    uses its sources in. ``given_to``: a prompt is given to an agent, ``link`` saying how."""

    source: str
    target: str
    kind: t.Literal["feeds", "given_to"]
    position: t.NotRequired[int]
    link: t.NotRequired[t.Literal["constructor", "method", "argument", "decorator", "nested"]]

class LineageGraph(t.TypedDict):
    """networkx's node-link format: ``networkx.node_link_graph(graph, edges="edges")``."""

    directed: t.Literal[True]
    multigraph: t.Literal[False]
    graph: dict[str, int]
    """``{"schema": 1}``, the version of this format."""
    nodes: list[GraphNode]
    edges: list[GraphEdge]

def lineage_graph(path: str | os.PathLike[str]) -> LineageGraph:
    """Return the lineage graph of the Python files under ``path``: the sources prompt text comes from (module-level
    constants, functions, files read at runtime, crewAI config fields), the prompts, and the agents they're given to.

    ``path`` can also be a single file. Raises ``FileNotFoundError`` if it doesn't exist.
    """

def lineage_json(path: str | os.PathLike[str]) -> str:
    """Return ``lineage_graph(path)`` as JSON text, to pass on without parsing it."""
