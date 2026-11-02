//! Layer 1: turns a question into a `Query`.
//!
//! 1. Names: words of the question are matched against the repository's own names
//!    (agents, models, frameworks, files), exactly or within a typo (Jaro-Winkler).
//!    Matched words become placeholders: "which agents use gpt-4o" → "which agents use
//!    <model>".
//! 2. Intent: the placeholder question is compared with the example questions in
//!    `intents.txt`. With an embedding model, by cosine similarity of static
//!    embeddings (nearest neighbours vote); without one, by word overlap.
//! 3. Topic searches rank prompts with BM25, fused with embedding similarity when a
//!    model is loaded.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bm25::{Document, Language, SearchEngine, SearchEngineBuilder};

use super::graph::Graph;
use super::query::{Intent, Query};

const EXAMPLES: &str = include_str!("intents.txt");

/// A word matches a name when their Jaro-Winkler similarity is at least this.
const NAME_SIMILARITY: f64 = 0.9;
/// Names shorter than this only match exactly, as typos of short words are guesses.
const FUZZY_MIN_CHARS: usize = 4;
/// Neighbours that vote on the intent when embeddings are used.
const NEIGHBOURS: usize = 5;
/// Below this word-overlap score the question isn't like any example, so it's read as a
/// search of the prompts' text.
const MIN_KEYWORD_SCORE: f64 = 0.2;
/// The same, for the best cosine similarity with embeddings.
const MIN_COSINE: f32 = 0.45;
/// Reciprocal rank fusion constant.
const RRF_K: f32 = 60.0;
/// How many prompts a search looks at before fusing.
const SEARCH_DEPTH: usize = 20;

/// Words that are part of how questions are asked, never names or topics.
const STOPWORDS: &[&str] = &[
    "a", "about", "agent", "agents", "all", "an", "and", "any", "are", "as", "at", "be", "by", "can", "code", "codebase",
    "defined", "do", "does", "every", "file", "files", "find", "for", "from", "get", "give", "has", "have", "how",
    "i", "in", "into", "is", "it", "its", "list", "llm", "me", "mention", "mentions", "model", "models", "my", "of",
    "on", "or", "please", "project", "prompt", "prompts", "repo", "repository", "say", "says", "search", "show",
    "system", "tell", "that", "the", "their", "there", "these", "this", "to", "use", "used", "uses", "using", "was",
    "what", "whats", "where", "which", "who", "why", "with", "you",
];

/// Parts of model names that are also everyday words: "show the latest prompts" isn't
/// about `claude-3-5-haiku-latest`.
const GENERIC_MODEL_PARTS: &[&str] = &[
    "latest", "large", "medium", "small", "mini", "preview", "instruct", "chat", "base", "new", "fast", "pro",
];
/// Words after which a question names what agents use: "which agents use gemini",
/// "agents built with crewai".
const USE_WORDS: &[&str] = &[
    "use", "uses", "using", "with", "on", "in", "built", "written", "made", "run", "running", "call", "calls",
];
/// Agent frameworks and model SDKs gglint knows, so a question naming one that this
/// repository doesn't use is answered as a framework, not a model.
const KNOWN_FRAMEWORKS: &[&str] = &[
    "langchain", "langgraph", "deepagents", "pydantic-ai", "openai-agents", "crewai", "google-adk", "autogen",
    "anthropic", "openai", "google-genai",
];

/// Encodes texts as vectors. `ask::embed::Embedder` implements it with model2vec.
pub trait Encode: Send + Sync {
    fn encode(&self, texts: &[String]) -> Vec<Vec<f32>>;
}

#[derive(Clone, Debug, PartialEq)]
enum NameKind {
    Agent(usize),
    Model(String),
    Framework(String),
    File(String),
}

struct Name {
    /// Lowercase.
    text: String,
    kind: NameKind,
}

pub struct Understanding {
    examples: Vec<(Intent, HashSet<String>)>,
    /// Every word of the example questions, except topic searches': a word outside it
    /// is what a question is about.
    vocabulary: HashSet<String>,
    names: Vec<Name>,
    engine: SearchEngine<usize>,
    embeddings: Option<Embeddings>,
}

struct Embeddings {
    encoder: Arc<dyn Encode>,
    examples: Vec<(Intent, Vec<f32>)>,
    prompts: Vec<Vec<f32>>,
}

