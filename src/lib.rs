//! gglint: discovers the AI agents and prompts defined in Python source code.
//!
//! The pipeline for each file:
//!
//! ```text
//! source text ─ruff_python_parser─▶ syntax tree ─checker.rs─▶ semantic model + candidates
//!                                                         ─discovery.rs─▶ agents, prompts
//! ```
//!
//! Files are checked in parallel. Afterwards, `project.rs` fills in prompt text that
//! lives in other files: constants imported from other modules, text files read at
//! runtime, and crewAI YAML configs. It only opens the files prompts point to.
//!
//! Tokenizing and parsing are done by Ruff's parser, used as a library. `checker.rs` is
//! Python-specific. `semantic.rs` doesn't know about any language.
//!
//! The same code is used by the `gglint` command (`main.rs`) and, with the `python`
//! feature, by the `gglint` Python package (`python.rs`).

// The syntax tree and model expose more than discovery reads today. Rules will use the rest.
#![allow(dead_code)]

pub mod checker;
pub mod discovery;
pub mod project;
pub mod prompts;
pub mod semantic;

#[cfg(feature = "python")]
mod python;

// Ruff swaps the system allocator for jemalloc or mimalloc. Parsing makes millions of
// tiny allocations, and the macOS system allocator is several times slower at that.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
