//! The cross-file step: fills in prompt text that lives outside the file the prompt
//! is passed in.
//!
//! Files are checked one at a time, in parallel. A prompt whose text comes from
//! somewhere else keeps a placeholder [`Part`] for it:
//!
//! ```python
//! from . import prompt                      # agent.py
//! root = LlmAgent(instruction=prompt.ROOT)  # → Part::Import(["app", "prompt", "ROOT"])
//!
//! ROOT = """You are a financial advisor."""  # prompt.py
//! ```
//!
//! Once every file has been checked, [`resolve`] looks the placeholders up. It only
//! opens the files that placeholders point to, so a project that keeps its prompts
//! in `prompt.py` pays for parsing `prompt.py`, not for a second scan. Imports are
//! followed through modules that import a name and pass it on, such as a package's
//! `__init__.py`, up to [`MAX_IMPORT_DEPTH`] modules deep.
//!
//! Files read at runtime (`Path("system.md").read_text()`) and crewAI YAML configs
//! (`config=self.agents_config["writer"]`) are read here too, each file once.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use ruff_python_ast::PySourceType;
use ruff_python_parser::parse_unchecked_source;
use ruff_text_size::Ranged;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::checker::{Checker, check_module};
use crate::discovery::{FileResult, module_path};
use crate::prompts::{Anchor, PathStep, Piece, SourcePath};
use crate::semantic::{BindingKind, QualifiedName};

/// How many modules deep an imported prompt is followed: `agent.py` imports it from
/// `prompts/__init__.py`, which imports it from `prompts/root.py`, and so on.
const MAX_IMPORT_DEPTH: usize = 4;

/// How many functions deep `build()` is followed when it returns `other()`.
const MAX_CALL_DEPTH: usize = 4;

/// Files read for their text are skipped above this size: a prompt file is a few
/// kilobytes, and anything much bigger is data.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// One part of a prompt's text, owned so it outlives the file's syntax tree.
#[derive(Clone, Debug)]
pub enum Part {
    Text(String),
    /// Only known at runtime. Holds the expression as written.
    Dynamic(String),
    /// A name in another module, not looked up yet. `origin` is the file it was
    /// written in, used to pick between modules of the same name.
    Import {
        name: Vec<String>,
        called: bool,
        source: String,
        origin: PathBuf,
    },
    /// A file read at runtime. `paths` are the places it may be, in order.
    File { paths: Vec<PathBuf>, source: String },
    /// A field of an entry in a crewAI YAML config.
    Config {
        path: PathBuf,
        key: String,
        field: &'static str,
        source: String,
    },
}

/// What a file's pieces need to be turned into parts.
pub struct FileContext<'s> {
    pub path: &'s Path,
    /// The directory the file was found under, which relative paths are tried against.
    pub root: &'s Path,
    pub module_path: &'s [&'s str],
    /// Whether the file is a package's `__init__.py`, which changes what `.` means in
    /// a relative import.
    pub is_package: bool,
    pub source: &'s str,
}

/// Turns pieces into parts, filling in what functions in the same module return.
pub fn to_parts(checker: &Checker<'_>, pieces: &[Piece<'_>], context: &FileContext<'_>) -> Vec<Part> {
    let mut parts = Vec::new();
    push_parts(checker, pieces, context, 0, &mut parts);
    parts
}

fn push_parts(checker: &Checker<'_>, pieces: &[Piece<'_>], context: &FileContext<'_>, depth: usize, out: &mut Vec<Part>) {
    let source = |expr: &dyn Ranged| context.source[expr.range()].to_string();
    for piece in pieces {
        match piece {
            Piece::Text(text, _) => match out.last_mut() {
                Some(Part::Text(previous)) => previous.push_str(text),
                _ => out.push(Part::Text((*text).to_string())),
            },
            Piece::Dynamic(expr) => out.push(Part::Dynamic(source(*expr))),
            Piece::Returned { function, expr } => match checker.function_return(*function) {
                Some(returned) if depth < MAX_CALL_DEPTH => {
                    push_parts(checker, returned, context, depth + 1, out);
                }
                _ => out.push(Part::Dynamic(source(*expr))),
            },
            Piece::Import { name, called, expr } => match absolute_name(name, context) {
                Some(name) => out.push(Part::Import {
                    name,
                    called: *called,
                    source: source(*expr),
                    origin: context.path.to_path_buf(),
                }),
                None => out.push(Part::Dynamic(source(*expr))),
            },
            Piece::File { path, expr } => out.push(Part::File {
                paths: candidate_paths(path, context),
                source: source(*expr),
            }),
            Piece::Config { file, key, field, expr } => out.push(Part::Config {
                path: context.path.parent().unwrap_or(Path::new("")).join(file),
                key: (*key).to_string(),
                field,
                source: source(*expr),
            }),
        }
    }
}

