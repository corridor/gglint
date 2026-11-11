//! `gglint mcp --fix`: gives every harness the repository is set up for each server.
//!
//! It adds, and never changes or removes what is there. A server missing from a harness is
//! added to its config file, copied from the first harness that has it. The file is created
//! only when the harness has its own directory (`.claude`, `.cursor`, ...): a `CLAUDE.md`
//! alone says people use Claude Code, not that they want MCP servers in it. Comments and
//! formatting are kept.
//!
//! A server set up differently is left as it is: in real repositories most differences
//! between harnesses are deliberate, such as each harness running its own runtime or
//! setting its own environment. With `FixOptions::overwrite` it's changed to match the
//! first harness's setup instead; only the keys that say how a server is started and what
//! it's given are written, so keys only one harness has, such as Codex's `enabled_tools`,
//! are kept.
//!
//! Left for a person either way: a different program under the same name, a server turned
//! off, another name, and a value that can't be written in the other harness's syntax.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table};

use super::compare::{Cell, ServerCheck, same_program};
use super::mcp::{Part, Setting, Syntax, parse_value, translate, write_value};
use super::{HARNESSES, Harness, McpConfig, McpServer, Transport};

/// How `fix_mcp` may change the config files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixOptions {
    /// Change a server set up differently to match the first harness's setup. Off by
    /// default: most differences between harnesses are deliberate.
    pub overwrite: bool,
}

/// What `fix_mcp` changed, and what it left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixReport {
    /// In the order harnesses are reported.
    pub changed: Vec<ChangedFile>,
    pub unfixed: Vec<Unfixed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    pub harness: Harness,
    /// Relative to the repository's root.
    pub relative_path: PathBuf,
    /// Whether it didn't exist before.
    pub created: bool,
    /// The servers added to it, and the ones changed to match the first harness's.
    pub added: Vec<String>,
    pub updated: Vec<String>,
    /// Whether the repository's `.gitignore` leaves it out, so the change stays on this
    /// machine and isn't committed.
    pub ignored: bool,
}

/// A server `fix_mcp` couldn't set up in a harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unfixed {
    pub harness: Harness,
    pub server: String,
    pub reason: String,
}

/// The value of a key in a server's entry.
enum Field {
    Text(String),
    List(Vec<String>),
    Map(Vec<(String, String)>),
}