impl Understanding {
    pub fn new(graph: &Graph, encoder: Option<Arc<dyn Encode>>) -> Understanding {
        let examples: Vec<(Intent, &str)> = EXAMPLES
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .filter_map(|line| {
                let (intent, question) = line.split_once('\t')?;
                Some((Intent::from_name(intent)?, question))
            })
            .collect();

        let documents = graph.prompts.iter().enumerate().map(|(index, prompt)| {
            let owner = prompt.agent.map(|agent| graph.agents[agent].label()).unwrap_or_default();
            Document::new(index, format!("{} {owner} {}", prompt.text, prompt.path))
        });
        let engine = SearchEngineBuilder::<usize>::with_documents(Language::English, documents).build();

        let embeddings = encoder.map(|encoder| {
            let questions: Vec<String> = examples.iter().map(|(_, question)| question.to_string()).collect();
            let prompts: Vec<String> = graph.prompts.iter().map(|prompt| prompt.text.clone()).collect();
            let example_vectors = encoder.encode(&questions);
            let prompt_vectors = if prompts.is_empty() { Vec::new() } else { encoder.encode(&prompts) };
            Embeddings {
                examples: examples.iter().map(|(intent, _)| *intent).zip(example_vectors).collect(),
                prompts: prompt_vectors,
                encoder,
            }
        });

        let examples: Vec<(Intent, HashSet<String>)> = examples
            .into_iter()
            .map(|(intent, question)| (intent, words(question).into_iter().collect()))
            .collect();
        let vocabulary = examples
            .iter()
            .filter(|(intent, _)| *intent != Intent::SearchPrompts)
            .flat_map(|(_, words)| words.iter().cloned())
            .collect();
        Understanding {
            examples,
            vocabulary,
            names: names(graph),
            engine,
            embeddings,
        }
    }

    /// Whether intents and searches use the embedding model.
    pub fn uses_embeddings(&self) -> bool {
        self.embeddings.is_some()
    }

    pub fn parse(&self, question: &str) -> Query {
        let tokens = words(question);
        let mut query = Query::default();
        let mut placeheld = Vec::with_capacity(tokens.len());
        let mut topic = Vec::new();
        for token in &tokens {
            match self.match_name(token) {
                Some(NameKind::Agent(index)) if query.agent.is_none() => {
                    query.agent = Some(index);
                    placeheld.push("<agent>".to_string());
                }
                Some(NameKind::Model(_)) if query.model.is_none() => {
                    query.model = Some(token.clone());
                    placeheld.push("<model>".to_string());
                }
                Some(NameKind::Framework(framework)) if query.framework.is_none() => {
                    query.framework = Some(framework);
                    placeheld.push("<framework>".to_string());
                }
                Some(NameKind::File(path)) if query.file.is_none() => {
                    query.file = Some(path);
                    placeheld.push("<file>".to_string());
                }
                _ => {
                    if !STOPWORDS.contains(&token.as_str()) {
                        topic.push(token.clone());
                    }
                    placeheld.push(token.clone());
                }
            }
        }
        query.topic = topic.join(" ");

        let placeheld = placeheld.join(" ");
        let intent = match &self.embeddings {
            Some(embeddings) => embeddings.intent(&placeheld),
            None => self.keyword_intent(&placeheld),
        };
        let about_something = topic.iter().any(|word| !self.vocabulary.contains(word));
        query.intent = self.adjust(intent, &query, &tokens, about_something);
        if query.agent.is_none() && query.model.is_none() && query.framework.is_none() && query.file.is_none() {
            self.unknown_model_or_framework(&mut query, &tokens);
        }
        if matches!(query.intent, Intent::AgentDetails | Intent::AgentPrompts) && query.agent.is_none() {
            query.unknown_agent = self.unknown_agent(question, &tokens);
        }
        query
    }