/// Resolves the dots of a relative import: in `app/agent.py`, `.prompt.ROOT` is
/// `app.prompt.ROOT`. `None` if the dots go above the scanned directory.
fn absolute_name(name: &QualifiedName<'_>, context: &FileContext<'_>) -> Option<Vec<String>> {
    let segments = name.segments();
    let level = segments.iter().take_while(|segment| **segment == ".").count();
    if level == 0 {
        return Some(segments.iter().map(|segment| (*segment).to_string()).collect());
    }
    // `.` is the package the module is in. In a package's `__init__.py`, that's the
    // package itself.
    let package_len = if context.is_package {
        context.module_path.len()
    } else {
        context.module_path.len().checked_sub(1)?
    };
    let package_len = package_len.checked_sub(level - 1)?;
    Some(
        context.module_path[..package_len]
            .iter()
            .chain(&segments[level..])
            .map(|segment| (*segment).to_string())
            .collect(),
    )
}

/// Where a file read at runtime may be. A path built from `__file__` has one place.
/// A relative path depends on the working directory, so it's tried against the
/// scanned directory, then against the module's own directory.
fn candidate_paths(path: &SourcePath<'_>, context: &FileContext<'_>) -> Vec<PathBuf> {
    let bases = match path.anchor {
        Anchor::File => vec![context.path.to_path_buf()],
        Anchor::WorkingDirectory => {
            let mut bases = vec![context.root.to_path_buf()];
            if let Some(directory) = context.path.parent() {
                bases.push(directory.to_path_buf());
            }
            bases
        }
    };
    bases
        .into_iter()
        .map(|mut base| {
            for step in &path.steps {
                match step {
                    PathStep::Parent => {
                        base.pop();
                    }
                    PathStep::Join(part) => base.push(part),
                }
            }
            base
        })
        .collect()
}

