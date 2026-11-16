//! What gglint knows about each coding agent, in one place. Supporting another agent starts
//! with a row in `AGENTS`.

use super::Harness;
use super::mcp::{Format, Syntax};

/// One coding agent, as gglint sees it.
pub(super) struct Agent {
    pub harness: Harness,
    /// Its name on the command line, as `--source` takes it.
    pub id: &'static str,
    /// Other names `--source` takes for it.
    pub aliases: &'static [&'static str],
    pub name: &'static str,
    /// Its own directory in a repository. When it's there, the repository is set up for the
    /// agent on purpose, not just mentioned by a file such as `CLAUDE.md`.
    pub directory: &'static str,
    /// The files it reads a repository's MCP servers from, relative to its root. A new
    /// config file is created at the first. Codex only reads `.codex/config.toml` in
    /// projects the user has trusted.
    pub files: &'static [&'static str],
    /// How those files list the servers.
    pub format: Format,
    /// How it refers to an environment variable inside the value of an environment
    /// variable or header.
    pub syntax: Syntax,
    /// How it refers to one inside a command, its arguments or a URL.
    pub command_line_syntax: Syntax,
    /// How a file in the `mcpServers` format writes a server reached over HTTP. Codex and
    /// OpenCode have their own way, set by their format.
    pub remote: Remote,
}

/// How a server reached over HTTP is written in an `mcpServers` file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Remote {
    /// `url` alone.
    Url,
    /// `url` with a `type`: `"sse"` for a URL ending in `/sse`, the older SSE transport, and
    /// `"http"` otherwise. Claude Code starts an entry without a `type` as a command.
    TypedUrl,
}

/// Every agent, in the order they're reported: the order of `Harness`.
pub(super) const AGENTS: &[Agent] = &[
    Agent {
        harness: Harness::ClaudeCode,
        id: "claude-code",
        aliases: &["claude"],
        name: "Claude Code",
        directory: ".claude",
        files: &[".mcp.json"],
        format: Format::McpServers,
        syntax: Syntax::Dollar,
        command_line_syntax: Syntax::Dollar,
        remote: Remote::TypedUrl,
    },
    Agent {
        harness: Harness::Codex,
        id: "codex",
        aliases: &[],
        name: "Codex",
        directory: ".codex",
        files: &[".codex/config.toml"],
        format: Format::Codex,
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::Url,
    },
    Agent {
        harness: Harness::Cursor,
        id: "cursor",
        aliases: &[],
        name: "Cursor",
        directory: ".cursor",
        files: &[".cursor/mcp.json"],
        format: Format::McpServers,
        syntax: Syntax::DollarEnv,
        command_line_syntax: Syntax::DollarEnv,
        remote: Remote::Url,
    },
    Agent {
        harness: Harness::OpenCode,
        id: "opencode",
        aliases: &[],
        name: "OpenCode",
        directory: ".opencode",
        files: &["opencode.json", "opencode.jsonc"],
        format: Format::OpenCode,
        syntax: Syntax::BraceEnv,
        command_line_syntax: Syntax::BraceEnv,
        remote: Remote::Url,
    },
    Agent {
        harness: Harness::Junie,
        id: "junie",
        aliases: &[],
        name: "Junie",
        directory: ".junie",
        files: &[".junie/mcp/mcp.json"],
        format: Format::McpServers,
        // Its docs don't say how a value refers to an environment variable.
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::Url,
    },
    Agent {
        harness: Harness::Trae,
        id: "trae",
        aliases: &["trae-cn"],
        name: "Trae",
        directory: ".trae",
        // Read once "Enable Project MCP" is turned on in Trae's settings.
        files: &[".trae/mcp.json"],
        format: Format::McpServers,
        // Its docs don't say how a value refers to an environment variable.
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::Url,
    },
];

/// What gglint knows about `harness`.
pub(super) fn agent(harness: Harness) -> &'static Agent {
    &AGENTS[harness as usize]
}

#[cfg(test)]
mod tests {
    use super::super::HARNESSES;
    use super::*;

    #[test]
    fn every_agent_has_one_row_in_the_order_they_are_reported() {
        let harnesses: Vec<Harness> = AGENTS.iter().map(|agent| agent.harness).collect();
        assert_eq!(harnesses, HARNESSES);
    }

    #[test]
    fn no_two_agents_share_a_name() {
        let mut names: Vec<&str> = AGENTS
            .iter()
            .flat_map(|agent| std::iter::once(agent.id).chain(agent.aliases.iter().copied()))
            .collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count, "{names:?}");
    }
}