/// One server to write into one harness's file.
struct Write {
    harness: Harness,
    relative_path: PathBuf,
    /// Its name in that file.
    name: String,
    fields: Vec<(&'static str, Field)>,
    /// Whether it's already in the file, to be changed rather than added.
    update: bool,
}

/// Fixes the problems in `checks` that have one right answer, by writing config files
/// under `root`.
pub fn fix_mcp(root: &Path, configs: &[McpConfig], checks: &[ServerCheck], options: &FixOptions) -> FixReport {
    let mut writes: Vec<Write> = Vec::new();
    let mut report = FixReport::default();
    for check in checks {
        let reference = &check.setups[0];
        let from = reference.site.harness.display_name();
        for &(harness, cell) in &check.cells {
            let target = check.setups.iter().find(|setup| setup.site.harness == harness);
            let name = target
                .map_or(&reference.server.name, |target| &target.server.name)
                .clone();
            let mut leave = |reason: String| {
                report.unfixed.push(Unfixed {
                    harness,
                    server: name.clone(),
                    reason,
                })
            };
            let update = match cell {
                Cell::Missing => {
                    let file = config_file(harness, configs);
                    if !root.join(&file).exists() && !root.join(harness.directory()).is_dir() {
                        leave(format!(
                            "{} has no {} and no {} directory; create {} if its users want MCP servers",
                            harness.display_name(),
                            file.display(),
                            harness.directory(),
                            file.display()
                        ));
                        continue;
                    }
                    false
                }
                Cell::Different => {
                    if let Some(target) = target
                        && !same_program(target, reference)
                    {
                        leave(format!(
                            "it's a different server than in {from}; decide which one is right"
                        ));
                        continue;
                    }
                    if !options.overwrite {
                        leave(format!(
                            "set up differently from {from}; left as it is, and `--overwrite` copies {from}'s setup"
                        ));
                        continue;
                    }
                    true
                }
                // A server turned off may be off on purpose.
                Cell::Same | Cell::Off => continue,
            };
            match fields(&reference.server, reference.site.harness, harness) {
                Ok(fields) => writes.push(Write {
                    harness,
                    relative_path: target.map_or_else(
                        || config_file(harness, configs),
                        |target| target.site.relative_path.clone(),
                    ),
                    name,
                    fields,
                    update,
                }),
                Err(reason) => report.unfixed.push(Unfixed {
                    harness,
                    server: name,
                    reason,
                }),
            }
        }
    }

    // One file at a time, in the order harnesses are reported.
    let mut files: Vec<(Harness, PathBuf)> = Vec::new();
    for write in &writes {
        if !files.iter().any(|(_, path)| *path == write.relative_path) {
            files.push((write.harness, write.relative_path.clone()));
        }
    }
    files.sort_by_key(|(harness, _)| HARNESSES.iter().position(|other| other == harness));
    for (harness, relative_path) in files {
        let file_writes: Vec<&Write> = writes
            .iter()
            .filter(|write| write.relative_path == relative_path)
            .collect();
        match write_file(root, harness, &relative_path, &file_writes) {
            Ok(Some(created)) => report.changed.push(ChangedFile {
                harness,
                ignored: ignored_by_git(root, &relative_path),
                relative_path,
                created,
                added: names(&file_writes, false),
                updated: names(&file_writes, true),
            }),
            Ok(None) => {}
            Err(reason) => report.unfixed.extend(file_writes.iter().map(|write| Unfixed {
                harness,
                server: write.name.clone(),
                reason: format!("{}: {reason}", relative_path.display()),
            })),
        }
    }
    report
}

/// Whether git ignores `relative_path` under `root`. The one program gglint runs is git
/// itself; outside a repository, or without git, nothing counts as ignored.
fn ignored_by_git(root: &Path, relative_path: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "-q", "--"])
        .arg(relative_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn names(writes: &[&Write], update: bool) -> Vec<String> {
    writes
        .iter()
        .filter(|write| write.update == update)
        .map(|write| write.name.clone())
        .collect()
}

/// The file `harness` reads servers from: the one it has, or the one to create.
fn config_file(harness: Harness, configs: &[McpConfig]) -> PathBuf {
    configs.iter().find(|config| config.harness == harness).map_or_else(
        || PathBuf::from(harness.mcp_file()),
        |config| config.relative_path.clone(),
    )
}

// ---------------------------------------------------------------------------
// A server's entry, in each harness's words
// ---------------------------------------------------------------------------

/// The keys `fields` writes in an entry that's already there. `type` isn't one: it only
/// says whether the server is started or reached, which is the same in a server to update.
fn written_keys(harness: Harness) -> &'static [&'static str] {
    match harness {
        Harness::ClaudeCode | Harness::Cursor => &["command", "args", "env", "url", "headers"],
        Harness::OpenCode => &["command", "environment", "url", "headers"],
        Harness::Codex => &[
            "command",
            "args",
            "env",
            "env_vars",
            "url",
            "http_headers",
            "env_http_headers",
            "bearer_token_env_var",
        ],
    }
}

/// `transport`'s command line or URL, with references to the environment written the way
/// `to` writes them, or why it can't be.
fn written_transport(transport: &Transport, from: Harness, to: Harness) -> Result<Transport, String> {
    let (from_syntax, to_syntax) = (Syntax::of(from), Syntax::of(to));
    let write = |text: &String, what: &str| {
        translate(text, from_syntax, to_syntax).ok_or_else(|| {
            let variables: Vec<String> = parse_value(text, from_syntax)
                .into_iter()
                .filter_map(|part| match part {
                    Part::Env(name) => Some(format!("`{name}`")),
                    Part::Text(_) => None,
                })
                .collect();
            let to = to.display_name();
            match variables.as_slice() {
                [] => format!("the {what} can't be written for {to}"),
                [one] => format!("the {what} uses the environment variable {one}, which {to} can't fill in"),
                many => format!(
                    "the {what} uses the environment variables {}, which {to} can't fill in",
                    many.join(", ")
                ),
            }
        })
    };
    match transport {
        Transport::Command { command, args } => Ok(Transport::Command {
            command: write(command, "command line")?,
            args: args
                .iter()
                .map(|arg| write(arg, "command line"))
                .collect::<Result<_, _>>()?,
        }),
        Transport::Remote { url } => Ok(Transport::Remote {
            url: write(url, "URL")?,
        }),
    }
}

