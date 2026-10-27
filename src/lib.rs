//! gglint: discovers AI agents defined in Python source code.
//!
//! The pipeline for each file:
//!
//! ```text
//! source text ─ruff_python_parser─▶ syntax tree ─checker.rs─▶ semantic model + candidates
//!                                                         ─discovery.rs─▶ agents
//! ```
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
pub mod semantic;

#[cfg(feature = "python")]
mod python;

// Ruff swaps the system allocator for jemalloc or mimalloc. Parsing makes millions of
// tiny allocations, and the macOS system allocator is several times slower at that.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
