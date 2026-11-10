//! The coding harnesses' config files in a repository, and how their setups compare.
//!
//! - `mcp.rs` reads the MCP servers Claude Code, Codex, Cursor and OpenCode are each set up
//!   to use.
//! - `compare.rs` checks that every harness the repository is set up for gets the same
//!   servers, set up the same way.
//! - `fix.rs` writes the config files so they do.

mod compare;
mod fix;
mod mcp;

use std::path::Path;

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

/// Files and directories that show a repository is set up for a harness, relative to its
/// root. `AGENTS.md` is read by several harnesses, so it doesn't count for any of them.
const MARKERS: &[(Harness, &str)] = &[
    (Harness::ClaudeCode, ".claude"),
    (Harness::ClaudeCode, "CLAUDE.md"),
    (Harness::ClaudeCode, ".mcp.json"),
    (Harness::Codex, ".codex"),
    (Harness::Cursor, ".cursor"),
    (Harness::Cursor, ".cursorrules"),
    (Harness::OpenCode, ".opencode"),
    (Harness::OpenCode, "opencode.json"),
    (Harness::OpenCode, "opencode.jsonc"),
];

/// The harnesses `root` is set up for: those with their own files in it.
pub fn used_harnesses(root: &Path) -> Vec<Harness> {
    let mut harnesses: Vec<Harness> = MARKERS
        .iter()
        .filter(|(_, marker)| root.join(marker).exists())
        .map(|(harness, _)| *harness)
        .collect();
    harnesses.dedup();
    harnesses
}
