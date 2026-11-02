# gglint ask: answering questions about a repository without an LLM API

> **Update 2026-10-31:** Layer 2 was removed. It used a small transformer language model
> (SmolLM2-360M via llama.cpp), which the requirements rule out, and its sentences could
> mislead. `gglint ask` is Layers 0 and 1 only. GGX's GG Code asks gglint through
> `POST /code-repos/{id}/ask` and only displays the answer. The Layer 2 sections below are kept
> as the record of what was tried.

## Goal

GG code answers questions about a repository, and runs read-only actions on it, from
gglint's own scan, on the user's machine, with no LLM API.

## Non-negotiable: latency

| What                                                  | Budget  |
| ----------------------------------------------------- | ------- |
| Layer 0 answer (graph query)                          | < 1 ms  |
| Layer 1 answer (intent, search, entity matching)      | < 10 ms |
| A whole answer, Layer 2 phrasing included             | ≤ 1 s   |
| Startup: scan + index + model load, on a typical repo | < 1 s   |

Per-question budgets apply once the repository is loaded (`gglint chat`, `Repo`). Models
load on a background thread while the scan runs. Layer 2 runs under a hard deadline: it
stops at 1 s and the answer from Layers 0 and 1 stands.

## Layers

**Layer 0: graph.** `ask::graph` builds a typed in-memory graph from `scan()`'s
`FileResult`s: nodes for agents, prompts, files, models and frameworks; edges for "uses
prompt", "defined in", "uses model", "text from". `ask::query` has one function per
intent. No new dependencies. Near-duplicate prompts use MinHash over word shingles.

**Layer 1: understanding.** `ask::embed` wraps `model2vec-rs` with
`minishlab/potion-code-16M` (MIT, 64 MB). It is used to:

- classify the question by nearest neighbour against example questions committed in
  `src/ask/intents.txt`; no training step,
- rank prompts for topic searches, fused with an in-house BM25 (`ask::bm25`) by
  reciprocal rank fusion,
- fill the question's slots by matching its words to the repository's own names
  (agents, variables, models, frameworks, files).

Without the model (not downloaded yet, offline), Layer 1 falls back to keyword rules and
BM25 alone.

**Layer 2: phrasing.** `ask::generate` runs SmolLM2-360M-Instruct (Q8_0 GGUF,
Apache-2.0) through llama.cpp, using the `llama-cpp-2` crate, to turn the facts Layer 0
found into one sentence. Layer 0 passes only the facts that answer the question, so the
model rephrases an answer rather than choosing one. Settings that make it fast on any
CPU: CPU only (no GPU layers, no op or KV offload), threads = physical cores, the KV
cache of the fixed system prompt kept between questions, greedy decoding capped by the
deadline. Output naming a path, number or quoted name absent from the facts is discarded.

### Spike results (2019 Intel i9-9880H, 8 cores, CPU only, portable build)

| Model                 | Runtime   | Decode         | Whole answer | Correct (of 4)                                           |
| --------------------- | --------- | -------------- | ------------ | -------------------------------------------------------- |
| RWKV-7 G1d 0.1B f32   | candle    | 53 ms/token    | 4–6 s        | 0                                                        |
| SmolLM2-135M f32      | candle    | 60–80 ms/token | 1.5–3 s      | 2                                                        |
| SmolLM2-135M Q8_0     | llama.cpp | 7 ms/token     | 110–310 ms   | 2                                                        |
| **SmolLM2-360M Q8_0** | llama.cpp | 17 ms/token    | 300–740 ms   | **4**                                                    |
| Qwen2.5-0.5B Q4_K_M   | llama.cpp | 18 ms/token    | 280–750 ms   | 4 (load 0.9 s)                                           |
| RWKV-7 G1 0.4B Q4_K_M | llama.cpp | 14 ms/token    | ~400 ms      | wrong on the first; recurrent state can't reuse a prefix |

candle's CPU path is dominated by per-op overhead; llama.cpp is the standard CPU engine
and dispatches to AVX2/AVX-512/NEON. Building llama.cpp needs CMake and a C++ compiler.

## Interfaces

- CLI: `gglint ask [PATH] "question"`, `gglint chat [PATH]`, `gglint models pull`.
- Python: `gglint.Repo(path).ask(question) -> Answer` with `text`, `facts`
  (path, line, label), `intent`, `layer`, `elapsed_ms`.
- Rust: `gglint::ask::Repo::open(paths)`, `repo.ask(question) -> Answer`.

## Models and licences

Downloaded on first use from Hugging Face, pinned to a commit, verified by SHA-256 and
cached in `~/.cache/gglint/models` (`GGLINT_MODELS_DIR` overrides it, for offline
machines). `gglint models pull` downloads ahead of time. Layer 0 never uses the network.

| Artifact                                   | Licence           |
| ------------------------------------------ | ----------------- |
| `minishlab/potion-code-16M`                | MIT               |
| `HuggingFaceTB/SmolLM2-360M-Instruct-GGUF` | Apache-2.0        |
| `model2vec-rs`                             | MIT               |
| `llama-cpp-2`, `llama-cpp-sys-2`           | MIT OR Apache-2.0 |
| llama.cpp (built by `llama-cpp-sys-2`)     | MIT               |
| `tokenizers`, `safetensors`                | Apache-2.0        |

Every one is added to `THIRD_PARTY_LICENSES`.

## Build

Cargo features `embed` (Layer 1 model) and `generate` (Layer 2) are on by default, so
`pip install gglint` and `cargo install` get everything. Embedders can turn them off.

## Out of scope

Code changes (extracting a prompt to a file, renaming an agent), languages other than
Python, and open questions the graph cannot answer ("why does this agent hallucinate").

## Testing

Unit tests per layer, end-to-end question tests against fixture repositories, and latency
tests that fail over budget. Measured on the `langgraph` repository and reported.
