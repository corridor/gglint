# gglint

gglint statically analyses AI agents. It reads a Python codebase and reports:

- **Agents**: every place an agent is created, subclassed or declared, for the major agent
  frameworks.
- **Prompts**: the system prompts, instructions, personas, templates and user messages
  passed to those agents and to the Anthropic, OpenAI and Google model APIs, with their
  text worked out from the source.
- **Lineage**: a graph of which text reaches which agent: the constants, functions, files
  and config entries prompt text comes from, the prompts built from them, and the agents
  they are given to.

It also reads a repository's coding-harness config files:

- **MCP servers**: the MCP servers Claude Code, Codex, Cursor and OpenCode are each set up
  to use, and where they differ: a server one harness gets and another doesn't, or gets
  set up differently.

gglint never runs the code it reads. Nothing is imported, no dependency is installed and
no environment is built, so it is safe to point at any repository, including untrusted
ones. It is written in Rust on top of [Ruff](https://github.com/astral-sh/ruff)'s Python
parser and scans a large repository in well under a second.

It is used from Python as the `gglint` package and from the terminal as the `gglint`
command.

## Contents

- [Quick start](#quick-start)
- [Installation](#installation)
- [Python API](#python-api)
- [Command line](#command-line)
- [Asking questions](#asking-questions)
- [Agents](#agents)
- [Prompts](#prompts)
- [Lineage graph](#lineage-graph)
- [MCP servers](#mcp-servers)
- [How it works](#how-it-works)
- [Performance](#performance)
- [Limitations](#limitations)
- [Development](#development)
- [License](#license)

## Quick start

Given this project:

```python
# support/bot.py
from langchain.agents import create_agent

from support.prompts import TONE

RULES = "Answer from the docs.\nNever guess."

agent = create_agent(
    model="openai:gpt-4o",
    name="support",
    system_prompt=f"You are a support agent. {TONE}\n{RULES}",
)
```

```python
# support/prompts/__init__.py
TONE = "Be friendly and brief."
```

```python
# support/summarise.py
from pathlib import Path

from anthropic import Anthropic

client = Anthropic()


def summarise(ticket: str, messages: list) -> str:
    client.messages.create(
        model="claude-sonnet-4-5",
        max_tokens=512,
        system=(Path(__file__).parent / "prompts" / "summarise.md").read_text(),
        messages=[{"role": "user", "content": f"Summarise ticket {ticket}."}],
    )
    for m in messages:
        client.messages.create(model="x", max_tokens=1, system=m.content, messages=[])
```

gglint reports:

```console
$ gglint .
./support/bot.py:7:9  langchain      agent         via langchain.agents.create_agent  (name="support", model="openai:gpt-4o")
./support/bot.py:10:19  langchain      system prompt via langchain.agents.create_agent(system_prompt=)  "You are a support agent. Be friendly and brief. Answer from the docs. Never gues..."  (agent="support", from ./support/prompts/__init__.py)
./support/summarise.py:12:16  anthropic      system prompt via anthropic.Anthropic.messages.create(system=)  "You summarise support tickets in three bullet points."  (from ./support/prompts/summarise.md)
./support/summarise.py:13:47  messages       user prompt   via message dict  "Summarise ticket {ticket}."  (1 part(s) set at runtime)

Found 1 agent(s) and 3 prompt(s) in 3 file(s) (0.8 KiB); parsed 2 of them; 5.06 ms on 16 threads
```

Note what happened:

- The system prompt was assembled from an f-string, a constant in the same file and a
  constant imported from another module, and linked to the agent it configures.
- The Anthropic system prompt was read from the Markdown file the code opens.
- `{ticket}` is only known at runtime, so it is kept as a placeholder and counted.
- `system=m.content` is not reported. It forwards a value that only exists at runtime
  and has no text in the code (see [What is not reported](#what-is-not-reported)).

## Installation

gglint needs Rust 1.85 or later to build (`rustup` installs it) and Python 3.11 or later
for the Python package. One `abi3` wheel works on every Python version from 3.11 on.

**In the GGX uv workspace**, gglint is a workspace member: `uv sync` builds it with
[maturin](https://www.maturin.rs/), and rebuilds it on the next `uv sync` or `uv run`
whenever its Rust source, `Cargo.toml`, `Cargo.lock` or `pyproject.toml` change. Release
wheels are built by CI, so the Docker image needs no Rust.

**Anywhere else**, install it from its directory:

```sh
pip install ./path/to/gglint # builds a wheel with maturin
maturin develop --release    # or, inside a virtualenv, while working on gglint
```

**The command** is built with Cargo:

```sh
cargo build --release # target/release/gglint
cargo run --release -- path/to/repo
```

## Python API

```python
import gglint

for agent in gglint.find_agents("path/to/repo"):
    print(f"{agent.path}:{agent.line}", agent.framework, agent.kind, agent.name)

for prompt in gglint.find_prompts("path/to/repo"):
    print(f"{prompt.path}:{prompt.line}", prompt.role, prompt.agent, prompt.text)
```

Both functions take a directory or a single `.py` file, return results ordered by file and
position, and raise `FileNotFoundError` when the path doesn't exist. The scan runs in Rust
on every core with the GIL released, so other Python threads keep running. Each call is a
complete, independent scan. The full types are in [`gglint.pyi`](gglint.pyi).

### `find_agents(path) -> list[Agent]`

| Attribute        | Example                         | Meaning                                                                                      |
| ---------------- | ------------------------------- | -------------------------------------------------------------------------------------------- |
| `path`           | `support/bot.py`                | The file, relative to the scanned directory, with `/` separators                             |
| `module`         | `support.bot`                   | The module's dotted path, as if the scanned directory were on `sys.path`                     |
| `line`, `column` | `7`, `9`                        | 1-based                                                                                      |
| `framework`      | `langchain`                     | See [Agents](#agents)                                                                        |
| `kind`           | `agent`                         | `agent`, `agent_class` or `agent_factory`                                                    |
| `via`            | `langchain.agents.create_agent` | The qualified name that was called, subclassed or used as a decorator                        |
| `name`           | `support`                       | The agent's name when written as a literal (`name=`, `role=`, or the class or function name) |
| `model`          | `openai:gpt-4o`                 | The model when written as a literal                                                          |
| `variable`       | `agent`                         | The module-level variable it is assigned to, which is the name other code imports it by      |
| `factory`        | `researcher`                    | The function decorated as an agent factory that returns it, such as crewAI's `@agent`        |

### `find_prompts(path) -> list[Prompt]`

| Attribute        | Example                           | Meaning                                                                                                  |
| ---------------- | --------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `path`           | `support/bot.py`                  | The file, relative to the scanned directory, with `/` separators                                         |
| `module`         | `support.bot`                     | The module's dotted path                                                                                 |
| `line`, `column` | `10`, `19`                        | 1-based position of the prompt's expression                                                              |
| `framework`      | `langchain`                       | The framework or SDK, or `messages` for a `{"role": ..., "content": ...}` dict                           |
| `role`           | `system`                          | `system` (instructions, persona), `user` (a request or task) or `template` (role decided elsewhere)      |
| `via`            | `langchain.agents.create_agent`   | What the prompt was passed to, or `None` for a message dict                                              |
| `argument`       | `system_prompt`                   | The argument it was passed as, or `return` for a function whose return value is the prompt               |
| `text`           | `You are a support agent. ...`    | The text. Parts only known at runtime are written as `{expression}`                                      |
| `dynamic_parts`  | `0`                               | How many parts of `text` are only known at runtime                                                       |
| `agent`          | `support`                         | The agent it is given to, by name, variable or factory: see [Lineage graph](#lineage-graph)              |
| `sources`        | `["support/prompts/__init__.py"]` | Other files the text came from: an imported module, a text file read at runtime, or a crewAI YAML config |

### From Rust

The crate is also a library. `gglint::discovery::scan(&[root], false)` returns a
`FileResult` per Python file found, each with its `agents` and `prompts`.
`gglint::graph::build(results)` turns them into the [lineage graph](#lineage-graph), and
`Graph::to_json()` writes it.

## Command line

```console
$ gglint [--all] [--graph] [PATH ...]
$ gglint mcp [--fix] [--format table|lines] [PATH]
$ gglint mcp --servers [PATH]
```

- With no path it scans the current directory. Several paths are scanned together.
- Each agent and prompt is printed on one line: `path:line:column`, the framework, the
  kind or role, what it was passed to, and its details. Prompt text is flattened to one
  line and cut at 80 characters; the Python API returns it in full.
- A summary goes to standard error: how many agents and prompts were found, how many
  files were read and parsed, the time taken and the number of threads.
- Files with syntax errors produce a warning on standard error and are still analysed.
- It exits with status 1 when there are no Python files under the paths, and 0 otherwise.
- `--all` parses every file, including the ones that cannot contain an agent or prompt.
  It is only useful for measuring the parser.
- `--graph` prints the [lineage graph](#lineage-graph) as JSON instead, and its size and the
  time taken to standard error.
- `gglint mcp` compares the [MCP servers](#mcp-servers) in a repository's harness config
  files instead, and draws them; `--fix` makes them the same, and `gglint mcp --servers`
  lists them.

## Asking questions

`gglint ask` answers questions about the agents and prompts it finds, on your machine,
in milliseconds, without a language model or an LLM API:

```console
$ gglint ask path/to/repo "which agents use gpt-4o"
1 agent uses gpt-4o: support.
  support/bot.py:7  agent support (langchain, openai:gpt-4o)
(agents_by_model, layer 1, 0.41 ms)

$ gglint chat path/to/repo         # one question per line, the repository scanned once
$ gglint models                    # whether the embedding model is downloaded, and where
$ gglint models pull               # download it ahead of time
```

`ask` and `chat` take `--offline` (never download) and `--no-embed` (keywords only).
From Python:

```python
repo = gglint.Repo("path/to/repo")           # embed= and download= are optional
answer = repo.ask("show the system prompt of support")
print(answer.text, answer.intent, answer.elapsed_ms)
for fact in answer.facts:
    print(f"{fact.path}:{fact.line}", fact.label)
```

### What it answers

A summary of the repository; the agents, prompts, models and frameworks; one agent's
details or prompts; the agents using a model or framework; prompts with parts set at
runtime; prompts read from other files; near-duplicate prompts; agents with no system
prompt; what a file defines; and prompts about a topic ("prompts about refunds"). A name
that isn't in the code gets the closest ones instead. Questions the code can't answer,
such as why an agent misbehaves, are out of scope. Every answer lists the facts it rests
on, each with its file and line.

### How it works

| Layer | What it does                                                                                                                                                                                                                                                                                                                                                                                                           | Speed (2,000 prompts) |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- |
| 0     | A graph of the scan: agents, prompts, files, models, linked; near-duplicates found with MinHash ([`gaoya`](https://github.com/serega/gaoya)). One query per kind of question.                                                                                                                                                                                                                                          | < 1 ms                |
| 1     | Understands the question: names matched to the repository's own (Jaro-Winkler, for typos), the kind of question by nearest example question ([`intents.txt`](src/ask/intents.txt)) using static embeddings ([model2vec](https://github.com/MinishLab/model2vec-rs), `potion-code-16M`), topic searches by [BM25](https://github.com/Michael-JB/bm25) fused with embedding similarity. Without the model, word overlap. | < 10 ms               |

A static embedding model is a lookup table from token to vector: encoding a question is
looking its tokens up and averaging them. No neural network runs, so it takes
microseconds on any CPU. It is loaded once per process and shared by every `Repo`.

Measured on a 2019 Intel i9 (8 cores), on the generated 2,000-prompt repository in
[`tests/latency.rs`](tests/latency.rs): answers in 0.3 to 4 ms, the repository scanned
and the embedding model loaded in about 0.5 s.

### The model

| Model                                                                           | Size  | Licence |
| ------------------------------------------------------------------------------- | ----- | ------- |
| [`minishlab/potion-code-16M`](https://huggingface.co/minishlab/potion-code-16M) | 65 MB | MIT     |

It's downloaded from Hugging Face on first use, pinned to a commit and checked against
its SHA-256 ([`src/ask/models.rs`](src/ask/models.rs)), and cached in
`$GGLINT_MODELS_DIR`, else `$XDG_CACHE_HOME/gglint/models`, else
`~/.cache/gglint/models`. For a machine without internet, copy that directory over and
set `GGLINT_MODELS_DIR`. Without it, questions are understood by keywords. Layer 0
never uses the network.

The Cargo feature `embed` is on by default; `--no-default-features` builds gglint with
Layer 0 and keyword understanding only, and nothing to download.

## Agents

| Framework       | Recognised                                                                             |
| --------------- | -------------------------------------------------------------------------------------- |
| `langchain`     | `langchain.agents.create_agent`                                                        |
| `langgraph`     | `langgraph.prebuilt.create_react_agent`                                                |
| `deepagents`    | `deepagents.create_deep_agent`                                                         |
| `pydantic-ai`   | `pydantic_ai.Agent`                                                                    |
| `openai-agents` | `agents.Agent`                                                                         |
| `crewai`        | `crewai.Agent`, `crewai.Crew`, and the `@agent` and `@crew` decorators                 |
| `google-adk`    | `google.adk.agents.Agent`, `LlmAgent`, `SequentialAgent`, `ParallelAgent`, `LoopAgent` |
| `autogen`       | `autogen_agentchat.agents.AssistantAgent`                                              |

The full list, with every module each name is re-exported from, is `REGISTRY` in
[`src/discovery.rs`](src/discovery.rs). Names are resolved the way Python resolves them,
so aliases and module imports are found (`from crewai import Agent as A`,
`import crewai` then `crewai.Agent(...)`), and a class or function that is merely named
`Agent`, or a name rebound before it is called, is not.

There are three kinds:

- `agent`: a call that creates an agent, `agent = create_agent(...)`, including a call to
  a subclass defined in the same file. Agents created inside functions are found too,
  without a `variable`.
- `agent_class`: a class that subclasses an agent, `class Reviewer(Agent): ...`.
- `agent_factory`: a function decorated as an agent whose agent can't be seen in its body,
  `@agent def researcher(self): return build()`.

A decorator only names an agent: the agent is what the function returns. So a factory that
returns an agent created in it, `@agent def researcher(self): return Agent(...)` (or
`a = Agent(...)` then `return a`), is reported once, as that `agent`, with `factory` set to
`researcher`, the name crewAI knows it by.

`name` and `model` are only filled in when written as literals: `name=NAME` or
`model=f"openai:{size}"` leave them empty rather than guessing.

## Prompts

A string is a prompt because of **where it is passed**, not what it says.
`system = platform.system()` is not a prompt and `instructions = "- Use currentColor"` is.
So gglint starts from a registry of _sinks_, the arguments that take a prompt, and works
out the text of whatever is passed to them.

### Where prompts are found

| Framework or SDK | Sinks                                                                                                                                                                                                                                                                          |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pydantic-ai`    | `Agent(system_prompt=, instructions=)`; `agent.run`, `run_sync`, `run_stream`, `iter` (`user_prompt`); functions decorated with `@agent.system_prompt` or `@agent.instructions` (their return value)                                                                           |
| `openai-agents`  | `Agent(instructions=)`; `Runner.run`, `run_sync`, `run_streamed` (`input`)                                                                                                                                                                                                     |
| `crewai`         | `Agent(role=, goal=, backstory=)`; `Task(description=, expected_output=)`; the same fields of an `agents.yaml` or `tasks.yaml` entry passed as `config=self.agents_config["name"]`                                                                                             |
| `google-adk`     | `Agent` / `LlmAgent(instruction=, global_instruction=, static_instruction=)`                                                                                                                                                                                                   |
| `google-genai`   | `types.GenerateContentConfig(system_instruction=)`; `google.generativeai.GenerativeModel(system_instruction=)`                                                                                                                                                                 |
| `autogen`        | `AssistantAgent(system_message=)`                                                                                                                                                                                                                                              |
| `langchain`      | `create_agent(system_prompt=)`; `SystemMessage(content)`, `HumanMessage(content)`; `ChatPromptTemplate.from_messages([(role, text), ...])`, `ChatPromptTemplate.from_template`; `PromptTemplate`; `SystemMessagePromptTemplate` and `HumanMessagePromptTemplate.from_template` |
| `langgraph`      | `create_react_agent(prompt=)`                                                                                                                                                                                                                                                  |
| `deepagents`     | `create_deep_agent(system_prompt=, instructions=)`                                                                                                                                                                                                                             |
| `anthropic`      | `client.messages.create` / `stream` and their `beta` variants (`system=`), on `Anthropic`, `AsyncAnthropic`, `AnthropicBedrock` and `AnthropicVertex`                                                                                                                          |
| `openai`         | `client.responses.create` / `stream` (`instructions=`, `input=`); `client.beta.assistants.create(instructions=)`, on `OpenAI`, `AsyncOpenAI`, `AzureOpenAI` and `AsyncAzureOpenAI`                                                                                             |
| `messages`       | Any `{"role": "system" \| "developer" \| "user", "content": ...}` dict                                                                                                                                                                                                         |

The registry, with every module path each name is re-exported from, is `REGISTRY` and
`CLIENT_METHODS` in [`src/prompts.rs`](src/prompts.rs). Methods are matched through the
object they are called on: `agent.run_sync(...)` is a sink when `agent = Agent(...)`, and
`client.messages.create(...)` when `client = Anthropic()`. A prompt nested in another
sink, such as a `SystemMessage` inside `from_messages`, is reported once.

### How the text is worked out

The text is built from the source without running it:

| Written as                                                                             | Reported as                                                                                                                                                                                                       |
| -------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `"You review code."`, `"a" "b"`                                                        | The literal, with implicit concatenation joined                                                                                                                                                                   |
| `BASE + "\nBe brief."`                                                                 | The parts joined                                                                                                                                                                                                  |
| `f"Today is {date}."`                                                                  | `Today is {date}.`, with `date` filled in when it is a known string                                                                                                                                               |
| `"Hello {name}".format(name=n)`, `.format_map(...)`                                    | The template, placeholders kept                                                                                                                                                                                   |
| `"Hello %s" % name`                                                                    | The template                                                                                                                                                                                                      |
| `"\n".join([intro, rules])`                                                            | The parts joined with the separator, when the list is written out                                                                                                                                                 |
| `.strip()`, `.lstrip()`, `.rstrip()`                                                   | The string it is called on                                                                                                                                                                                        |
| `textwrap.dedent(...)`, `inspect.cleandoc(...)`                                        | The argument                                                                                                                                                                                                      |
| A variable                                                                             | The text of the last assignment to it written before that point                                                                                                                                                   |
| A function in the same module, `build()`                                               | What it returns                                                                                                                                                                                                   |
| `prompts.ROOT`, `from app.prompts import ROOT`, `prompts.build()`                      | The text from that module, even when it lives in another file                                                                                                                                                     |
| `Path("system.md").read_text()`, `open(path).read()`, `with open(path) as f: f.read()` | The contents of the file. Paths may be built from literals and `__file__` with `/`, `.joinpath()`, `.parent`, `.parents[i]`, `.resolve()`, `.absolute()`, `os.path.join`, `os.path.dirname` and `os.path.abspath` |
| `config=self.agents_config["writer"]`                                                  | The fields of that entry in the crewAI YAML config next to the code                                                                                                                                               |

Anything else, such as a function parameter, an attribute of a runtime object or a value
read from the environment, is kept in the text as `{expression}` and counted in
`dynamic_parts`.

Text that lives in other files is filled in by a second pass after every file has been
analysed. It only opens the files prompts point to, even ones the first pass skipped.
Imports are followed up to 4 levels deep and calls to helper functions up to 4 levels,
files read at runtime are only read when they are 1 MiB or smaller, and a prompt is
assembled from at most 256 parts.

### What is not reported

A prompt is something someone wrote. Once every part of the text has been worked out, a
prompt is dropped if none of its text is written in the code, that is, if it is made only
of values known at runtime:

```python
HumanMessage(content=m.content)             # relays a message: not reported
SystemMessage(content=system_prompt)        # a parameter: not reported
{"role": "user", "content": state["input"]} # runtime input: not reported
SystemMessage(content=os.environ["PROMPT"]) # not in the code: not reported

HumanMessage(content=f"Hi {name}, how can I help?")  # reported: "Hi {name}, how can I help?"
SystemMessage(content=SUPPORT)                       # reported, with the text of SUPPORT
```

Whitespace alone does not count as written text, so `f"\n{system_prompt}\n"` is not
reported either.

## Lineage graph

The lineage graph shows which text reaches which agent. It is built from the scan in the
same call and adds a few milliseconds to it (see [Performance](#performance)).

```text
constant TONE ──feeds──▶ prompt (a.py, system_prompt) ──given_to──▶ agent triage
      │
      └──feeds──▶ constant RULES ──feeds──▶ prompt (b.py, instructions) ──given_to──▶ agent helper
file summarise.md ──feeds──▶ prompt (bots.py, system_prompt) ──given_to──▶ agent summariser
```

```python
graph = gglint.lineage_graph("path/to/repo")   # a dict
text = gglint.lineage_json("path/to/repo")     # the same as JSON text, to pass on unparsed

import networkx
G = networkx.node_link_graph(graph, edges="edges")
```

```console
$ gglint --graph path/to/repo > graph.json
```

### Nodes

Every node has `id`, `kind`, `label` (a short name to show), `path`, `module`, `line` and
`column`.

| `kind`   | What it is                                                                                                                                                                                                                           | Other attributes                                                         |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------ |
| `agent`  | An agent, as [found](#agents)                                                                                                                                                                                                        | `framework`, `agent_kind`, `via`, `name`, `model`, `variable`, `factory` |
| `prompt` | A prompt, as [found](#prompts): text passed to a sink, where it is passed                                                                                                                                                            | `framework`, `role`, `via`, `argument`, `text`, `dynamic_parts`          |
| `source` | A named definition prompt text comes from. `source_kind` is `constant` (a module-level variable), `function` (whose return value is used), `file` (read at runtime) or `config` (a field of a crewAI YAML entry) | `source_kind`, `name`, `text`, `dynamic_parts`                           |

Text written inline at the sink is part of the prompt, not a source. Variables local to a
function are not sources either: only module-level names can be shared, and sharing is
what the graph shows. A source used by several prompts is one node with an edge to each,
which is how one constant reaches many agents.

Ids are derived from names, so they stay the same between scans of the same code:
`agent:app.bots.support`, `prompt:app/bots.py:12:19`, `constant:app.prompts.TONE`,
`function:app.prompts.build`, `file:app/system.md`,
`config:app/config/agents.yaml#writer.role`. An agent is named by its variable, then its
factory, then its name. An id that would repeat gets `#2`, `#3` in source order.

### Edges

| `kind`     | From → to                          | Meaning                                                                                                                                                       |
| ---------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `feeds`    | source → prompt, or source → source | The source's text is part of the target's. `position` is the order the target uses its sources in. `RULES = BASE + "..."` gives `BASE → RULES → prompt` |
| `given_to` | prompt → agent                     | The prompt is given to the agent. `link` says how (below)                                                                                                     |

| `link`        | Example                                                                     |
| ------------- | --------------------------------------------------------------------------- |
| `constructor` | `Agent(instructions="...")`                                                 |
| `method`      | `agent.run_sync("...")`, including on an agent imported from another module |
| `argument`    | `Runner.run(agent, "...")`, `Task(description="...", agent=self.researcher())` |
| `decorator`   | `@agent.instructions def rules(): return "..."`                             |
| `nested`      | `create_react_agent(prompt=SystemMessage("..."))`                           |

An agent is followed through a variable (`agent = Agent(...)`), written inline, returned
by a crewAI factory (`self.researcher()`), or imported from another module by its
variable (`from app.bots import support`, `bots.support`), where the closest module of
that name wins. `support.run_sync(...)` on an imported name is only a prompt if `support`
turns out to be an agent whose class has `run_sync` as a sink.

The JSON is networkx's node-link format, which most graph tools read:
`{"directed": true, "multigraph": false, "graph": {"schema": 1}, "nodes": [...], "edges": [...]}`.
`schema` is the version of this format. The full types are `LineageGraph`, `GraphNode`
and `GraphEdge` in [`gglint.pyi`](gglint.pyi).

### Seeing it

gglint does not draw. To look at a graph while working on gglint, a throwaway viewer
writes one HTML page that draws it with Cytoscape.js:

```sh
cargo run --release --example visualize -- path/to/repo graph.html
```

## MCP servers

Each coding harness keeps the MCP servers it starts for a repository in its own file, in
its own format:

| Harness     | File                              | Format                                             |
| ----------- | --------------------------------- | -------------------------------------------------- |
| Claude Code | `.mcp.json`                       | JSON, `mcpServers`                                 |
| Codex       | `.codex/config.toml`              | TOML, `[mcp_servers.<name>]`                       |
| Cursor      | `.cursor/mcp.json`                | JSON, `mcpServers`                                 |
| OpenCode    | `opencode.json`, `opencode.jsonc` | JSON with comments, `mcp.servers` (or `mcp` before V2); the command is one list |

The copies drift apart: a server is added for one harness and not the others, or set up
differently in each. `gglint mcp` reads them all, draws each server in each harness, and
says what to fix:

```console
$ gglint mcp path/to/repo
MCP servers in repo

  ╭───────────────────┬─────────────┬───────┬────────╮
  │ Server            │ Claude Code │ Codex │ Cursor │
  ├───────────────────┼─────────────┼───────┼────────┤
  │ code-review-graph │      ✗      │   ✓   │   ✗    │
  │ angular-cli       │      ✗      │   ✓   │   ≠    │
  ╰───────────────────┴─────────────┴───────┴────────╯
  ✓ set up   ≠ set up differently   ✗ missing   ○ turned off

3 problems

  code-review-graph
    ✗ Missing from Claude Code and Cursor.
      Add it to .mcp.json and .cursor/mcp.json, as set up in .codex/config.toml:3

  angular-cli
    ✗ Missing from Claude Code.
      Add it to .mcp.json, as set up in .codex/config.toml:12
    ≠ Started with different arguments: `mcp --read-only` in Codex; `mcp` in Cursor.
      .codex/config.toml:12, .cursor/mcp.json:3

Checked 3 MCP server(s) in 2 file(s) for Claude Code, Codex and Cursor: 3 problem(s); 0.48 ms
```

- A server's row compares each harness with the first one that has it, in the order of
  the columns: `≠` marks the harnesses where it's set up differently from that one. With a
  `.mcp.json`, that's Claude Code's.
- The symbols are coloured when standard output is a terminal and `NO_COLOR` isn't set.
- The last line goes to standard error.
- It exits with status 1 when it finds a problem or no config files, and 0 otherwise.

`--format lines` prints one line per problem instead, for editors and `grep`: where the
server is set up, the rule and a sentence naming the server, then where the other
harnesses set it up.

```console
$ gglint mcp --format lines path/to/repo
path/to/repo/.codex/config.toml:12:14  missing-server   `angular-cli` is set up for Codex and Cursor but not for Claude Code (.mcp.json)
    also at path/to/repo/.cursor/mcp.json:3:5
```

### Which harnesses are compared

Only the harnesses the repository is set up for: those with their own files in it. A
directory must be a directory: in some repositories `.codex` is an empty file.

| Harness     | Its files                                       |
| ----------- | ----------------------------------------------- |
| Claude Code | `.claude/`, `CLAUDE.md`, `.mcp.json`            |
| Codex       | `.codex/`                                       |
| Cursor      | `.cursor/`, `.cursorrules`                      |
| OpenCode    | `.opencode/`, `opencode.json`, `opencode.jsonc` |

`AGENTS.md` is read by several harnesses, so it doesn't count for any of them. A
repository with a `.claude/` directory and no `.mcp.json` is set up for Claude Code, so
every server the other harnesses get is missing from it.

### What is reported

Servers are matched by name, and then by what they run, so a server renamed in one
harness is still the same server.

| Rule               | Reported when                                                                                                                   |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| `missing-server`   | A harness the repository is set up for doesn't get a server the others do. The message names its file.                          |
| `different-server` | The same server runs another program, URL or version, with other arguments, environment variables or headers, or is turned off. |
| `renamed-server`   | The same server has another name in some harnesses.                                                                             |

How a package is started isn't a difference. `npx -y pkg`, `npx --yes pkg@latest`,
`pnpm dlx pkg`, `yarn dlx pkg`, `bunx pkg` and `bun x pkg` all run the npm package `pkg`,
and `uvx pkg`, `uvx --from pkg pkg`, `uv tool run pkg` and `pipx run pkg` the PyPI package
`pkg`. No version and `latest` both mean the newest version. Environment variables and
headers are compared by name, since each harness writes references to the user's
environment differently (`${KEY}`, `{env:KEY}`, Codex's `env_vars`).

### Fixing

`gglint mcp --fix` changes the config files so every harness gets each server, set up as in
the first harness that has it: Claude Code's `.mcp.json` when there is one. It then draws
the table again, and exits with status 0 if no problem is left.

```console
$ gglint mcp --fix path/to/repo
Fixed 2 files:
  .mcp.json (new)    added code-review-graph, angular-cli
  .cursor/mcp.json   added code-review-graph; updated angular-cli

Review the changes with `git diff`.

MCP servers in repo
...
```

- A missing server is added to the harness's config file, which is created if it doesn't
  exist. A server set up differently is changed to match.
- Each harness's file is written in its own format: `command` and `args`, or OpenCode's
  one `command` list; `type: "http"` or `"sse"` for Claude Code's remote servers; and
  references to the user's environment, in values and command lines, in each harness's
  syntax: `${KEY}` for Claude Code, `${env:KEY}` for Cursor, `{env:KEY}` for OpenCode, and
  Codex's `env_vars`, `env_http_headers` and `bearer_token_env_var`. Codex can't fill in a
  reference in a command line, so such a server is left for a person.
- Only the keys that say how a server is started and what it's given are written. Keys
  only one harness has, such as Codex's `cwd` and `enabled_tools`, are kept, and so are
  comments and formatting: JSON is edited with jsonc-parser's CST and TOML with
  `toml_edit`.
- Running it again changes nothing.

Left for a person to decide, and reported under `Couldn't fix`:

- A different program or URL under the same name.
- A value that can't be written in another harness's syntax, such as Claude Code's
  `${KEY:-default}`, or a variable passed to Codex under another name.

Also left: a server turned off in one harness, which may be on purpose, and a server with
another name, since permission rules refer to servers by name. Both stay in the table.

### Listing the servers

`gglint mcp --servers` lists every server instead, one line each: where its name is
written, the harness, the name, and the command it's started with or the URL it's reached
at. Then, in brackets: the names of its environment variables and HTTP headers, the tools
it's limited to or kept from (Codex's `enabled_tools` and `disabled_tools`), and `disabled`
when the config turns it off.

```console
$ gglint mcp --servers path/to/repo
path/to/repo/.codex/config.toml:3:14  codex        code-review-graph  uvx code-review-graph@latest serve
path/to/repo/.codex/config.toml:12:14  codex        angular-cli  pnpm dlx @angular/cli mcp --read-only
path/to/repo/.cursor/mcp.json:3:5  cursor       angular-cli  npx -y @angular/cli mcp
```

### What it doesn't do

- Print values. Environment variables and headers are listed by name: a config can hold a
  secret in full.
- Start, download or expand anything. A server's package isn't looked up, and references
  to the user's environment are left as references, even when `--fix` writes them for
  another harness.
- Read anything outside the repository's root. Servers set up for one user (Claude Code's
  `~/.claude.json`, `~/.codex/config.toml`) aren't in the repository, and neither are the
  servers Claude Code plugins add. A server a plugin gives Claude Code is reported as
  missing from it.
- Take into account that Codex only reads `.codex/config.toml` in projects the user has
  trusted.

A file that doesn't parse, or a server with neither a command nor a URL, is reported as a
warning on standard error.

From Rust, `gglint::harness::read_mcp(path)` returns each file's servers, and
`check_mcp(&configs, &used_harnesses(path))` each server's cells and findings.

## How it works

```text
             ┌──────────────── per file, in parallel ────────────────┐
.py files ─▶ prefilter ─▶ parse ─▶ checker ─▶ agents + prompt sinks ─┼─▶ cross-file pass ─▶ filter ─▶ results
             └───────────────────────────────────────────────────────┘
```

1. **Find the files** ([`discovery.rs`](src/discovery.rs)): every `.py` file under the
   paths, skipping hidden folders, `venv`, `node_modules`, `__pycache__`, `target` and
   `site-packages`.
2. **Prefilter**: a file can only create an agent or pass a prompt to an SDK if it
   mentions the framework's top-level module, so a SIMD byte search for those names
   (`memchr`) skips most files before they are parsed.
3. **Parse** with Ruff's parser, which recovers from syntax errors, so a file with a
   mistake still yields its other results.
4. **Resolve names** ([`checker.rs`](src/checker.rs), [`semantic.rs`](src/semantic.rs)): a
   walk modelled on Ruff's checker records what each name refers to at each point
   (imports, assignments, `def`s, `class`es, scopes) and the qualified name of every call,
   class base and decorator.
5. **Match**: agent calls, bases and decorators are compared with the agent registry
   ([`discovery.rs`](src/discovery.rs)). When the walk reaches a prompt sink, the text of
   the argument is worked out right there, using the semantic model as it stands at that
   point ([`prompts.rs`](src/prompts.rs)). Text is kept as pieces that borrow from the
   syntax tree, so nothing is copied until a prompt is reported, and a call that is not a
   sink costs one hash lookup.
6. **Cross-file pass** ([`project.rs`](src/project.rs)): fills in names imported from the
   project's other modules, files read at runtime and crewAI YAML configs. When several
   modules match an import, the one closest to the importing file wins, which handles
   repositories holding several projects.
7. **Link imported agents** ([`discovery.rs`](src/discovery.rs)): prompts given to an agent
   imported from another module are linked to it by module and variable.
8. **Filter**: prompts with no written text are dropped.
9. **Graph**, on request ([`graph.rs`](src/graph.rs)): while text is worked out, the parts
   that come from a source are marked, and each prompt records the agent it is given to.
   The graph is built from those in one pass over the results, without reading any code.
   The design, and the tools it follows (Ruff's `analyze graph`, Joern's code property
   graph, sqlglot's lineage, GitHub's stack-graphs), are in
   [the design notes](docs/superpowers/specs/2026-10-29-lineage-graph-design.md).

Files are processed in parallel with `rayon`, and `mimalloc` replaces the system
allocator, which is much slower for the many small allocations parsing makes.

## Performance

Measured with the release build on a 16-thread Mac, wall-clock time for the whole scan:

| Codebase                              | Files  | Size    | Parsed | Time    |
| ------------------------------------- | ------ | ------- | ------ | ------- |
| `langgraph` repository                | 479    | 6.8 MiB | 394    | ~70 ms  |
| 130 agent-skill repositories          | 393    | 6.1 MiB | 75     | ~200 ms |
| A Python 3.13 `site-packages`         | 13,155 | 142 MiB | 2,230  | ~600 ms |
| Python 3.13 standard library, `--all` | 1,748  | 32 MiB  | 1,748  | ~110 ms |

Scan time grows with the number of files that mention a framework, since the others are
skipped before parsing, not with the size of the codebase.

Building the lineage graph and writing its JSON adds, on the same machine:

| Codebase                             | Files  | Nodes  | Edges  | Added   |
| ------------------------------------ | ------ | ------ | ------ | ------- |
| `deepagents` repository              | 1,106  | 1,412  | 49     | ~4 ms   |
| A Python 3.14 `site-packages`        | 31,016 | 21     | 5      | ~2 ms   |
| Generated: 10,000 agent files        | 11,000 | 82,000 | 91,000 | ~200 ms |

Its cost grows with the number of agents, prompts and sources, not with the size of the
codebase.

`gglint mcp` opens a few files at fixed paths and searches for nothing, so its time
doesn't depend on the size of the repository. Reading and comparing four harnesses'
configs with 83 servers between them takes under 0.6 ms, much less than starting the
program.

## Limitations

gglint reads code one file at a time and does not run it, so it reports what the code
says, not what it does:

- **Agents** re-exported through another module (an `__init__.py` that imports and passes
  them on), compiled LangGraph `StateGraph`s, and agents returned by the project's own
  undecorated factory functions are not linked yet. An agent is found only in a file that
  imports its framework, and a prompt only in a file that imports a framework or SDK, so a
  file that runs an imported agent without importing any framework is not read.
- **Prompts passed on in messages**, such as `agent.invoke({"messages": [HumanMessage(...)]})`,
  are found, but not linked to the agent, because `invoke` is not a registered sink.
- **Function parameters are not followed to their callers.** A prompt passed into a
  function, such as `def make(system_prompt): create_agent(system_prompt=system_prompt)`,
  has no written text at the sink and is not reported, even when a caller passes a
  literal.
- **One value per name.** A variable takes the text of the last assignment written before
  it is used, so when it is assigned in both branches of an `if`, only the `else` branch
  is reported. A function with several `return`s has no single text, so it is treated as
  only known at runtime.
- **Path helpers** other than those listed above, such as `Path.with_name`, are not
  followed, so a file opened through them is not read.
- **Test files are scanned** like any other code, so string fixtures in tests, such as
  `HumanMessage(content="hello")`, are reported.
- **Only the registered sinks are seen.** A prompt passed to an SDK or argument that is not
  in the registry is not found; see [Development](#development) to add one.
- Files that are not UTF-8 are skipped.

## Development

```sh
cargo test                          # unit tests in src/, integration tests in tests/
cargo run --release -- path/to/repo # the command
uv run python -c "import gglint"    # the Python package, after `uv sync` in the GGX workspace
```

Unit tests live next to the code they test. [`tests/scan.rs`](tests/scan.rs) scans real
directories, [`tests/graph.rs`](tests/graph.rs) builds lineage graphs from them and
[`tests/cli.rs`](tests/cli.rs) runs the command.

### Project layout

| Path                                   | Purpose                                                             |
| -------------------------------------- | ------------------------------------------------------------------- |
| [`src/discovery.rs`](src/discovery.rs) | Finding files, the prefilter, the agent registry and the scan       |
| [`src/checker.rs`](src/checker.rs)     | The walk over the syntax tree that builds the semantic model        |
| [`src/semantic.rs`](src/semantic.rs)   | Scopes, bindings and qualified names                                |
| [`src/prompts.rs`](src/prompts.rs)     | The prompt sink registry and how prompt text is worked out          |
| [`src/project.rs`](src/project.rs)     | The cross-file pass: imports, files read at runtime, crewAI configs |
| [`src/graph.rs`](src/graph.rs)         | The lineage graph and its JSON                                      |
| [`src/harness/`](src/harness/)         | The coding harnesses' MCP config files, and comparing them          |
| [`examples/visualize.rs`](examples/visualize.rs) | A throwaway HTML viewer for the graph                     |
| [`src/ask/`](src/ask/)                 | `gglint ask`: the graph, queries, understanding and its model       |
| [`src/python.rs`](src/python.rs)       | The Python bindings, behind the `python` feature                    |
| [`src/main.rs`](src/main.rs)           | The `gglint` command                                                |
| [`gglint.pyi`](gglint.pyi)             | Type stubs for the Python package                                   |

### Common changes

- **Adding an agent framework**: add its public import paths to `REGISTRY` in
  `src/discovery.rs`, including every module the framework re-exports the name from, and
  its top-level module is picked up by the prefilter automatically. Add a test next to the
  existing ones.
- **Adding a prompt sink**: add it to `REGISTRY` in `src/prompts.rs`, or to
  `CLIENT_METHODS` for a method on an SDK client, with the arguments that take a prompt
  and their role. A sink that isn't the call creating the agent says which agent its
  prompts are given to (`AgentArg`: the receiver, as in `agent.run_sync`, or an argument,
  as in `Task(agent=...)`). Add a test in `src/prompts.rs`.
- **Adding to the lineage graph**: a new kind of node is a `NodeData` variant and a new
  kind of edge an `EdgeKind` variant in `src/graph.rs`, both written by `Graph::to_json`.
  What they come from is recorded during the scan, as prompts record their sources and
  agent, never by reading code in `graph.rs`. Raise `SCHEMA_VERSION` when the JSON changes
  in a way readers must know about, keep `gglint.pyi` in step, and add a test in
  `tests/graph.rs`.
- **Changing the Python API**: the bindings are in `src/python.rs`, behind the `python`
  feature that maturin turns on. Keep `gglint.pyi` in step with them.
- **Upgrading Ruff's crates**: `ruff_python_ast`, `ruff_python_parser` and
  `ruff_text_size` are internal crates with no stability promise, so they are pinned to an
  exact version in `Cargo.toml`. Upgrade all three together and run the tests.

## License

Apache-2.0, see [`LICENSE`](LICENSE). The parser and the design of the semantic model come
from Ruff, under the MIT license: see [`THIRD_PARTY_LICENSES`](THIRD_PARTY_LICENSES).
