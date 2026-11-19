//! The MCP config files of the coding agents (harnesses) in a repository.
//!
//! - `agents.rs` says what differs between agents: where each keeps its MCP servers, and how
//!   it writes them.
//! - `mcp.rs` reads the MCP servers each agent is set up to use.
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
pub use fix::{ChangedFile, FixReport, Unfixed, fix_mcp, ignored_by_git};
pub use mcp::{McpConfig, McpServer, Part, Setting, Transport, read_mcp};

/// A program that runs a model as a coding agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Harness {
    ClaudeCode,
    Codex,
    Cursor,
    OpenCode,
    Antigravity,
    CopilotCli,
    FactoryDroid,
    GeminiCli,
    Junie,
    KimiCode,
    Kiro,
    Qoder,
    QwenCode,
    Trae,
}

/// All of them, in the order they're reported.
pub const HARNESSES: [Harness; 14] = [
    Harness::ClaudeCode,
    Harness::Codex,
    Harness::Cursor,
    Harness::OpenCode,
    Harness::Antigravity,
    Harness::CopilotCli,
    Harness::FactoryDroid,
    Harness::GeminiCli,
    Harness::Junie,
    Harness::KimiCode,
    Harness::Kiro,
    Harness::Qoder,
    Harness::QwenCode,
    Harness::Trae,
];

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

    /// Every file it reads a repository's MCP servers from, relative to the root.
    pub fn mcp_files(self) -> &'static [&'static str] {
        agents::agent(self).files
    }

    /// The agent that reads `relative_path`, a repository-relative MCP config file such as
    /// `.cursor/mcp.json`.
    pub fn of_mcp_file(relative_path: &str) -> Option<Harness> {
        HARNESSES
            .iter()
            .copied()
            .find(|harness| harness.mcp_files().contains(&relative_path))
    }

    /// How it refers to the environment variable `name` in the value of one of an MCP
    /// server's environment variables or headers, such as `${env:NAME}` for Cursor. `None`
    /// when it has no way to.
    pub fn env_reference(self, name: &str) -> Option<String> {
        mcp::write_value(&[Part::Env(name.to_string())], mcp::Syntax::of(self))
    }

    /// The key of an MCP server's entry that names the environment variable holding its
    /// Bearer token, for an agent that reads no other variable there: Kimi Code's
    /// `bearerTokenEnvVar`.
    pub fn bearer_token_key(self) -> Option<&'static str> {
        agents::agent(self).bearer_token_key
    }

    /// A file it reads instead of its own while that one exists: Copilot CLI reads
    /// `.mcp.json` instead of `.github/mcp.json`.
    pub fn reads_instead(self) -> Option<&'static str> {
        agents::agent(self).read_instead
    }

    /// Its own directory in a repository. When it's there, the repository is set up for the
    /// agent on purpose, not just mentioned by a file such as `CLAUDE.md`. `None` for an agent
    /// that counts only once it has an MCP config file.
    pub fn directory(self) -> Option<&'static str> {
        agents::agent(self).directory
    }

    /// Its files other than MCP config files that can hold a secret, relative to the root:
    /// instructions such as `CLAUDE.md`, and settings and hooks that hold commands.
    pub fn other_files(self) -> &'static [&'static str] {
        agents::agent(self).other_files
    }

    /// The directories of its rules, commands, agents and hooks, relative to the root.
    pub fn rule_directories(self) -> &'static [&'static str] {
        agents::agent(self).rule_directories
    }

    /// Its files meant for one person's machine and never for git, relative to the root, and
    /// what each one is.
    pub fn personal_files(self) -> &'static [(&'static str, &'static str)] {
        agents::agent(self).personal_files
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
