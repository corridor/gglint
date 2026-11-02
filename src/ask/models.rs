//! The model files Layer 1 uses: where they are cached and how they're fetched.
//!
//! Each file is pinned to a Hugging Face commit and its SHA-256, so what runs is
//! exactly what was reviewed. Files are downloaded on first use to the cache directory
//! and checked before they're used; a file that doesn't match is deleted.

use std::path::{Path, PathBuf};

/// One file of a model on Hugging Face.
#[derive(Clone, Copy, Debug)]
pub struct ModelFile {
    pub repo: &'static str,
    /// The commit the file is pinned to.
    pub revision: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

/// Layer 1: `minishlab/potion-code-16M`, static embeddings distilled for code (MIT).
pub const EMBED_FILES: [ModelFile; 3] = [
    ModelFile {
        repo: "minishlab/potion-code-16M",
        revision: "1b0ff71095656b23306542bbad34a09109673720",
        file: "config.json",
        sha256: "edf07552b5d768d556ded176d19f3a34f25360548de3a246d226ce8e28647914",
        bytes: 97,
    },
    ModelFile {
        repo: "minishlab/potion-code-16M",
        revision: "1b0ff71095656b23306542bbad34a09109673720",
        file: "tokenizer.json",
        sha256: "8e84217af15e70e8127c855435fc3d8a4cd91d7bbe686f72e75f188118ec78ae",
        bytes: 1_041_917,
    },
    ModelFile {
        repo: "minishlab/potion-code-16M",
        revision: "1b0ff71095656b23306542bbad34a09109673720",
        file: "model.safetensors",
        sha256: "ca6159081a6e96cebe4ad878e5e8437bfccc761e8db16223370149cd2faa6c0b",
        bytes: 64_299_272,
    },
];

/// `$GGLINT_MODELS_DIR`, else `$XDG_CACHE_HOME/gglint/models`, else
/// `~/.cache/gglint/models`.
pub fn models_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GGLINT_MODELS_DIR") {
        return PathBuf::from(dir);
    }
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    cache.join("gglint").join("models")
}

/// Where `file` is cached: `<models_dir>/<owner>__<repo>/<file>`.
pub fn local_path(file: &ModelFile) -> PathBuf {
    models_dir().join(file.repo.replace('/', "__")).join(file.file)
}

/// The path of `file`, downloading it first when it's missing and `download` is set.
///
/// A cached file is trusted once its size matches; downloaded files are hashed while
/// they're written and only kept when the hash matches.
pub fn ensure(file: &ModelFile, download: bool) -> Result<PathBuf, String> {
    let path = local_path(file);
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() == file.bytes) {
        return Ok(path);
    }
    if !download {
        return Err(format!(
            "{} is not downloaded; run `gglint models pull` or set GGLINT_MODELS_DIR",
            path.display()
        ));
    }
    fetch(file, &path)?;
    Ok(path)
}

/// Whether every file is already cached, so loading won't download anything.
pub fn cached(files: &[ModelFile]) -> bool {
    files
        .iter()
        .all(|file| std::fs::metadata(local_path(file)).is_ok_and(|metadata| metadata.len() == file.bytes))
}

/// Checks a cached file's hash; used by `gglint models pull`, which re-verifies.
pub fn verify(file: &ModelFile) -> Result<(), String> {
    let path = local_path(file);
    let mut reader = std::fs::File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let hash = sha256(&mut reader).map_err(|error| format!("{}: {error}", path.display()))?;
    if hash != file.sha256 {
        return Err(format!("{}: SHA-256 is {hash}, expected {}", path.display(), file.sha256));
    }
    Ok(())
}

fn fetch(file: &ModelFile, path: &Path) -> Result<(), String> {
    use std::io::Write;
    use std::time::Duration;

    let url = format!("https://huggingface.co/{}/resolve/{}/{}", file.repo, file.revision, file.file);
    eprintln!("gglint: downloading {} ({:.1} MB)", url, file.bytes as f64 / 1e6);
    let parent = path.parent().expect("a cached file has a directory");
    std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    // A name of its own, so two processes downloading at once never write one file.
    let partial = Partial(parent.join(format!(
        ".{}.{}.{}.part",
        file.file,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos()),
    )));

    // A network that stops answering fails the download instead of hanging it: the
    // body may take 60 s plus the time to receive it at 256 KB/s.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_resolve(Some(Duration::from_secs(15)))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_send_request(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(60 + file.bytes / (256 * 1024))))
        .build()
        .into();
    let mut response = agent.get(&url).call().map_err(|error| format!("{url}: {error}"))?;
    let mut reader = response.body_mut().with_config().limit(file.bytes + 1).reader();
    let mut writer = std::fs::File::create(&partial.0).map_err(|error| format!("{}: {error}", partial.0.display()))?;
    let hash = {
        let mut tee = Tee {
            reader: &mut reader,
            writer: &mut writer,
        };
        sha256(&mut tee).map_err(|error| format!("{url}: {error}"))?
    };
    writer.flush().map_err(|error| format!("{}: {error}", partial.0.display()))?;
    if hash != file.sha256 {
        return Err(format!("{url}: SHA-256 is {hash}, expected {}", file.sha256));
    }
    std::fs::rename(&partial.0, path).map_err(|error| format!("{}: {error}", path.display()))
}

/// A download in progress, deleted unless it's renamed into place.
struct Partial(PathBuf);

impl Drop for Partial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Copies what it reads to `writer`, so a download is hashed and saved in one pass.
struct Tee<'a, R, W> {
    reader: &'a mut R,
    writer: &'a mut W,
}

impl<R: std::io::Read, W: std::io::Write> std::io::Read for Tee<'_, R, W> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.reader.read(buffer)?;
        self.writer.write_all(&buffer[..read])?;
        Ok(read)
    }
}

/// The lowercase hex SHA-256 of everything `reader` yields.
pub fn sha256(reader: &mut impl std::io::Read) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}
