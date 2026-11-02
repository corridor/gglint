# gglint ask Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Answer questions about a scanned repository from gglint's own results, on the
user's machine, in at most 1 s per answer, with no LLM API.

**Architecture:** A new `ask` module in gglint. Layer 0 is a typed graph of the
scan results with one query per intent. Layer 1 classifies the question and fills its
slots (model2vec static embeddings, BM25, fuzzy name matching), falling back to keywords
without the model. Layer 2 rephrases Layer 0's facts with SmolLM2-360M through llama.cpp
on a worker thread, under a deadline, behind a grounding guard.

**Tech Stack:** Rust 2024, `model2vec-rs` 0.3.0, `bm25` 2.3.2, `gaoya` 0.2.2, `strsim`
0.11.1, `llama-cpp-2` 0.1.158, `ureq` 3.4.2, `sha2` 0.11.1, PyO3 (existing).

**Spec:** `docs/superpowers/specs/2026-10-31-gglint-ask-design.md`

## Global Constraints

- Layer 0 answer < 1 ms; Layer 1 < 10 ms; whole answer including Layer 2 ≤ 1 s.
- Runs on Linux and macOS, x86_64 and arm64, CPU only. No platform-only acceleration.
- llama.cpp: `with_n_gpu_layers(0)`, `with_op_offload(false)`, `with_offload_kqv(false)`,
  threads = physical cores, KV cache of the fixed system prompt reused across questions.
- Models download on first use from Hugging Face, pinned to a commit, SHA-256 verified,
  cached in `$GGLINT_MODELS_DIR`, else `$XDG_CACHE_HOME/gglint/models`, else
  `~/.cache/gglint/models`. Layer 0 never uses the network.
- Only established open-source crates; no hand-written inference, ranking or hashing.
- Every new dependency and model licence is listed in `THIRD_PARTY_LICENSES`.
- Cargo features `embed` and `generate` are on by default.

## Review Focus