    /// Ranks prompts for a topic, best first.
    pub fn search(&self, topic: &str) -> Vec<usize> {
        let lexical: Vec<usize> = self
            .engine
            .search(topic, SEARCH_DEPTH)
            .into_iter()
            .map(|result| result.document.id)
            .collect();
        let Some(embeddings) = &self.embeddings else {
            return lexical;
        };
        if embeddings.prompts.is_empty() {
            return lexical;
        }
        let query = &embeddings.encoder.encode(&[topic.to_string()])[0];
        let mut semantic: Vec<(usize, f32)> = embeddings
            .prompts
            .iter()
            .enumerate()
            .map(|(index, vector)| (index, cosine(query, vector)))
            .collect();
        semantic.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut fused: HashMap<usize, f32> = HashMap::new();
        for (rank, index) in lexical.iter().enumerate() {
            *fused.entry(*index).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
        for (rank, (index, similarity)) in semantic.iter().take(SEARCH_DEPTH).enumerate() {
            // A prompt unrelated to the topic is still somebody's nearest neighbour.
            if *similarity < MIN_COSINE / 2.0 {
                break;
            }
            *fused.entry(*index).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
        let mut fused: Vec<(usize, f32)> = fused.into_iter().collect();
        fused.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        fused.into_iter().map(|(index, _)| index).collect()
    }

    fn match_name(&self, token: &str) -> Option<NameKind> {
        if token.len() < 2 || STOPWORDS.contains(&token) {
            return None;
        }
        // Exact names first, then a model or path containing the word, then typos.
        if let Some(name) = self.names.iter().find(|name| name.text == token) {
            return Some(name.kind.clone());
        }
        let contains = self.names.iter().find(|name| match &name.kind {
            NameKind::Model(_) => self.names_model(&name.text, token),
            NameKind::File(_) => (token.contains('/') || token.contains('.')) && name.text.ends_with(token),
            _ => false,
        });
        if let Some(name) = contains {
            return Some(name.kind.clone());
        }
        if token.len() < FUZZY_MIN_CHARS {
            return None;
        }
        self.names
            .iter()
            .filter(|name| !matches!(name.kind, NameKind::File(_)))
            .map(|name| (strsim::jaro_winkler(token, &name.text), name))
            .filter(|(similarity, _)| *similarity >= NAME_SIMILARITY)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, name)| name.kind.clone())
    }

    /// Whether `token` names `model`: one of its parts ("haiku" in
    /// `claude-3-5-haiku-latest`) that isn't an everyday word, or a longer piece with a
    /// digit or hyphen ("gpt-4o" in `openai:gpt-4o`).
    fn names_model(&self, model: &str, token: &str) -> bool {
        let specific = token.chars().any(|c| c.is_ascii_digit() || c == '-');
        if specific {
            return token.len() >= FUZZY_MIN_CHARS && model.contains(token);
        }
        !GENERIC_MODEL_PARTS.contains(&token)
            && !self.vocabulary.contains(token)
            && model.split([':', '-', '/', '.', '_']).any(|part| part == token)
    }

    /// A question about the agents using something this repository doesn't have
    /// ("which agents use gemini") names it after a word like "use" or "with". It is
    /// answered as a model or framework that isn't found, rather than as a list of
    /// everything.
    fn unknown_model_or_framework(&self, query: &mut Query, tokens: &[String]) {
        if !matches!(
            query.intent,
            Intent::AgentsByModel
                | Intent::AgentsByFramework
                | Intent::ListAgents
                | Intent::ListModels
                | Intent::ListFrameworks
                | Intent::Summary
        ) {
            return;
        }
        let named = tokens.iter().enumerate().filter(|(_, token)| USE_WORDS.contains(&token.as_str())).find_map(
            |(position, _)| {
                tokens.iter().skip(position + 1).take(3).find(|token| {
                    !STOPWORDS.contains(&token.as_str())
                        && !self.vocabulary.contains(*token)
                        && !token.chars().all(|c| c.is_ascii_digit())
                })
            },
        );
        let Some(named) = named else { return };
        if KNOWN_FRAMEWORKS.contains(&named.as_str()) {
            query.framework = Some(named.clone());
            query.intent = Intent::AgentsByFramework;
        } else {
            query.model = Some(named.clone());
            query.intent = Intent::AgentsByModel;
        }
    }

    fn keyword_intent(&self, question: &str) -> Intent {
        let asked: HashSet<String> = words(question).into_iter().collect();
        let mut best = (0.0, Intent::SearchPrompts);
        for (intent, example) in &self.examples {
            let shared = asked.intersection(example).count() as f64;
            let score = shared / asked.union(example).count().max(1) as f64;
            if score > best.0 {
                best = (score, *intent);
            }
        }
        if best.0 < MIN_KEYWORD_SCORE { Intent::SearchPrompts } else { best.1 }
    }

