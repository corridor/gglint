//! The MCP config files of the coding agents (harnesses) in a repository.
//!
//! - `mcp.rs` reads the MCP servers Claude Code, Codex, Cursor and OpenCode are each set up
//!   to use.
//! - `compare.rs` finds the servers each config file is missing.
//! - `fix.rs` adds them.

mod compare;
mod fix;
mod mcp;

pub use compare::{Cell, Finding, Rule, ServerCheck, Setup, Site, check_mcp, compare_mcp};
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
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Codex => "codex",
            Harness::Cursor => "cursor",
            Harness::OpenCode => "opencode",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "Claude Code",
            Harness::Codex => "Codex",
            Harness::Cursor => "Cursor",
            Harness::OpenCode => "OpenCode",
        }
    }

    /// The file it reads a repository's MCP servers from, relative to the root.
    pub fn mcp_file(self) -> &'static str {
        mcp::mcp_file(self)
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
