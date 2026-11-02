//! Layer 0's data: the agents and prompts of one scan, linked to each other.
//!
//! The graph is built once per scan and only read afterwards, so every question is a
//! walk over a few vectors. Agents and prompts refer to each other by index.

use std::path::Path;

use gaoya::minhash::{MinHashIndex, MinHasher, MinHasher32};
use gaoya::text::shingle_tokens;

use crate::discovery::FileResult;

/// Two prompts whose word 3-shingles overlap at least this much are near-duplicates.
const DUPLICATE_JACCARD: f64 = 0.8;
/// MinHash bands × rows: 20 × 5 = 100 hashes, which finds pairs above
/// `DUPLICATE_JACCARD` with near certainty.
const BANDS: usize = 20;
const BAND_WIDTH: usize = 5;

#[derive(Debug)]
pub struct AgentNode {
    /// The file, relative to the scanned directory, with `/` separators.
    pub path: String,
    pub line: usize,
    pub framework: &'static str,
    /// `agent`, `agent_class` or `agent_factory`.
    pub kind: &'static str,
    pub name: Option<String>,
    pub model: Option<String>,
    pub variable: Option<String>,
    /// Indices into `Graph::prompts` of the prompts passed to the call that creates it.
    pub prompts: Vec<usize>,
}

impl AgentNode {
    /// What the agent is called: its name, else the variable it is assigned to, else
    /// where it is defined.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.variable.clone())
            .unwrap_or_else(|| self.location())
    }

    pub fn location(&self) -> String {
        format!("{}:{}", self.path, self.line)
    }
}

#[derive(Debug)]
pub struct PromptNode {
    pub path: String,
    pub line: usize,
    pub framework: &'static str,
    /// `system`, `user` or `template`.
    pub role: &'static str,
    pub argument: &'static str,
    pub text: String,
    pub dynamic_parts: usize,
    /// Index into `Graph::agents` of the agent the prompt configures.
    pub agent: Option<usize>,
    /// Other files the text came from, relative to the scanned directory.
    pub sources: Vec<String>,
}

impl PromptNode {
    pub fn location(&self) -> String {
        format!("{}:{}", self.path, self.line)
    }
}

#[derive(Debug, Default)]
pub struct Graph {
    pub agents: Vec<AgentNode>,
    pub prompts: Vec<PromptNode>,
    /// Groups of near-duplicate prompts, as indices into `prompts`, in file order.
    pub duplicate_groups: Vec<Vec<usize>>,
    /// How many Python files were scanned.
    pub files: usize,
}

impl Graph {
    pub fn build(results: &[FileResult]) -> Graph {
        let mut graph = Graph {
            files: results.len(),
            ..Graph::default()
        };
        for result in results {
            let path = slash_path(&result.relative_path);
            let root = result.path.ancestors().nth(result.relative_path.components().count());
            let first_agent = graph.agents.len();
            for (location, agent) in &result.agents {
                graph.agents.push(AgentNode {
                    path: path.clone(),
                    line: location.line,
                    framework: agent.framework,
                    kind: agent.kind.as_str(),
                    name: agent.name.clone(),
                    model: agent.model.clone(),
                    variable: agent.variable.clone(),
                    prompts: Vec::new(),
                });
            }
            for (location, prompt) in &result.prompts {
                // A prompt names its agent by name or variable, and is passed to the
                // call that creates it, so the agent is in the same file.
                let agent = prompt.agent.as_deref().and_then(|key| {
                    (first_agent..graph.agents.len()).find(|&index| {
                        let agent = &graph.agents[index];
                        agent.name.as_deref() == Some(key) || agent.variable.as_deref() == Some(key)
                    })
                });
                let index = graph.prompts.len();
                if let Some(agent) = agent {
                    graph.agents[agent].prompts.push(index);
                }
                graph.prompts.push(PromptNode {
                    path: path.clone(),
                    line: location.line,
                    framework: prompt.framework,
                    role: prompt.role.as_str(),
                    argument: prompt.argument,
                    text: prompt.text.clone(),
                    dynamic_parts: prompt.dynamic_parts,
                    agent,
                    sources: prompt
                        .sources
                        .iter()
                        .map(|source| {
                            slash_path(root.and_then(|root| source.strip_prefix(root).ok()).unwrap_or(source))
                        })
                        .collect(),
                });
            }
        }
        graph.duplicate_groups = duplicate_groups(&graph.prompts);
        graph
    }
}

/// Finds near-duplicate prompts with MinHash locality-sensitive hashing.
fn duplicate_groups(prompts: &[PromptNode]) -> Vec<Vec<usize>> {
    let hasher = MinHasher32::new(BANDS * BAND_WIDTH);
    let mut index = MinHashIndex::new(BANDS, BAND_WIDTH, DUPLICATE_JACCARD);
    let mut signatures = Vec::with_capacity(prompts.len());
    for (id, prompt) in prompts.iter().enumerate() {
        let lowered = prompt.text.to_lowercase();
        let words: Vec<&str> = lowered.split_whitespace().collect();
        // Too short to say two texts were copied rather than written alike.
        if words.len() < 6 {
            signatures.push(None);
            continue;
        }
        let signature = hasher.create_signature(shingle_tokens(&words, 3));
        index.insert(id, signature.clone());
        signatures.push(Some(signature));
    }

    let mut grouped = vec![false; prompts.len()];
    let mut groups = Vec::new();
    for (id, signature) in signatures.iter().enumerate() {
        let Some(signature) = signature else { continue };
        if grouped[id] {
            continue;
        }
        let mut group: Vec<usize> = index.query_owned(signature).into_iter().collect();
        if group.len() < 2 {
            continue;
        }
        group.sort_unstable();
        for &member in &group {
            grouped[member] = true;
        }
        groups.push(group);
    }
    groups
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
