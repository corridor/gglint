# gglint

gglint statically analyses AI agents. It reads a Python codebase and reports:

- **Agents**: every place an agent is created, subclassed or declared, for the major agent
  frameworks.
- **Prompts**: the system prompts, instructions, personas, templates and user messages
  passed to those agents and to the Anthropic, OpenAI and Google model APIs, with their
  text worked out from the source.

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
- [Agents](#agents)
- [Prompts](#prompts)
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
| `agent`          | `support`                         | The agent it configures, by name or variable, when passed to the call that creates the agent             |
| `sources`        | `["support/prompts/__init__.py"]` | Other files the text came from: an imported module, a text file read at runtime, or a crewAI YAML config |

### From Rust

The crate is also a library. `gglint::discovery::scan(&[root], false)` returns a
`FileResult` per Python file found, each with its `agents` and `prompts`.

## Command line

```console
$ gglint [--all] [PATH ...]
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
- `agent_factory`: a function decorated as an agent, `@agent def researcher(self): ...`.

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
7. **Filter**: prompts with no written text are dropped.

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

## Limitations

gglint reads code one file at a time and does not run it, so it reports what the code
says, not what it does:

- **Agents** built in another file and imported, compiled LangGraph `StateGraph`s, and
  agents returned by the project's own factory functions are not found yet. An agent is
  found only in a file that imports its framework.
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
directories and [`tests/cli.rs`](tests/cli.rs) runs the command.

### Project layout

| Path                                   | Purpose                                                             |
| -------------------------------------- | ------------------------------------------------------------------- |
| [`src/discovery.rs`](src/discovery.rs) | Finding files, the prefilter, the agent registry and the scan       |
| [`src/checker.rs`](src/checker.rs)     | The walk over the syntax tree that builds the semantic model        |
| [`src/semantic.rs`](src/semantic.rs)   | Scopes, bindings and qualified names                                |
| [`src/prompts.rs`](src/prompts.rs)     | The prompt sink registry and how prompt text is worked out          |
| [`src/project.rs`](src/project.rs)     | The cross-file pass: imports, files read at runtime, crewAI configs |
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
  and their role. Add a test in `src/prompts.rs`.
- **Changing the Python API**: the bindings are in `src/python.rs`, behind the `python`
  feature that maturin turns on. Keep `gglint.pyi` in step with them.
- **Upgrading Ruff's crates**: `ruff_python_ast`, `ruff_python_parser` and
  `ruff_text_size` are internal crates with no stability promise, so they are pinned to an
  exact version in `Cargo.toml`. Upgrade all three together and run the tests.

## License

Apache-2.0, see [`LICENSE`](LICENSE). The parser and the design of the semantic model come
from Ruff, under the MIT license: see [`THIRD_PARTY_LICENSES`](THIRD_PARTY_LICENSES).
