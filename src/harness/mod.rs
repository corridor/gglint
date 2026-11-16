//! The MCP config files of the coding agents (harnesses) in a repository.
//!
//! - `mcp.rs` reads the MCP servers Claude Code, Codex, Cursor and OpenCode are each set up
//!   to use.
//! - `compare.rs` finds how they differ: missing servers, missing config files, and servers
//!   set up differently.
//! - `fix.rs` adds the missing servers and config files, and starts each server the same way.

mod agents;
mod compare;
mod fix;
mod mcp;

pub use compare::{
    Cell, Finding, McpCheck, Rule, ServerCheck, Setup, Site, check_mcp, check_mcp_from, compare_mcp, compared_harnesses,
};
pub use fix::{ChangedFile, FixReport, Unfixed, fix_mcp};
pub use mcp::{McpConfig, McpServer, Part, Setting, Transport, read_mcp};

/// A program that runs a model as a coding agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Harness {
    ClaudeCode,
    Codex,
    Cursor,
    OpenCode,
}

/// All of them, in the order they're reported.
pub const HARNESSES: [Harness; 4] = [Harness::ClaudeCode, Harness::Codex, Harness::Cursor, Harness::OpenCode];

impl Harness {
    pub fn as_str(self) -> &'static str {
        agents::agent(self).id
    }

    /// The harness named `name`, as `as_str` writes it or by another name it goes by, such
    /// as `claude` for Claude Code.
    pub fn parse(name: &str) -> Option<Harness> {
        let name = name.to_ascii_lowercase();
        agents::AGENTS
            .iter()
            .find(|agent| agent.id == name || agent.aliases.contains(&name.as_str()))
            .map(|agent| agent.harness)
    }

    pub fn display_name(self) -> &'static str {
        agents::agent(self).name
    }

    /// The file it reads a repository's MCP servers from, relative to the root: the one
    /// created when it has none.
    pub fn mcp_file(self) -> &'static str {
        agents::agent(self).files[0]
    }

    /// Its own directory in a repository. When it's there, the repository is set up for the
    /// agent on purpose, not just mentioned by a file such as `CLAUDE.md`.
    pub fn directory(self) -> &'static str {
        agents::agent(self).directory
    }
}

/// A 1-based line and column in a config file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}
