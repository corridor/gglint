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
    /// agent on purpose, not just mentioned by a file such as `CLAUDE.md`. `None` when the
    /// directory is in many repositories for other reasons (`.github`, `.gemini`), so the
    /// agent counts only once it has an MCP config file.
    pub directory: Option<&'static str>,
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
    /// How a server is turned off without removing it.
    pub off: Off,
    /// The key of an `mcpServers` entry that names the environment variable holding a
    /// bearer token, sent as `Authorization: Bearer <token>`. Codex's is set by its format.
    pub bearer_token_key: Option<&'static str>,
    /// The `type` an `mcpServers` entry started as a command has, when the agent gives it one.
    pub command_type: Option<&'static str>,
    /// The agents whose config files it reads too: every server in them is one it gets.
    pub also_reads: &'static [Harness],
    /// Whether its `mcpServers` file can list servers right at its top instead.
    pub servers_at_top: bool,
    /// A file it reads instead of its own while that one exists, so its own file's servers
    /// aren't its servers then.
    pub read_instead: Option<&'static str>,
    /// Its other files that can hold a secret, relative to the root: instructions, and
    /// settings and hooks that hold commands. `gglint secrets` reads them with `files`.
    pub other_files: &'static [&'static str],
    /// Directories of its rules, commands, agents and hooks, relative to the root: `gglint
    /// secrets` reads the files in them.
    pub rule_directories: &'static [&'static str],
    /// Its files meant for one person's machine and never for git, which its docs say to keep
    /// out of it, and what each one is.
    pub personal_files: &'static [(&'static str, &'static str)],
}

/// The agent whose config file `harness` reads too and gets a server from, among those
/// compared, if there's one. A server missing there and from `harness` is added there.
pub(super) fn shares_file_with(harness: Harness, compared: &[Harness]) -> Option<Harness> {
    agent(harness)
        .also_reads
        .iter()
        .copied()
        .find(|other| compared.contains(other))
}

/// How a server is turned off in an agent's config file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Off {
    /// It can't be: the agent turns servers off in its own settings, not in the file.
    Never,
    /// `"disabled": true`.
    Disabled,
    /// `"enabled": false`, or `enabled = false` in Codex's TOML.
    NotEnabled,
    /// Left out of the file's `mcp.allowed` list, when it has one, even an empty one, or
    /// named in its `mcp.excluded` list: Gemini CLI.
    Lists,
}

/// How a server reached over HTTP is written in an `mcpServers` file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Remote {
    /// `url` alone.
    Url,
    /// `url` with a `type`: `"sse"` for a URL ending in `/sse`, the older SSE transport, and
    /// `"http"` otherwise. Claude Code starts an entry without a `type` as a command.
    TypedUrl,
    /// `url`, with `"transport": "sse"` for a URL ending in `/sse`.
    TransportSse,
    /// `httpUrl`, or `url` for a URL ending in `/sse`: Gemini CLI reaches a `url` over SSE.
    HttpUrl,
    /// `serverUrl`, over streamable HTTP only: a URL ending in `/sse` can't be written.
    ServerUrl,
}

impl Remote {
    /// The keys an entry's URL can be read from, in the order they're looked for.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Remote::Url | Remote::TypedUrl | Remote::TransportSse => &["url"],
            Remote::HttpUrl => &["httpUrl", "url"],
            Remote::ServerUrl => &["serverUrl", "url"],
        }
    }
}

