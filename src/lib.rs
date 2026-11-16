//! gglint: keeps the MCP servers of the coding agents in a repository in sync.
//!
//! `harness/` reads the MCP config files of coding agents such as Claude Code, Codex, Cursor
//! and Gemini CLI, compares the MCP servers each one is set up to use, and adds the servers
//! missing from a config file. The `gglint` command (`main.rs`) is a thin layer over it.

pub mod harness;