- A question naming nothing in the repo ("what does agent foo do" when there's no foo):
  answer says it wasn't found and suggests the closest names. Test in Task 3.
- A repo with no agents or no prompts: every intent answers with a sentence, never an
  empty string or panic. Test in Task 2.
- No network / model download fails: Layers 0 and 1 still answer (keyword fallback).
  Test in Task 7.
- Layer 2 invents a path, number or name: the guard drops it and the template stands.
  Test in Task 6.
- Very large repo (thousands of prompts): Layer 0/1 stay under budget. Test in Task 8.

---

### Task 1: Layer 0 graph

**Files:**

- Create: `src/ask/mod.rs`, `src/ask/graph.rs`
- Modify: `src/lib.rs` (add `pub mod ask;`)

**Interfaces:**

- Produces: `Graph::build(results: &[FileResult]) -> Graph`; `Graph { agents: Vec<AgentNode>, prompts: Vec<PromptNode> }`;
  `AgentNode { path: String, line: usize, framework: &'static str, kind: &'static str, name: Option<String>, model: Option<String>, variable: Option<String>, prompts: Vec<usize> }`;
  `PromptNode { path: String, line: usize, framework: &'static str, role: &'static str, argument: &'static str, text: String, dynamic_parts: usize, agent: Option<usize>, sources: Vec<String> }`;
  `AgentNode::label(&self) -> String` (name, else variable, else `path:line`).
  Paths are relative with `/`. A prompt links to the agent in the same file whose name or variable equals `prompt.agent`.

- [ ] Test `builds_agents_and_links_prompts`: scan the README quick-start repo (Task 1 fixture `tests/common` `support/bot.py` + prompts module); assert one agent `support` with model `openai:gpt-4o`, its `prompts` holds the system prompt, and that prompt's `agent` points back.
- [ ] Implement; `cargo test ask::graph` passes.

### Task 2: Layer 0 queries and answers

**Files:**

- Create: `src/ask/query.rs`

**Interfaces:**

- Produces: `enum Intent { Summary, ListAgents, ListPrompts, AgentDetails, AgentPrompts, AgentsByModel, AgentsByFramework, ListModels, ListFrameworks, DynamicPrompts, ExternalPrompts, DuplicatePrompts, AgentsWithoutSystemPrompt, FileContents, SearchPrompts, Help }` with `Intent::as_str()` (snake_case) and `Intent::from_str`;
  `struct Query { intent: Intent, agent: Option<usize>, model: Option<String>, framework: Option<String>, file: Option<String>, topic: String }`;
  `struct Fact { path: String, line: usize, label: String }`;
  `struct Reply { text: String, facts: Vec<Fact> }`;
  `fn run(graph: &Graph, query: &Query, search: &dyn Fn(&str) -> Vec<usize>) -> Reply`.
  `search` returns prompt indices ranked for a topic (Task 3/5 supply it).
- Duplicates: `gaoya` MinHash LSH over word 3-shingles, Jaccard ≥ 0.8, computed once in `Graph::build` into `Graph.duplicate_groups: Vec<Vec<usize>>`.

- [ ] Tests, one per intent, on a fixture with two agents (langchain `support` gpt-4o; crewai `triage` claude-sonnet-4-5) and three prompts, two near-identical. E.g. `agents_by_model`: `text == "1 agent uses openai:gpt-4o: support."`, facts `[support/bot.py:3]`.
- [ ] Test `every_intent_answers_on_an_empty_repo`: each intent returns non-empty `text`.
- [ ] Implement; tests pass.

### Task 3: Layer 1 without a model: keywords, names, BM25

**Files:**

- Create: `src/ask/understand.rs`, `src/ask/intents.txt`

**Interfaces:**

- Produces: `struct Understanding` built by `Understanding::new(graph: &Graph, embedder: Option<&Embedder>)`;
  `fn parse(&self, graph: &Graph, question: &str) -> Query`;
  `fn search(&self, graph: &Graph, topic: &str) -> Vec<usize>` (BM25 via `bm25::SearchEngine<usize>` over prompt text + agent label + path).
- `intents.txt`: lines `intent_name<TAB>example question`, ≥ 15 examples per intent, `include_str!`'d.
- Entity matching: words and quoted strings matched against agent names/variables, models,
  frameworks and paths with `strsim::jaro_winkler ≥ 0.9` (exact substring wins).
- Keyword fallback picks the intent whose example words overlap most (token Jaccard), with slot-driven overrides (an agent slot + "prompt" → AgentPrompts).
- Not-found: when the question names an agent-like word that matches nothing, `Reply.text` = `No agent named "foo". Closest: support, triage.`

- [ ] Tests: `parses_agents_by_model`, `parses_agent_prompts_with_typo` ("sytem prompt of suport"), `topic_search_ranks_refund_prompt_first`, `unknown_agent_suggests_closest`.
- [ ] Implement; tests pass.

### Task 4: Model downloads

**Files:**

- Create: `src/ask/models.rs`
- Modify: `Cargo.toml` (features `download = ["dep:ureq", "dep:sha2"]`, `embed = ["download", "dep:model2vec-rs"]`, `generate = ["download", "dep:llama-cpp-2"]`, `default = ["embed", "generate"]`)

**Interfaces:**

- Produces: `struct ModelFile { repo: &'static str, revision: &'static str, file: &'static str, sha256: &'static str }`;
  `const EMBED_FILES: [ModelFile; 3]` (potion-code-16M: `config.json`, `tokenizer.json`, `model.safetensors`);
  `const GENERATE_FILE: ModelFile` (SmolLM2-360M-Instruct Q8_0 GGUF);
  `fn models_dir() -> PathBuf`; `fn local_path(file: &ModelFile) -> PathBuf` (`<dir>/<repo with / → __>/<file>`);
  `fn ensure(file: &ModelFile, download: bool) -> Result<PathBuf, String>` streams to `*.part`, hashes while writing, renames on match, deletes on mismatch.
  Revisions and SHA-256 come from the Hugging Face API (`/api/models/<repo>/tree/<rev>` `lfs.oid`).

- [ ] Tests: `models_dir_honours_env`, `ensure_uses_existing_verified_file` (write file, compute its hash into a `ModelFile`, `ensure(.., false)` returns path), `ensure_rejects_wrong_hash`, `ensure_without_download_errors_when_missing`.
- [ ] Implement; tests pass.

### Task 5: Layer 1 with the embedding model

**Files:**

- Create: `src/ask/embed.rs` (behind `embed`)
- Modify: `understand.rs`

**Interfaces:**

- Produces: `struct Embedder` with `Embedder::load(dir: &Path) -> Result<Embedder, String>` and `fn encode(&self, texts: &[String]) -> Vec<Vec<f32>>` (wraps `model2vec_rs::model::StaticModel`).
- `Understanding::new` with an embedder: embeds intent examples once; intent = label of the nearest example by cosine (k = 5 majority); `search` fuses BM25 and cosine ranks by reciprocal rank fusion (k = 60).

- [ ] Test (ignored unless the model is cached): `embedding_intents_beat_keywords` on 10 paraphrases not in `intents.txt`, ≥ 9 correct.
- [ ] Implement; tests pass.

### Task 6: Layer 2 phrasing

**Files:**

- Create: `src/ask/generate.rs` (behind `generate`)

**Interfaces:**

- Produces: `struct Phraser` with `Phraser::spawn(model: PathBuf) -> Phraser` (worker thread owns backend, model and context; loads in the background; warms the system-prompt prefix) and `fn phrase(&self, question: &str, reply: &Reply, deadline: Instant) -> Option<String>` (sends a job, waits until `deadline`; `None` if not loaded, late, or rejected by the guard).
- `fn grounded(text: &str, facts_text: &str) -> bool`: every path-like token (`x/y.py`, `x.py:12`), number and quoted string in `text` occurs in `facts_text`.
- Prompt: system = `You answer questions about a code repository. Use only the facts given. Answer in one short sentence.`; user = `Facts:\n- <reply.text>\n- <each fact label>\nQuestion: <question>`. Greedy, stop at end-of-generation, newline, or deadline; at most 64 tokens.

- [ ] Tests: `grounded_rejects_invented_path`, `grounded_accepts_rephrasing`; ignored model test `phrases_within_one_second`.
- [ ] Implement; tests pass.

### Task 7: `Repo`, `Answer` and the command line

**Files:**

- Modify: `src/ask/mod.rs`, `src/main.rs`, `tests/cli.rs`
- Create: `tests/ask.rs`

**Interfaces:**

- Produces: `struct Options { embed: bool, generate: bool, download: bool, deadline: Duration }` (`Default`: all true, 1 s);
  `struct Answer { text: String, facts: Vec<Fact>, intent: &'static str, layer: u8, elapsed: Duration }`;
  `Repo::open(paths: &[PathBuf], options: &Options) -> Repo` (scan and embedder load run with `rayon::join`; the phraser spawns before the scan);
  `Repo::ask(&self, question: &str) -> Answer` (`layer` = 0 when no Layer 1 model was used, 1 with embeddings, 2 when phrased).
- CLI: `gglint ask [PATH] QUESTION`, `gglint chat [PATH]` (reads questions from stdin, one per line), `gglint models pull`, flags `--no-generate`, `--no-embed`, `--offline`. Answers print the text, then one `path:line  label` per fact, then `(intent, layer N, X ms)` on stderr.
- Plain `gglint [PATH ...]` keeps working unchanged.

- [ ] Tests: `ask_answers_without_models` (`--offline --no-embed --no-generate`), `chat_answers_each_line`, `offline_with_missing_models_still_answers`.
- [ ] Implement; tests pass.

### Task 8: Python bindings, docs, licences, latency

**Files:**

- Modify: `src/python.rs`, `gglint.pyi`, `README.md`, `THIRD_PARTY_LICENSES`
- Create: `tests/latency.rs`

**Interfaces:**

- Python: `class Repo(path, *, embed=True, generate=True, download=True)` with `ask(question) -> Answer`; `Answer(text, facts: list[Fact], intent, layer, elapsed_ms)`; `Fact(path, line, label)`. `ask` releases the GIL.
- `tests/latency.rs` (ignored; run with `cargo test --release --test latency -- --ignored`): a generated repo with 2,000 prompts; asserts Layer 0 < 1 ms and Layer 1 < 10 ms per question, and whole answers ≤ 1 s when the generator is loaded.

- [ ] Implement; `uv run python -c "import gglint; print(gglint.Repo('.').ask('which agents are there'))"` prints an answer.
- [ ] README "Asking questions" section; licences listed.
- [ ] Run the full checks: `cargo test`, `cargo clippy`, the latency test, and the ignored model tests.