/// The prompt's text, with each part that isn't known written as `{expression}`, and
/// how many such parts there are.
pub fn render(parts: &[Part]) -> (String, usize) {
    let mut text = String::new();
    let mut dynamic_parts = 0;
    for part in parts {
        match part {
            Part::Text(value) => text.push_str(value),
            Part::Dynamic(source)
            | Part::Import { source, .. }
            | Part::File { source, .. }
            | Part::Config { source, .. } => {
                dynamic_parts += 1;
                text.push('{');
                text.push_str(source);
                text.push('}');
            }
        }
    }
    (text, dynamic_parts)
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Every module under the scanned directories, by the last segment of its name.
struct ModuleIndex {
    modules: Vec<Module>,
    by_last_segment: FxHashMap<String, Vec<usize>>,
}

struct Module {
    segments: Vec<String>,
    root: PathBuf,
    path: PathBuf,
}

impl ModuleIndex {
    fn new(files: &[(&Path, PathBuf)]) -> Self {
        let mut modules = Vec::with_capacity(files.len());
        let mut by_last_segment: FxHashMap<String, Vec<usize>> = FxHashMap::default();
        for (root, path) in files {
            let segments = module_path(root, path);
            if let Some(last) = segments.last() {
                by_last_segment.entry(last.clone()).or_default().push(modules.len());
            }
            modules.push(Module {
                segments,
                root: root.to_path_buf(),
                path: path.clone(),
            });
        }
        Self {
            modules,
            by_last_segment,
        }
    }

    /// The module that `name` is an attribute of, and the attribute: `app.prompt.ROOT`
    /// → (`app/prompt.py`, `ROOT`).
    ///
    /// Module names depend on which directory was scanned. In a repository of several
    /// projects, `app/agent.py` may be the module `samples.advisor.app.agent` while
    /// its own code imports `app.prompt`. So a module matches if its name *ends* with
    /// the one imported, and if several do, the one closest to the importing file wins.
    fn lookup(&self, name: &[String], origin: &Path) -> Option<(usize, String)> {
        let (attribute, module) = name.split_last()?;
        let candidates = self.by_last_segment.get(module.last()?)?;
        let index = candidates
            .iter()
            .copied()
            .filter(|&index| self.modules[index].segments.ends_with(module))
            .max_by_key(|&index| common_prefix(&self.modules[index].path, origin))?;
        Some((index, attribute.clone()))
    }
}

fn common_prefix(a: &Path, b: &Path) -> usize {
    a.components().zip(b.components()).take_while(|(a, b)| a == b).count()
}

/// The text of a module-level name, by module, name, and whether it's called.
type Exports = FxHashMap<(usize, String, bool), Option<Vec<Part>>>;

/// Fills in imported names, files and crewAI configs in every prompt of `results`.
/// `files` is every Python file that was scanned, with the directory it was found
/// under.
pub fn resolve(results: &mut [FileResult], files: &[(&Path, PathBuf)]) {
    let unresolved = |part: &Part| matches!(part, Part::Import { .. } | Part::File { .. } | Part::Config { .. });
    if !results
        .iter()
        .flat_map(|result| &result.prompts)
        .any(|(_, prompt)| prompt.parts.iter().any(unresolved))
    {
        return;
    }

    resolve_imports(results, files);
    resolve_files(results);

    for (_, prompt) in results.iter_mut().flat_map(|result| result.prompts.iter_mut()) {
        let (text, dynamic_parts) = render(&prompt.parts);
        prompt.text = text;
        prompt.dynamic_parts = dynamic_parts;
    }
}

fn resolve_imports(results: &mut [FileResult], files: &[(&Path, PathBuf)]) {
    let index = ModuleIndex::new(files);
    let mut exports: Exports = FxHashMap::default();

    for _ in 0..MAX_IMPORT_DEPTH {
        // Which names of which modules are needed and not looked up yet.
        let mut wanted: FxHashMap<usize, FxHashSet<(String, bool)>> = FxHashMap::default();
        for (_, prompt) in results.iter().flat_map(|result| &result.prompts) {
            for part in &prompt.parts {
                let Part::Import { name, called, origin, .. } = part else {
                    continue;
                };
                if let Some((module, attribute)) = index.lookup(name, origin) {
                    if !exports.contains_key(&(module, attribute.clone(), *called)) {
                        wanted.entry(module).or_default().insert((attribute, *called));
                    }
                }
            }
        }
        if wanted.is_empty() {
            break;
        }

        let found: Vec<_> = wanted
            .into_par_iter()
            .flat_map_iter(|(module, names)| module_exports(&index, module, names))
            .collect();
        exports.extend(found);

        // Replace each import with what it refers to, which may be another import.
        for (_, prompt) in results.iter_mut().flat_map(|result| result.prompts.iter_mut()) {
            if !prompt.parts.iter().any(|part| matches!(part, Part::Import { .. })) {
                continue;
            }
            let mut parts = Vec::with_capacity(prompt.parts.len());
            for part in std::mem::take(&mut prompt.parts) {
                let Part::Import {
                    name,
                    called,
                    source,
                    origin,
                } = part
                else {
                    parts.push(part);
                    continue;
                };
                let found = index.lookup(&name, &origin).and_then(|(module, attribute)| {
                    let parts = exports.get(&(module, attribute, called))?.as_ref()?;
                    Some((module, parts))
                });
                match found {
                    Some((module, found)) => {
                        // A module that only passes the name on isn't where the text is.
                        if found.iter().any(|part| !matches!(part, Part::Import { .. })) {
                            add_source(&mut prompt.sources, &index.modules[module].path);
                        }
                        // If a passed-on name can't be found either, show what's
                        // written here, not the name in the module that passed it on.
                        parts.extend(found.iter().map(|part| match part {
                            Part::Import { name, called, origin, .. } => Part::Import {
                                name: name.clone(),
                                called: *called,
                                source: source.clone(),
                                origin: origin.clone(),
                            },
                            part => part.clone(),
                        }));
                    }
                    None => parts.push(Part::Dynamic(source)),
                }
            }
            prompt.parts = parts;
        }
    }

    // Imports still left are deeper than the limit.
    for (_, prompt) in results.iter_mut().flat_map(|result| result.prompts.iter_mut()) {
        for part in &mut prompt.parts {
            if let Part::Import { source, .. } = part {
                *part = Part::Dynamic(std::mem::take(source));
            }
        }
    }
}

/// Parses `module` and returns the text of each of `names`.
fn module_exports(
    index: &ModuleIndex,
    module: usize,
    names: FxHashSet<(String, bool)>,
) -> Vec<((usize, String, bool), Option<Vec<Part>>)> {
    let entry = &index.modules[module];
    let not_found = |names: FxHashSet<(String, bool)>| {
        names
            .into_iter()
            .map(|(name, called)| ((module, name, called), None))
            .collect()
    };
    let Ok(source) = std::fs::read_to_string(&entry.path) else {
        return not_found(names);
    };
    let parsed = parse_unchecked_source(&source, PySourceType::Python);
    let module_path: Vec<&str> = entry.segments.iter().map(String::as_str).collect();
    let checker = check_module(parsed.syntax(), module_path.clone());
    let context = FileContext {
        path: &entry.path,
        root: &entry.root,
        module_path: &module_path,
        is_package: entry.path.file_name().is_some_and(|name| name == "__init__.py"),
        source: &source,
    };

    names
        .into_iter()
        .map(|(name, called)| {
            let parts = export(&checker, &context, &name, called);
            ((module, name, called), parts)
        })
        .collect()
}

/// The text of the module-level name `name`, or what it returns if it's `called`.
fn export(checker: &Checker<'_>, context: &FileContext<'_>, name: &str, called: bool) -> Option<Vec<Part>> {
    let binding = checker.semantic.global_scope().get(name)?;
    match &checker.semantic.binding(binding).kind {
        // A function passed as a prompt, `instruction=build`, also stands for what it
        // returns: frameworks call it to get the prompt.
        BindingKind::FunctionDefinition => Some(to_parts(checker, checker.function_return(binding)?, context)),
        // Re-exported: `from .root import ROOT` in `prompts/__init__.py`.
        BindingKind::Import(qualified_name) | BindingKind::FromImport(qualified_name) => {
            let name = absolute_name(qualified_name, context)?;
            Some(vec![Part::Import {
                name,
                called,
                source: name_source(qualified_name),
                origin: context.path.to_path_buf(),
            }])
        }
        _ if !called => Some(to_parts(checker, checker.binding_text(binding)?, context)),
        _ => None,
    }
}

fn name_source(name: &QualifiedName<'_>) -> String {
    name.to_string()
}

fn add_source(sources: &mut Vec<PathBuf>, path: &Path) {
    if !sources.iter().any(|source| source == path) {
        sources.push(path.to_path_buf());
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// Reads each file that prompts read at runtime, and each crewAI config, once.
fn resolve_files(results: &mut [FileResult]) {
    let mut text_files: FxHashSet<PathBuf> = FxHashSet::default();
    let mut configs: FxHashSet<PathBuf> = FxHashSet::default();
    for (_, prompt) in results.iter().flat_map(|result| &result.prompts) {
        for part in &prompt.parts {
            match part {
                Part::File { paths, .. } => text_files.extend(paths.iter().cloned()),
                Part::Config { path, .. } => {
                    configs.insert(path.clone());
                }
                _ => {}
            }
        }
    }
    if text_files.is_empty() && configs.is_empty() {
        return;
    }

    let texts: FxHashMap<PathBuf, String> = text_files
        .into_par_iter()
        .filter_map(|path| {
            let text = read_small_file(&path)?;
            Some((path, text))
        })
        .collect();
    let configs: FxHashMap<PathBuf, CrewaiConfig> = configs
        .into_par_iter()
        .filter_map(|path| {
            let config = parse_crewai_config(&read_small_file(&path)?);
            Some((path, config))
        })
        .collect();

    for result in results.iter_mut() {
        result.prompts.retain_mut(|(_, prompt)| {
            let mut keep = true;
            let mut sources = std::mem::take(&mut prompt.sources);
            for part in &mut prompt.parts {
                match part {
                    Part::File { paths, source } => {
                        let found = paths.iter().find_map(|path| Some((path, texts.get(path)?)));
                        *part = match found {
                            Some((path, text)) => {
                                add_source(&mut sources, path);
                                Part::Text(text.clone())
                            }
                            None => Part::Dynamic(std::mem::take(source)),
                        };
                    }
                    Part::Config {
                        path,
                        key,
                        field,
                        source,
                    } => {
                        let Some(config) = configs.get(path) else {
                            *part = Part::Dynamic(std::mem::take(source));
                            continue;
                        };
                        match config.get(key.as_str()).and_then(|entry| entry.get(*field)) {
                            Some(text) => {
                                add_source(&mut sources, path);
                                *part = Part::Text(text.clone());
                            }
                            // The config is there but doesn't set this field, so no
                            // prompt is passed for it.
                            None => keep = false,
                        }
                    }
                    _ => {}
                }
            }
            prompt.sources = sources;
            keep
        });
    }
}

fn read_small_file(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

// ---------------------------------------------------------------------------
// crewAI YAML configs
// ---------------------------------------------------------------------------

/// Entry name → field → text.
type CrewaiConfig = FxHashMap<String, FxHashMap<String, String>>;

/// Reads the part of YAML that crewAI configs use: top-level names, each a mapping of
/// fields to strings, written plain, quoted, or as a block (`|` or `>`):
///
/// ```yaml
/// researcher:
///   role: >
///     {topic} Senior Data Researcher
///   goal: Uncover developments in {topic}
/// ```
///
/// Anything else, such as lists or deeper nesting, is skipped. A full YAML parser would
/// be a large dependency for this one format.
fn parse_crewai_config(text: &str) -> CrewaiConfig {
    let lines: Vec<&str> = text.lines().collect();
    let mut config = CrewaiConfig::default();
    let mut entry: Option<String> = None;
    let mut field_indent: Option<usize> = None;

    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let content = line.trim();
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        let line_indent = indent(line);
        if line_indent == 0 {
            entry = content
                .strip_suffix(':')
                .map(|name| unquote(name.trim()).to_string());
            field_indent = None;
            continue;
        }
        let Some(entry) = &entry else {
            continue;
        };
        let field_indent = *field_indent.get_or_insert(line_indent);
        if line_indent != field_indent {
            continue;
        }
        let Some((field, rest)) = content.split_once(':') else {
            continue;
        };
        let rest = rest.trim();

        // The lines after the field that are indented further belong to its value.
        let start = index;
        while index < lines.len() && (lines[index].trim().is_empty() || indent(lines[index]) > field_indent) {
            index += 1;
        }
        let mut following = &lines[start..index];
        while let [rest @ .., last] = following {
            if !last.trim().is_empty() {
                break;
            }
            following = rest;
        }

        let value = if let Some(style) = rest.chars().next().filter(|c| matches!(c, '|' | '>')) {
            block_scalar(following, style == '>')
        } else if rest.starts_with('"') || rest.starts_with('\'') {
            unquote(rest).to_string()
        } else {
            // A plain value may go on over more lines, which fold into one.
            let mut value = strip_comment(rest).to_string();
            for line in following {
                let line = line.trim();
                if !line.is_empty() {
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(line);
                }
            }
            value
        };
        config
            .entry(entry.clone())
            .or_default()
            .insert(field.trim().to_string(), value);
    }
    config
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The lines of a `|` block keep their line breaks. A `>` block folds lines into
/// spaces, and a blank line becomes a line break.
fn block_scalar(lines: &[&str], folded: bool) -> String {
    let base = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| indent(line))
        .min()
        .unwrap_or(0);
    let mut value = String::new();
    for (index, line) in lines.iter().enumerate() {
        let line = line.get(base..).unwrap_or("").trim_end();
        if index > 0 {
            let separator = if !folded || line.is_empty() || value.ends_with('\n') {
                "\n"
            } else {
                " "
            };
            if !(folded && line.is_empty() && value.ends_with('\n')) {
                value.push_str(separator);
            }
        }
        if !(folded && line.is_empty()) {
            value.push_str(line);
        }
    }
    value
}

fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && (bytes[0] == b'"' || bytes[0] == b'\'') && bytes[bytes.len() - 1] == bytes[0] {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

/// `value  # comment` → `value`. A `#` only starts a comment after whitespace.
fn strip_comment(value: &str) -> &str {
    match value.find(" #") {
        Some(index) => value[..index].trim_end(),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crewai_configs() {
        let config = parse_crewai_config(
            "# agents\n\
             researcher:\n  \
               role: >\n    \
                 {topic} Senior\n    \
                 Data Researcher\n\n  \
               goal: Uncover developments  # inline comment\n    \
                 in {topic}\n  \
               backstory: |\n    \
                 Line one.\n    \
                 Line two.\n\
             writer:\n  \
               role: \"Writer: docs\"\n",
        );
        let researcher = &config["researcher"];
        assert_eq!(researcher["role"], "{topic} Senior Data Researcher");
        assert_eq!(researcher["goal"], "Uncover developments in {topic}");
        assert_eq!(researcher["backstory"], "Line one.\nLine two.");
        assert_eq!(config["writer"]["role"], "Writer: docs");
    }

    /// A project with prompts in other modules, a text file and crewAI configs.
    #[test]
    fn finds_text_in_other_files() {
        let root = std::env::temp_dir().join(format!("gglint-project-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "app/prompts/root.py",
            r"
ROOT = 'You advise.'
def build():
    text = 'Be brief.'
    return text
def branches(x):
    if x:
        return 'a'
    return 'b'
",
        );
        write("app/prompts/__init__.py", "from .root import ROOT, build, branches\n");
        write("app/system.md", "Summarise.");
        write(
            "app/agent.py",
            r"
from pathlib import Path
from google.adk.agents import LlmAgent
from . import prompts
from .prompts import build, branches
import os
a = LlmAgent(name='a', instruction=prompts.ROOT)
b = LlmAgent(name='b', instruction=build())
c = LlmAgent(name='c', instruction=build)
d = LlmAgent(name='d', instruction=branches(1))
e = LlmAgent(name='e', instruction=(Path(__file__).parent / 'system.md').read_text())
f = LlmAgent(name='f', instruction=os.environ['PROMPT'] + prompts.MISSING)
",
        );
        write(
            "crew/crew.py",
            r"
from crewai import Agent, Task
class Crew:
    agents_config = 'config/agents.yaml'
    def researcher(self):
        return Agent(config=self.agents_config['researcher'])
    def task(self):
        return Task(config=self.tasks_config['missing'])
",
        );
        write(
            "crew/config/agents.yaml",
            r"
researcher:
  role: Researcher
  goal: >
    Find {topic}
",
        );

        let results = crate::discovery::scan(std::slice::from_ref(&root), false);
        let found: Vec<(String, String, Vec<String>)> = results
            .iter()
            .flat_map(|result| &result.prompts)
            .map(|(_, prompt)| {
                let sources = prompt
                    .sources
                    .iter()
                    .map(|source| source.strip_prefix(&root).unwrap().display().to_string())
                    .collect();
                (prompt.argument.to_string(), prompt.text.clone(), sources)
            })
            .collect();
        std::fs::remove_dir_all(&root).unwrap();

        let strings = |values: &[&str]| values.iter().map(|value| (*value).to_string()).collect::<Vec<_>>();
        assert_eq!(
            found,
            vec![
                ("instruction".into(), "You advise.".into(), strings(&["app/prompts/root.py"])),
                ("instruction".into(), "Be brief.".into(), strings(&["app/prompts/root.py"])),
                ("instruction".into(), "Be brief.".into(), strings(&["app/prompts/root.py"])),
                // Two `return`s: which one runs is only known at runtime.
                ("instruction".into(), "{branches(1)}".into(), vec![]),
                ("instruction".into(), "Summarise.".into(), strings(&["app/system.md"])),
                (
                    "instruction".into(),
                    "{os.environ['PROMPT']}{prompts.MISSING}".into(),
                    vec![]
                ),
                // The config has no `backstory`, so none is passed. `tasks.yaml` doesn't
                // exist, so the task's fields stay unknown.
                ("role".into(), "Researcher".into(), strings(&["crew/config/agents.yaml"])),
                ("goal".into(), "Find {topic}".into(), strings(&["crew/config/agents.yaml"])),
                (
                    "description".into(),
                    "{self.tasks_config['missing']}".into(),
                    vec![]
                ),
                (
                    "expected_output".into(),
                    "{self.tasks_config['missing']}".into(),
                    vec![]
                ),
            ]
        );
    }

    #[test]
    fn relative_imports() {
        let check = |module_path: &[&str], is_package: bool, name: &str| {
            let context = FileContext {
                path: Path::new("x.py"),
                root: Path::new("."),
                module_path,
                is_package,
                source: "",
            };
            let segments: Vec<&str> = name.split(' ').collect();
            absolute_name(&QualifiedName::from_segments(segments), &context).map(|name| name.join("."))
        };
        assert_eq!(check(&["app", "agent"], false, ". prompt ROOT"), Some("app.prompt.ROOT".into()));
        assert_eq!(check(&["app"], true, ". prompt ROOT"), Some("app.prompt.ROOT".into()));
        assert_eq!(check(&["app", "sub", "agent"], false, ". . prompt X"), Some("app.prompt.X".into()));
        assert_eq!(check(&["agent"], false, ". . prompt X"), None);
        assert_eq!(check(&["app", "agent"], false, "pkg X"), Some("pkg.X".into()));
    }
}
