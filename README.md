# gglint

gglint is a set of tools to statically analyze agents.

The components it contains are:

- `detector` - finds the AI agents defined in Python source code. It reads the files and never
  runs them: nothing is imported and no environment is built, so it is safe to point at
  any repository, and a large one takes milliseconds.

## Detector

The detector is written in Rust, parses Python with [Ruff](https://github.com/astral-sh/ruff)'s
parser, and is used from Python as the `gglint` package or from the terminal as the
`gglint` command.

### Use from Python

```python
import gglint

for agent in gglint.find_agents("path/to/repo"):
    print(agent.path, agent.line, agent.framework, agent.kind, agent.variable)
```

`find_agents` takes a directory or a single file and returns the agents ordered by file
and position. It raises `FileNotFoundError` when the path doesn't exist. The scan runs in
Rust on every core, with the GIL released.

Each `Agent` has:

| Attribute        | Example                         | Meaning                                                                                      |
| ---------------- | ------------------------------- | -------------------------------------------------------------------------------------------- |
| `path`           | `support/bot.py`                | The file, relative to the scanned directory, with `/` separators                             |
| `module`         | `support.bot`                   | The module's dotted path, as if the scanned directory were on `sys.path`                     |
| `line`, `column` | `3`, `9`                        | 1-based                                                                                      |
| `framework`      | `langchain`                     | See [What it finds](#what-it-finds)                                                          |
| `kind`           | `agent`                         | `agent`, `agent_class` or `agent_factory`                                                    |
| `via`            | `langchain.agents.create_agent` | What was called, subclassed or used as a decorator                                           |
| `name`           | `support`                       | The agent's name when written as a literal (`name=`, `role=`, or the class or function name) |
| `model`          | `openai:gpt-4o`                 | The model when written as a literal                                                          |
| `variable`       | `agent`                         | The module-level variable it is assigned to: the name to import it by                        |

The types are in [`gglint.pyi`](gglint.pyi).

### Use from the terminal

```console
$ cargo run --release -- path/to/repo
support/bot.py:3:9  langchain      agent         via langchain.agents.create_agent  (name="support", model="openai:gpt-4o")
support/crew.py:4:10  crewai         agent         via crewai.Agent  (name="writer")
support/crew.py:9:9  crewai         agent factory via crewai.project.agent  (name="researcher")

Found 3 agent(s) in 2 file(s) (0.3 KiB); parsed 2 of them; 1.57 ms on 15 threads
```

With no path it scans the current directory. `--all` parses every file, even the ones
that cannot contain an agent; it is only useful for measuring the parser.

## What it finds

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
so aliases and module imports are found too (`from crewai import Agent as A`,
`import crewai` then `crewai.Agent(...)`), and a class or function that is merely named
`Agent` is not.

There are three kinds:

- `agent`: a call that creates an agent, `agent = create_agent(...)`, including a call
  to a subclass defined in the same file.
- `agent_class`: a class that subclasses an agent, `class Reviewer(Agent): ...`.
- `agent_factory`: a function decorated as an agent, `@agent def researcher(self): ...`.

Not found yet: agents built in another file of the repository and imported, compiled
LangGraph `StateGraph`s, and agents returned by the project's own factory functions.
Each file is read on its own, so an agent is found only where the framework is imported.

## How it works

```text
source ─ruff_python_parser─▶ syntax tree ─checker.rs─▶ semantic model + candidates ─discovery.rs─▶ agents
```

1. **Find the files** (`discovery.rs`): every `.py` file under the path, skipping hidden
   folders, `venv`, `node_modules`, `__pycache__`, `target` and `site-packages`.
2. **Skip files that cannot contain an agent**: a file can only create a framework's
   agent if it mentions the framework's top-level module, so a fast byte search for those
   names skips most files before parsing.
3. **Parse** with Ruff's parser, which recovers from syntax errors, so a file with a
   mistake still yields its other agents.
4. **Resolve names** (`checker.rs`, `semantic.rs`): a walk modelled on Ruff's checker
   records what each name refers to at each point (imports, assignments, `def`s,
   `class`es, scopes), and every call, class base and decorator with its qualified name.
5. **Match** (`discovery.rs`): those qualified names are compared with the registry.

The files are processed in parallel with `rayon`, and `mimalloc` replaces the system
allocator, which is much slower for the many small allocations parsing makes.

## Development

gglint needs Rust 1.85 or later (`rustup` installs it). It is a member of the uv
workspace, so `uv sync` builds it with [maturin](https://www.maturin.rs/) and rebuilds it
whenever its Rust source changes. Release wheels are built by CI, so the Docker image
needs no Rust.

```sh
cargo test                          # unit tests in src/, and tests/ for scans of real folders and the command
cargo run --release -- path/to/repo # the command
uv run python -c "import gglint"    # the Python package, after `uv sync`
```

- **Adding a framework**: add its public import paths to `REGISTRY` in
  `src/discovery.rs` (every module the framework re-exports the name from), and a test
  next to the existing ones.
- **Upgrading Ruff's crates**: `ruff_python_ast`, `ruff_python_parser` and
  `ruff_text_size` are internal crates with no stability promise, so they are pinned to
  an exact version in `Cargo.toml`. Upgrade all three together and run the tests.
- **Python bindings** are in `src/python.rs`, behind the `python` feature that maturin
  turns on. Keep `gglint.pyi` in step with them.

## License

Apache-2.0, see [`LICENSE`](LICENSE). The parser and the design of the semantic model
come from Ruff, under the MIT license: see [`THIRD_PARTY_LICENSES`](THIRD_PARTY_LICENSES).
