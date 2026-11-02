//! `gglint ask`: answers questions about the agents and prompts of a repository from
//! its scan, on this machine, with no language model.
//!
//! ```text
//! question ─understand.rs─▶ Query ─query.rs─▶ Answer (sentence + facts)
//!            Layer 1               Layer 0
//! ```
//!
//! - Layer 0 (`graph.rs`, `query.rs`): the agents and prompts of the scan, linked, with
//!   one query per kind of question. Microseconds; no model.
//! - Layer 1 (`understand.rs`, `embed.rs`): which kind of question it is and what it
//!   names, with static embeddings (model2vec: a lookup table, no neural network runs)
//!   and BM25. Falls back to keywords without the model.
//!
//! The embedding model is downloaded on first use (`models.rs`) and loaded once per
//! process, shared by every `Repo`. Layer 0 never uses the network.

pub mod graph;
pub mod query;
pub mod understand;

#[cfg(feature = "embed")]
pub mod embed;
#[cfg(feature = "download")]
pub mod models;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use graph::Graph;
use query::{Fact, run};
use understand::{Encode, Understanding};

use crate::discovery::scan;

#[derive(Clone, Debug)]
pub struct Options {
    /// Use the embedding model for Layer 1.
    pub embed: bool,
    /// Download the embedding model if it isn't cached yet.
    pub download: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            embed: cfg!(feature = "embed"),
            download: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Answer {
    pub text: String,
    /// What the answer rests on, each pointing at a line of code.
    pub facts: Vec<Fact>,
    /// The kind of question, such as `agents_by_model`.
    pub intent: &'static str,
    /// 0 when keywords understood the question, 1 when the embedding model did.
    pub layer: u8,
    pub elapsed: Duration,
}

pub struct Repo {
    graph: Graph,
    understanding: Understanding,
    /// The embedding model couldn't be loaded, and why.
    pub warnings: Vec<String>,
}

impl Repo {
    /// Scans `paths` and gets ready to answer. The embedding model loads while the scan
    /// runs, the first time only.
    pub fn open(paths: &[PathBuf], options: &Options) -> Repo {
        let mut warnings = Vec::new();
        let (results, encoder) = rayon::join(|| scan(paths, false), || encoder(options));
        let encoder = encoder.unwrap_or_else(|warning| {
            warnings.extend(warning);
            None
        });
        let graph = Graph::build(&results);
        let understanding = Understanding::new(&graph, encoder);
        Repo {
            graph,
            understanding,
            warnings,
        }
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Answers in milliseconds.
    pub fn ask(&self, question: &str) -> Answer {
        let start = Instant::now();
        let query = self.understanding.parse(question);
        let reply = run(&self.graph, &query, &|topic| self.understanding.search(topic));
        Answer {
            text: reply.text,
            facts: reply.facts,
            intent: query.intent.as_str(),
            layer: u8::from(self.understanding.uses_embeddings()),
            elapsed: start.elapsed(),
        }
    }
}

/// The embedding model, `Ok(None)` when it's turned off, or why it couldn't be loaded.
fn encoder(options: &Options) -> Result<Option<Arc<dyn Encode>>, Option<String>> {
    if !options.embed {
        return Ok(None);
    }
    #[cfg(feature = "embed")]
    {
        embed::Embedder::shared(options.download)
            .map(|embedder| Some(embedder as Arc<dyn Encode>))
            .map_err(Some)
    }
    #[cfg(not(feature = "embed"))]
    Err(Some("gglint was built without the `embed` feature".to_string()))
}
