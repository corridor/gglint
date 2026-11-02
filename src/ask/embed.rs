//! Layer 1's embedding model: model2vec static embeddings.
//!
//! A static embedding model is a lookup table from token to vector: encoding a text
//! is tokenizing it and averaging its tokens' vectors, with no neural network to run,
//! so a question is encoded in microseconds on any CPU.

use std::path::Path;
use std::sync::{Arc, Mutex};

use model2vec_rs::model::StaticModel;

use super::models::{EMBED_FILES, ensure};
use super::understand::Encode;

/// Questions and prompts are cut to this many tokens.
const MAX_TOKENS: usize = 512;

pub struct Embedder {
    model: StaticModel,
}

impl Embedder {
    /// The process's copy of `potion-code-16M`, loaded by the first call (which downloads
    /// it if needed and allowed) and shared after that. A failed load is tried again by
    /// the next call, as the model may have been downloaded since.
    pub fn shared(download: bool) -> Result<Arc<Embedder>, String> {
        static SHARED: Mutex<Option<Arc<Embedder>>> = Mutex::new(None);
        let mut shared = SHARED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(embedder) = shared.as_ref() {
            return Ok(embedder.clone());
        }
        let embedder = Arc::new(Self::load(download)?);
        *shared = Some(embedder.clone());
        Ok(embedder)
    }

    /// Loads `potion-code-16M`, downloading it first if needed and allowed.
    pub fn load(download: bool) -> Result<Embedder, String> {
        let paths = EMBED_FILES
            .iter()
            .map(|file| ensure(file, download))
            .collect::<Result<Vec<_>, _>>()?;
        let [config, tokenizer, model] = [&paths[0], &paths[1], &paths[2]];
        Self::from_files(tokenizer, model, config)
    }

    pub fn from_files(tokenizer: &Path, model: &Path, config: &Path) -> Result<Embedder, String> {
        let read = |path: &Path| std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()));
        let model = StaticModel::from_bytes(read(tokenizer)?, read(model)?, read(config)?, None)
            .map_err(|error| format!("loading the embedding model: {error}"))?;
        Ok(Embedder { model })
    }
}

impl Encode for Embedder {
    fn encode(&self, texts: &[String]) -> Vec<Vec<f32>> {
        self.model.encode_with_args(texts, Some(MAX_TOKENS), 1024)
    }
}
