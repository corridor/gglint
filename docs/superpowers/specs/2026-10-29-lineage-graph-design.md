# Lineage graph: design and plan

Status: approved in conversation on 2026-10-29. Implementation follows the plan at the end.

## Goal

gglint already finds agents and prompts. GGX needs to draw **which text reaches which agent**:
a many-to-many lineage graph of the text sources, the prompts built from them, and the
agents those prompts are given to. gglint produces the graph data only; drawing is GGX's job.

It must cost almost nothing on top of the scan: the scan is milliseconds to ~1.7 s on a
31,000-file `site-packages`, and the graph must add no more than low single-digit
milliseconds to that, on any corpus.

## References and what was taken from each

| Reference | Taken |
| --- | --- |
| Ruff `analyze graph` (`crates/ruff_graph`, `commands/analyze_graph.rs`) | Collect facts per file in parallel; link in one merge afterwards. Sorted, deterministic JSON output. |
| Ruff `SemanticModel` (`IndexVec<BindingId, Binding>`, `Binding.references`) | `u32` indices into flat vectors instead of pointers or a graph library. |
| Joern Code Property Graph (`REACHING_DEF`, `ARGUMENT` with `ARGUMENT_NAME`) | Definition → use edges; the argument a value was passed as lives on the edge/prompt. |
| sqlglot `lineage` (`Node.downstream`, node cache) | A shared source is **one** node with several parents: that is where many-to-many comes from. |
| sqllineage (`HAS_COLUMN` with `INDEX`) | Keep the order of parts: `feeds` edges carry `position`. sqlglot's unordered `downstream` is not copied. |
| GitHub stack-graphs | A file's analysis never reads another file's syntax tree; cross-file links are symbolic keys resolved at merge time. |
| CodeQL path queries / Pysa | Source → sink framing; prompt sinks stay a registry. |
| Biome module graph, ty, rust-analyzer, `ruff_graph` | None use petgraph: plain vectors and hash maps. |
| networkx node-link JSON, Cytoscape, LangGraph `get_graph()` | Output `{nodes, edges}` with string ids that are stable across scans (names, not counters). Drawing stays outside the library. |

## Model

Nodes:

- **agent**: an agent found by discovery (instance, class or factory).
- **prompt**: a prompt as reported today: text passed to a sink, at the place it is passed.
- **source**: a named definition text comes from:
  - `constant`: a module-level variable assigned text (`TONE = "..."`), in this file or imported;
  - `function`: a function whose return value is used as text (`build()`, `instructions=build`);
  - `file`: a file read at runtime (`Path("system.md").read_text()`);
  - `config`: a field of a crewAI YAML entry (`agents.yaml` → `writer.role`).

  Text written inline at the sink is part of the prompt, not a separate node. Function-local
  variables are transparent: only module-level names can be shared, and sharing is what the
  graph is for.

Edges:

- `feeds` (source → prompt, or source → source): the source's text is part of the target's text.
  `position` is the order the target uses its sources in. `RULES = BASE + "..."` gives
  `BASE → RULES → prompt`, not `BASE → prompt`.
- `given_to` (prompt → agent), with `link`:
  - `constructor`: passed to the call that creates the agent (`Agent(instructions=...)`);
  - `method`: passed to a method of the agent (`agent.run_sync("...")`);
  - `argument`: passed alongside the agent (`Runner.run(agent, "...")`, `Task(agent=a, description=...)`);
  - `decorator`: returned by a function decorated with the agent's method (`@agent.system_prompt`);
  - `nested`: a prompt sink nested directly in an agent's arguments
    (`create_react_agent(prompt=SystemMessage("..."))`).

  An agent bound to a variable, created inline, returned by a crewAI `@agent` factory
  (`agent=self.researcher()`), or imported from another module by its variable is followed.

Ids are stable strings: `agent:<module>.<variable|name>`, `prompt:<path>:<line>:<column>`,
`constant:<module>.<NAME>`, `function:<module>.<name>`, `file:<path>`, `config:<path>#<entry>.<field>`.
A repeated id gets `#2`, `#3` in source order.

## How it is built

1. During the walk (`prompts.rs`), when the text of a module-level name is inlined, it is
   wrapped in `Piece::Origin(binding)` … `Piece::End` markers. They cost two entries per use.
2. `project.rs` turns them into owned `Part::Begin(SourceRef)` / `Part::End`, and adds the same
   markers around imported names, functions' returns, files and config fields as it resolves
   them. `render` and `has_written_text` skip markers, so the reported text is unchanged.
3. Each prompt records the agent it is given to (`AgentTarget`), resolved at the sink with the
   semantic model as it stands there; imported agents keep their absolute qualified name.
4. `graph::build(results)` consumes the scan results (no copying of text), walks each prompt's
   parts once with a small stack, deduplicates sources by key and edges by endpoints, and
   resolves imported agents by module and variable (closest module wins, as for imports).

## Output

- Rust: `gglint::graph::build(Vec<FileResult>) -> Graph { nodes, edges }`, `Graph::to_json()`.
- CLI: `gglint --graph [PATH ...]` prints the JSON.
- Python: `gglint.lineage_graph(path) -> dict` and `gglint.lineage_json(path) -> str` (to forward
  to a frontend without parsing). The dict loads with `networkx.node_link_graph(data, edges="edges")`.

JSON: `{"directed": true, "multigraph": false, "graph": {"schema": 1}, "nodes": [...], "edges": [...]}`.

## Not covered (existing limitations)

Agents re-exported through another module, parameters followed to callers, agents created by
the project's own factory functions, and agent → agent edges (crews, sub-agents).

## Plan

1. Markers: `Piece::Origin/End` in `prompts.rs`; `Part::Begin/End` + `SourceRef` in `project.rs`,
   including imports, functions, files and configs. Existing tests must pass unchanged.
2. Agent targets: `agent` field on sinks, receiver/argument/decorator/nested resolution in
   `checker.rs`, `AgentLink` on `DiscoveredPrompt`.
3. `src/graph.rs`: build, ids, JSON writer, unit tests (many-to-many, nesting, links, cross-file).
4. CLI `--graph`, Python `lineage_graph`/`lineage_json`, `gglint.pyi`, README.
5. Benchmark against the baseline (best of 5): graph build must add ≤ a few ms.