/// `server`'s entry in `harness`'s file, as its keys and values, or why it can't be written.
/// `source` is the harness `server` was read from.
fn fields(server: &McpServer, source: Harness, harness: Harness) -> Result<Vec<(&'static str, Field)>, String> {
    let server = &McpServer {
        transport: written_transport(&server.transport, source, harness)?,
        ..server.clone()
    };
    let mut fields = Vec::new();
    let mut add = |key: &'static str, field: Field| {
        if !is_empty(&field) {
            fields.push((key, field));
        }
    };
    let cannot = |what: &str| format!("the value of {what} can't be written for {}", harness.display_name());

    if harness == Harness::Codex {
        return codex_fields(server, cannot)
            .map(|codex| codex.into_iter().filter(|(_, field)| !is_empty(field)).collect());
    }
    let syntax = Syntax::of(harness);
    let settings = |settings: &[Setting], what: &dyn Fn(&str) -> String| -> Result<Field, String> {
        settings
            .iter()
            .map(|setting| {
                write_value(&setting.value, syntax)
                    .map(|value| (setting.name.clone(), value))
                    .ok_or_else(|| cannot(&what(&setting.name)))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Field::Map)
    };
    let env = settings(&server.env, &|name| format!("`{name}`"))?;
    let headers = settings(&server.headers, &|name| format!("the `{name}` header"))?;
    match (&server.transport, harness) {
        (Transport::Command { command, args }, Harness::OpenCode) => {
            add("type", Field::Text("local".to_string()));
            let mut list = vec![command.clone()];
            list.extend(args.iter().cloned());
            add("command", Field::List(list));
            add("environment", env);
        }
        (Transport::Command { command, args }, _) => {
            add("command", Field::Text(command.clone()));
            add("args", Field::List(args.clone()));
            add("env", env);
        }
        (Transport::Remote { url }, Harness::OpenCode) => {
            add("type", Field::Text("remote".to_string()));
            add("url", Field::Text(url.clone()));
            add("headers", headers);
        }
        (Transport::Remote { url }, _) => {
            // Claude Code starts an entry without a `type` as a command. A URL ending in
            // `/sse` is the older SSE transport.
            if harness == Harness::ClaudeCode {
                let kind = if url.trim_end_matches('/').ends_with("/sse") {
                    "sse"
                } else {
                    "http"
                };
                add("type", Field::Text(kind.to_string()));
            }
            add("url", Field::Text(url.clone()));
            add("headers", headers);
        }
    }
    Ok(fields)
}

fn is_empty(field: &Field) -> bool {
    match field {
        Field::Text(_) => false,
        Field::List(list) => list.is_empty(),
        Field::Map(map) => map.is_empty(),
    }
}

/// Codex's values are literal. A value read whole from the environment goes in another key:
/// `env_vars` for a variable passed under its own name, `env_http_headers` for a header,
/// and `bearer_token_env_var` for `Authorization: Bearer ${TOKEN}`.
fn codex_fields(server: &McpServer, cannot: impl Fn(&str) -> String) -> Result<Vec<(&'static str, Field)>, String> {
    let literal = |setting: &Setting| write_value(&setting.value, Syntax::Literal);
    match &server.transport {
        Transport::Command { command, args } => {
            let mut env = Vec::new();
            let mut env_vars = Vec::new();
            for setting in &server.env {
                match setting.value.as_slice() {
                    [Part::Env(variable)] if *variable == setting.name => env_vars.push(variable.clone()),
                    _ => match literal(setting) {
                        Some(value) => env.push((setting.name.clone(), value)),
                        None => return Err(cannot(&format!("`{}`", setting.name))),
                    },
                }
            }
            Ok(vec![
                ("command", Field::Text(command.clone())),
                ("args", Field::List(args.clone())),
                ("env", Field::Map(env)),
                ("env_vars", Field::List(env_vars)),
            ])
        }
        Transport::Remote { url } => {
            let mut headers = Vec::new();
            let mut env_headers = Vec::new();
            let mut bearer = None;
            for setting in &server.headers {
                match setting.value.as_slice() {
                    [Part::Env(variable)] => env_headers.push((setting.name.clone(), variable.clone())),
                    [Part::Text(prefix), Part::Env(variable)]
                        if setting.name.eq_ignore_ascii_case("Authorization") && prefix == "Bearer " =>
                    {
                        bearer = Some(variable.clone());
                    }
                    _ => match literal(setting) {
                        Some(value) => headers.push((setting.name.clone(), value)),
                        None => return Err(cannot(&format!("the `{}` header", setting.name))),
                    },
                }
            }
            let mut fields = vec![
                ("url", Field::Text(url.clone())),
                ("http_headers", Field::Map(headers)),
                ("env_http_headers", Field::Map(env_headers)),
            ];
            if let Some(variable) = bearer {
                fields.push(("bearer_token_env_var", Field::Text(variable)));
            }
            Ok(fields)
        }
    }
}

// ---------------------------------------------------------------------------
// Writing files
// ---------------------------------------------------------------------------