    /// Corrects the intent with the names the question contains: a question naming a
    /// file is about that file, one naming a model is about the agents using it.
    /// `about_something`: the question has words no example question uses, so it asks
    /// about a topic ("prompts about refunds").
    fn adjust(&self, intent: Intent, query: &Query, tokens: &[String], about_something: bool) -> Intent {
        let mentions_prompt = tokens
            .iter()
            .any(|token| matches!(token.as_str(), "prompt" | "prompts" | "instructions" | "instruction" | "persona"));
        if query.file.is_some() {
            return Intent::FileContents;
        }
        if query.agent.is_some() {
            return match intent {
                Intent::AgentDetails | Intent::AgentPrompts => intent,
                _ if mentions_prompt => Intent::AgentPrompts,
                _ => Intent::AgentDetails,
            };
        }
        if query.model.is_some() && !matches!(intent, Intent::AgentsByModel) {
            return Intent::AgentsByModel;
        }
        if query.framework.is_some() && matches!(intent, Intent::ListAgents | Intent::Summary | Intent::SearchPrompts) {
            return Intent::AgentsByFramework;
        }
        if about_something && matches!(intent, Intent::ListPrompts | Intent::Summary) {
            return Intent::SearchPrompts;
        }
        intent
    }

    /// The word a question uses for an agent it names but that doesn't exist: a quoted
    /// string, the word next to "agent", the word after "of", "for" or "named", or else
    /// the last word no example question uses.
    fn unknown_agent(&self, question: &str, tokens: &[String]) -> Option<String> {
        // Not `'`, which is far more often an apostrophe ("what's") than a quote.
        let mut quoted = question.split(['"', '`']);
        if let Some(inner) = quoted.nth(1).filter(|inner| !inner.trim().is_empty()) {
            return Some(inner.trim().to_string());
        }
        let content = |token: &&String| !STOPWORDS.contains(&token.as_str());
        if let Some(position) = tokens.iter().position(|token| token == "agent") {
            let next = tokens.get(position + 1).filter(content);
            let before = position.checked_sub(1).and_then(|before| tokens.get(before)).filter(content);
            if let Some(word) = next.or(before) {
                return Some(word.clone());
            }
        }
        let after = tokens
            .windows(2)
            .rev()
            .find(|pair| matches!(pair[0].as_str(), "of" | "for" | "named" | "called"))
            .map(|pair| &pair[1])
            .filter(content);
        after
            .or_else(|| tokens.iter().rev().filter(content).find(|token| !self.vocabulary.contains(*token)))
            .cloned()
    }
}

impl Embeddings {
    fn intent(&self, question: &str) -> Intent {
        let asked = &self.encoder.encode(&[question.to_string()])[0];
        let mut nearest: Vec<(f32, Intent)> = self
            .examples
            .iter()
            .map(|(intent, vector)| (cosine(asked, vector), *intent))
            .collect();
        nearest.sort_by(|a, b| b.0.total_cmp(&a.0));
        if nearest.first().is_none_or(|(similarity, _)| *similarity < MIN_COSINE) {
            return Intent::SearchPrompts;
        }
        let mut votes: HashMap<Intent, f32> = HashMap::new();
        for (similarity, intent) in nearest.into_iter().take(NEIGHBOURS) {
            *votes.entry(intent).or_default() += similarity;
        }
        votes
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(Intent::SearchPrompts, |(intent, _)| intent)
    }
}

/// The repository's names that a question can refer to.
fn names(graph: &Graph) -> Vec<Name> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |text: &str, kind: NameKind| {
        let text = text.to_lowercase();
        if !text.is_empty() && !STOPWORDS.contains(&text.as_str()) && seen.insert((text.clone(), format!("{kind:?}"))) {
            names.push(Name { text, kind });
        }
    };
    for (index, agent) in graph.agents.iter().enumerate() {
        for label in [&agent.name, &agent.variable].into_iter().flatten() {
            add(label, NameKind::Agent(index));
        }
    }
    for agent in &graph.agents {
        if let Some(model) = &agent.model {
            add(model, NameKind::Model(model.clone()));
        }
        add(agent.framework, NameKind::Framework(agent.framework.to_string()));
        add(&agent.path, NameKind::File(agent.path.clone()));
    }
    for prompt in &graph.prompts {
        if prompt.framework != "messages" {
            add(prompt.framework, NameKind::Framework(prompt.framework.to_string()));
        }
        add(&prompt.path, NameKind::File(prompt.path.clone()));
        for source in &prompt.sources {
            add(source, NameKind::File(source.clone()));
        }
    }
    names
}

/// Lowercase words, keeping the characters that appear in model names and paths.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '<' | '>')))
        .map(|word| word.trim_matches(|c: char| matches!(c, '.' | ':' | '-')).to_string())
        .map(|word| word.strip_suffix("'s").map(str::to_string).unwrap_or(word))
        .filter(|word| !word.is_empty())
        .collect()
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norms = a.iter().map(|x| x * x).sum::<f32>().sqrt() * b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norms == 0.0 { 0.0 } else { dot / norms }
}
