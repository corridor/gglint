//! What gglint knows about each coding agent, in one place. Supporting another agent starts
//! with a row in `AGENTS`.

use super::Harness;

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
}

/// Every agent, in the order they're reported: the order of `Harness`.
pub(super) const AGENTS: &[Agent] = &[
    Agent {
        harness: Harness::ClaudeCode,
        id: "claude-code",
        aliases: &["claude"],
        name: "Claude Code",
        directory: ".claude",
    },
    Agent {
        harness: Harness::Codex,
        id: "codex",
        aliases: &[],
        name: "Codex",
        directory: ".codex",
    },
    Agent {
        harness: Harness::Cursor,
        id: "cursor",
        aliases: &[],
        name: "Cursor",
        directory: ".cursor",
    },
    Agent {
        harness: Harness::OpenCode,
        id: "opencode",
        aliases: &[],
        name: "OpenCode",
        directory: ".opencode",
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