/// Writes `writes` into the file at `relative_path`, if that changes it: `Some(created)`.
fn write_file(root: &Path, harness: Harness, relative_path: &Path, writes: &[&Write]) -> Result<Option<bool>, String> {
    let path = root.join(relative_path);
    let (text, created) = match std::fs::read_to_string(&path) {
        Ok(text) => (text, false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (String::new(), true),
        Err(error) => return Err(format!("couldn't read it: {error}")),
    };
    let new_text = match harness {
        Harness::Codex => write_toml(&text, writes)?,
        Harness::ClaudeCode | Harness::Cursor => write_json(&text, "mcpServers", None, writes)?,
        Harness::OpenCode => write_json(&text, "mcp", Some("https://opencode.ai/config.json"), writes)?,
    };
    if new_text == text {
        return Ok(None);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("couldn't create its directory: {error}"))?;
    }
    std::fs::write(&path, new_text).map_err(|error| format!("couldn't write it: {error}"))?;
    Ok(Some(created))
}

fn json_value(field: &Field) -> CstInputValue {
    match field {
        Field::Text(text) => text.as_str().into(),
        Field::List(list) => CstInputValue::Array(list.iter().map(|item| item.as_str().into()).collect()),
        Field::Map(map) => CstInputValue::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), value.as_str().into()))
                .collect(),
        ),
    }
}

/// Writes the servers into the object at `key`, keeping the rest of the file as it is. A
/// new file starts with `schema`, when the harness has one.
fn write_json(text: &str, key: &str, schema: Option<&str>, writes: &[&Write]) -> Result<String, String> {
    let root =
        CstRootNode::parse(text, &ParseOptions::default()).map_err(|error| format!("couldn't parse it: {error}"))?;
    let object = root.object_value_or_set();
    if let Some(schema) = schema
        && text.trim().is_empty()
    {
        object.append("$schema", schema.into());
    }
    let mut servers = object.object_value_or_set(key);
    // OpenCode: write where the file keeps its servers, under `mcp.servers` in its current
    // layout or right under `mcp` in the older one. A file with none gets the current one.
    if key == "mcp" {
        let is_server = |entry: &CstObject| ["type", "command", "url"].iter().any(|key| entry.get(key).is_some());
        match servers.object_value("servers") {
            Some(nested) if !is_server(&nested) => servers = nested,
            Some(_) => {}
            None if servers.properties().is_empty() => servers = servers.object_value_or_set("servers"),
            None => {}
        }
    }
    for write in writes {
        match servers.get(&write.name).and_then(|property| property.object_value()) {
            Some(entry) if write.update => {
                for &key in written_keys(write.harness) {
                    match (write.fields.iter().find(|(name, _)| *name == key), entry.get(key)) {
                        (Some((_, field)), Some(property)) => property.set_value(json_value(field)),
                        (Some((_, field)), None) => {
                            entry.append(key, json_value(field));
                        }
                        (None, Some(property)) => property.remove(),
                        (None, None) => {}
                    }
                }
            }
            _ => {
                let entry = write
                    .fields
                    .iter()
                    .map(|(key, field)| (key.to_string(), json_value(field)))
                    .collect();
                servers.append(&write.name, CstInputValue::Object(entry));
            }
        }
    }
    let mut written = root.to_string();
    if !written.ends_with('\n') {
        written.push('\n');
    }
    Ok(written)
}

fn toml_item(field: &Field) -> Item {
    match field {
        Field::Text(text) => toml_edit::value(text.as_str()),
        Field::List(list) => toml_edit::value(Array::from_iter(list.iter().map(String::as_str))),
        Field::Map(map) => toml_edit::value(InlineTable::from_iter(
            map.iter().map(|(key, value)| (key.as_str(), value.as_str())),
        )),
    }
}

/// Writes the servers as `[mcp_servers.<name>]` tables, keeping the rest of the file as it is.
fn write_toml(text: &str, writes: &[&Write]) -> Result<String, String> {
    let mut document: DocumentMut = text.parse().map_err(|error| format!("couldn't parse it: {error}"))?;
    let servers = document.entry("mcp_servers").or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    let Some(servers) = servers.as_table_mut() else {
        return Err("its `mcp_servers` is written on one line".to_string());
    };
    for write in writes {
        match servers.get_mut(&write.name).and_then(Item::as_table_like_mut) {
            Some(entry) if write.update => {
                for &key in written_keys(write.harness) {
                    match write.fields.iter().find(|(name, _)| *name == key) {
                        Some((_, field)) => {
                            entry.insert(key, toml_item(field));
                        }
                        None => {
                            entry.remove(key);
                        }
                    }
                }
            }
            _ => {
                let mut table = Table::new();
                for (key, field) in &write.fields {
                    table.insert(key, toml_item(field));
                }
                servers.insert(&write.name, Item::Table(table));
            }
        }
    }
    Ok(document.to_string())
}