/// Every agent, in the order they're reported: the order of `Harness`.
pub(super) const AGENTS: &[Agent] = &[
    Agent {
        harness: Harness::ClaudeCode,
        id: "claude-code",
        aliases: &["claude"],
        name: "Claude Code",
        directory: Some(".claude"),
        files: &[".mcp.json"],
        format: Format::McpServers,
        syntax: Syntax::Dollar,
        command_line_syntax: Syntax::Dollar,
        remote: Remote::TypedUrl,
        off: Off::Never,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &["CLAUDE.md", ".claude/settings.json"],
        rule_directories: &[".claude/commands", ".claude/agents"],
        personal_files: &[
            (
                ".claude/settings.local.json",
                "Claude Code's personal settings, where it saves the commands you allow, secrets and all",
            ),
            ("CLAUDE.local.md", "Claude Code's personal notes for this project"),
            (".claude/.credentials.json", "Claude Code's login"),
        ],
    },
    Agent {
        harness: Harness::Codex,
        id: "codex",
        aliases: &[],
        name: "Codex",
        directory: Some(".codex"),
        files: &[".codex/config.toml"],
        format: Format::Codex,
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::Url,
        off: Off::NotEnabled,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &["AGENTS.md"],
        rule_directories: &[],
        personal_files: &[(".codex/auth.json", "Codex's login")],
    },
    Agent {
        harness: Harness::Cursor,
        id: "cursor",
        aliases: &[],
        name: "Cursor",
        directory: Some(".cursor"),
        files: &[".cursor/mcp.json"],
        format: Format::McpServers,
        syntax: Syntax::DollarEnv,
        command_line_syntax: Syntax::DollarEnv,
        remote: Remote::Url,
        off: Off::Never,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        // `cli.json` holds the commands Cursor's CLI may run.
        other_files: &[".cursorrules", ".cursor/cli.json"],
        rule_directories: &[".cursor/rules", ".cursor/commands"],
        personal_files: &[],
    },
    Agent {
        harness: Harness::OpenCode,
        id: "opencode",
        aliases: &[],
        name: "OpenCode",
        directory: Some(".opencode"),
        files: &["opencode.json", "opencode.jsonc"],
        format: Format::OpenCode,
        syntax: Syntax::BraceEnv,
        command_line_syntax: Syntax::BraceEnv,
        remote: Remote::Url,
        off: Off::NotEnabled,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &["AGENTS.md"],
        // The singular names are older, and still read.
        rule_directories: &[
            ".opencode/agents",
            ".opencode/agent",
            ".opencode/commands",
            ".opencode/command",
        ],
        personal_files: &[],
    },
    Agent {
        harness: Harness::Antigravity,
        id: "antigravity",
        aliases: &[],
        name: "Antigravity",
        // `.agents` is also where many agents look for skills, so it doesn't count alone.
        directory: None,
        files: &[".agents/mcp_config.json"],
        format: Format::McpServers,
        // Its docs don't say how a value refers to an environment variable.
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::ServerUrl,
        off: Off::Disabled,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &[],
        // `.agent` is the older name, and still read.
        rule_directories: &[".agents/rules", ".agents/workflows", ".agent/rules", ".agent/workflows"],
        personal_files: &[],
    },
    Agent {
        harness: Harness::CopilotCli,
        id: "copilot-cli",
        aliases: &["copilot", "github-copilot-cli"],
        name: "Copilot CLI",
        // Nearly every repository has a `.github` directory, so it doesn't count alone.
        directory: None,
        // Copilot CLI 1.x reads `.mcp.json` instead while it exists: see `read_instead`.
        files: &[".github/mcp.json"],
        format: Format::McpServers,
        // Its docs don't say how a value refers to an environment variable.
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::TypedUrl,
        off: Off::Never,
        bearer_token_key: None,
        command_type: Some("local"),
        also_reads: &[Harness::ClaudeCode],
        servers_at_top: true,
        read_instead: Some(".mcp.json"),
        other_files: &[
            ".github/copilot-instructions.md",
            "AGENTS.md",
            ".github/copilot/settings.json",
        ],
        rule_directories: &[".github/instructions", ".github/hooks"],
        personal_files: &[(
            ".github/copilot/settings.local.json",
            "Copilot CLI's personal settings for this repository",
        )],
    },
    Agent {
        harness: Harness::FactoryDroid,
        id: "droid",
        aliases: &["factory", "factory-droid"],
        name: "Factory Droid",
        directory: Some(".factory"),
        files: &[".factory/mcp.json"],
        format: Format::McpServers,
        // `${NAME}` in `env` and `headers`, and nowhere else.
        syntax: Syntax::Dollar,
        command_line_syntax: Syntax::Literal,
        remote: Remote::TypedUrl,
        off: Off::Disabled,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        // `settings.json` can list the commands Droid may run.
        other_files: &["AGENTS.md", ".factory/settings.json"],
        rule_directories: &[".factory/droids", ".factory/commands"],
        personal_files: &[(
            ".factory/settings.local.json",
            "Factory Droid's personal settings for this project",
        )],
    },
    Agent {
        harness: Harness::GeminiCli,
        id: "gemini-cli",
        aliases: &["gemini"],
        name: "Gemini CLI",
        // `.gemini` also holds Gemini Code Assist's review settings, so it doesn't count alone.
        directory: None,
        files: &[".gemini/settings.json"],
        format: Format::McpServers,
        // `$VAR` or `${VAR}` in any string of settings.json.
        syntax: Syntax::Shell,
        command_line_syntax: Syntax::Shell,
        remote: Remote::HttpUrl,
        off: Off::Lists,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &["GEMINI.md"],
        rule_directories: &[".gemini/commands"],
        personal_files: &[(
            ".gemini/.env",
            "Gemini CLI's environment file, where its docs put your API key",
        )],
    },
    Agent {
        harness: Harness::Junie,
        id: "junie",
        aliases: &[],
        name: "Junie",
        directory: Some(".junie"),
        files: &[".junie/mcp/mcp.json"],
        format: Format::McpServers,
        // Its docs don't say how a value refers to an environment variable.
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::Url,
        off: Off::Never,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &[],
        rule_directories: &[".junie"],
        personal_files: &[],
    },
    Agent {
        harness: Harness::KimiCode,
        id: "kimi-code",
        aliases: &["kimi"],
        name: "Kimi Code",
        directory: Some(".kimi-code"),
        files: &[".kimi-code/mcp.json"],
        format: Format::McpServers,
        // Values are literal: a bearer token is read from the environment with
        // `bearerTokenEnvVar`.
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::TransportSse,
        off: Off::NotEnabled,
        bearer_token_key: Some("bearerTokenEnvVar"),
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &["AGENTS.md"],
        rule_directories: &[],
        personal_files: &[],
    },
    Agent {
        harness: Harness::Kiro,
        id: "kiro",
        aliases: &["kiro-cli"],
        name: "Kiro",
        directory: Some(".kiro"),
        files: &[".kiro/settings/mcp.json"],
        format: Format::McpServers,
        // `${VAR}` in `env` and `headers`; its docs show none in a command line or URL.
        syntax: Syntax::Dollar,
        command_line_syntax: Syntax::Literal,
        remote: Remote::Url,
        off: Off::Disabled,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &[],
        // Hooks run commands.
        rule_directories: &[".kiro/steering", ".kiro/hooks"],
        personal_files: &[],
    },
    Agent {
        harness: Harness::Qoder,
        id: "qoder",
        aliases: &["qoder-cli"],
        name: "Qoder",
        directory: Some(".qoder"),
        // It reads `.mcp.json` too, in `also_reads`.
        files: &[".qoder/settings.json"],
        format: Format::McpServers,
        // Its docs don't say, but Qoder CLI 1.1.89 fills in `$VAR` and `${VAR}` in `env`, `args`
        // and `headers`, as Gemini CLI does.
        syntax: Syntax::Shell,
        command_line_syntax: Syntax::Shell,
        remote: Remote::TypedUrl,
        off: Off::Disabled,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[Harness::ClaudeCode],
        servers_at_top: false,
        read_instead: None,
        other_files: &["AGENTS.md"],
        rule_directories: &[],
        personal_files: &[(
            ".qoder/settings.local.json",
            "Qoder's personal settings, where it saves the commands you allow",
        )],
    },
    Agent {
        harness: Harness::QwenCode,
        id: "qwen-code",
        aliases: &["qwen"],
        name: "Qwen Code",
        directory: Some(".qwen"),
        files: &[".qwen/settings.json"],
        format: Format::McpServers,
        // As in Gemini CLI, which it's built on: `$VAR` or `${VAR}` in any string.
        syntax: Syntax::Shell,
        command_line_syntax: Syntax::Shell,
        remote: Remote::HttpUrl,
        off: Off::Never,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &["QWEN.md"],
        rule_directories: &[".qwen/commands"],
        personal_files: &[(
            ".qwen/.env",
            "Qwen Code's environment file, where its docs put your API key",
        )],
    },
    Agent {
        harness: Harness::Trae,
        id: "trae",
        aliases: &["trae-cn"],
        name: "Trae",
        directory: Some(".trae"),
        // Read once "Enable Project MCP" is turned on in Trae's settings.
        files: &[".trae/mcp.json"],
        format: Format::McpServers,
        // Its docs don't say how a value refers to an environment variable.
        syntax: Syntax::Literal,
        command_line_syntax: Syntax::Literal,
        remote: Remote::Url,
        off: Off::Never,
        bearer_token_key: None,
        command_type: None,
        also_reads: &[],
        servers_at_top: false,
        read_instead: None,
        other_files: &[],
        rule_directories: &[".trae/rules"],
        personal_files: &[],
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
