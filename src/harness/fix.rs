//! `gglint mcp --fix`: gives every agent the same servers, started the same way.
//!
//! Each server is copied from the first file where it's turned on, in the order agents are
//! reported, and written the way each agent writes it:
//!
//! - A server missing from an agent's config file is added to that file.
//! - An agent with its own directory and no config file gets one, with every server.
//! - A server that runs another version of the same program, or the same program with other
//!   arguments, gets the command and arguments of the first file. Its environment, headers
//!   and keys only one agent has (Codex's `cwd`, for example) are kept.
//!
//! No server is removed, and comments and formatting are kept.
//!
//! Left for a person: a different program under the same name, other environment variables
//! or headers, a server turned off in one agent, and a server whose setup can't be written
//! for the other agent, such as a command line that reads an environment variable Codex
//! can't fill in.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table};

use super::compare::{Cell, McpCheck, Setup, same_command_line, same_program};
use super::mcp::{Part, Setting, Syntax, parse_value, translate, write_value};
use super::{HARNESSES, Harness, McpConfig, McpServer, Transport};

/// What `fix_mcp` changed, and what it left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixReport {
    /// In the order agents are reported.
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
    /// The servers added to it, and those whose command line was changed.
    pub added: Vec<String>,
    pub updated: Vec<String>,
    /// Whether the repository's `.gitignore` leaves it out, so the change stays on this
    /// machine and isn't committed.
    pub ignored: bool,
}

/// A server `fix_mcp` couldn't add to an agent's config file.
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

/// One server to add to one agent's file, or whose command line to change in it.
struct Write {
    harness: Harness,
    relative_path: PathBuf,
    /// Its name in that file.
    name: String,
    fields: Vec<(&'static str, Field)>,
    /// Whether it's already in the file, so only the keys in `fields` are written.
    update: bool,
}

/// Fixes what `check` found in the config files under `root`: adds the missing servers and
/// config files, and starts each server the same way everywhere.
pub fn fix_mcp(root: &Path, configs: &[McpConfig], check: &McpCheck) -> FixReport {
    let mut writes: Vec<Write> = Vec::new();
    let mut report = FixReport::default();
    for server in &check.servers {
        let Some(source) = &server.source else {
            continue;
        };
        for &(harness, cell) in &server.cells {
            if cell != Cell::Missing {
                continue;
            }
            // An agent compared without a config file has its own directory: it gets one.
            let relative_path = configs.iter().find(|config| config.harness == harness).map_or_else(
                || PathBuf::from(harness.mcp_file()),
                |config| config.relative_path.clone(),
            );
            match fields(&source.server, source.site.harness, harness) {
                Ok(fields) => writes.push(Write {
                    harness,
                    relative_path,
                    name: source.server.name.clone(),
                    fields,
                    update: false,
                }),
                Err(reason) => report.unfixed.push(Unfixed {
                    harness,
                    server: source.server.name.clone(),
                    reason,
                }),
            }
        }
        for target in server
            .setups
            .iter()
            .filter(|target| needs_new_command_line(target, source))
        {
            let harness = target.site.harness;
            match command_line_fields(&source.server.transport, source.site.harness, harness) {
                Ok(fields) => writes.push(Write {
                    harness,
                    relative_path: target.site.relative_path.clone(),
                    name: target.server.name.clone(),
                    fields,
                    update: true,
                }),
                Err(reason) => report.unfixed.push(Unfixed {
                    harness,
                    server: target.server.name.clone(),
                    reason,
                }),
            }
        }
    }

    // One file at a time, in the order agents are reported.
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

/// Whether `target` should start as `source` does: the same program, turned on, started
/// with another version or other arguments.
fn needs_new_command_line(target: &Setup, source: &Setup) -> bool {
    target.site.harness != source.site.harness
        && target.server.enabled
        && same_program(target, source)
        && !same_command_line(target, source)
}

fn names(writes: &[&Write], update: bool) -> Vec<String> {
    writes
        .iter()
        .filter(|write| write.update == update)
        .map(|write| write.name.clone())
        .collect()
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

// ---------------------------------------------------------------------------
// A server's entry, in each agent's words
// ---------------------------------------------------------------------------

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

/// The keys that say how `transport`, read from `source`, starts in `harness`'s file: its
/// command and arguments, or OpenCode's one `command` list.
fn command_line_fields(
    transport: &Transport,
    source: Harness,
    harness: Harness,
) -> Result<Vec<(&'static str, Field)>, String> {
    match written_transport(transport, source, harness)? {
        Transport::Command { command, args } if harness == Harness::OpenCode => {
            let mut list = vec![command];
            list.extend(args);
            Ok(vec![("command", Field::List(list))])
        }
        Transport::Command { command, args } => {
            Ok(vec![("command", Field::Text(command)), ("args", Field::List(args))])
        }
        Transport::Remote { .. } => Err("a remote server has no command line to change".to_string()),
    }
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

/// Adds `writes` to the file at `relative_path`, creating it if it isn't there, if that
/// changes it: `Some(created)`.
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

/// Adds the servers to the object at `key`, keeping the rest of the file as it is. A new
/// file starts with `schema`, when the agent has one.
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
        let existing = servers.get(&write.name).and_then(|property| property.object_value());
        match existing {
            Some(entry) if write.update => {
                for (key, field) in &write.fields {
                    match entry.get(key) {
                        Some(property) if is_empty(field) => property.remove(),
                        Some(property) => property.set_value(json_value(field)),
                        None if is_empty(field) => {}
                        None => {
                            entry.append(key, json_value(field));
                        }
                    }
                }
            }
            // Never over a server that's there.
            Some(_) => {}
            None if write.update => {}
            None => {
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

/// Adds the servers as `[mcp_servers.<name>]` tables, keeping the rest of the file as it is.
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
        if write.update {
            if let Some(entry) = servers.get_mut(&write.name).and_then(Item::as_table_like_mut) {
                for (key, field) in &write.fields {
                    if is_empty(field) {
                        entry.remove(key);
                    } else {
                        entry.insert(key, toml_item(field));
                    }
                }
            }
            continue;
        }
        // Never over a server that's there.
        if servers.contains_key(&write.name) {
            continue;
        }
        let mut table = Table::new();
        for (key, field) in &write.fields {
            table.insert(key, toml_item(field));
        }
        servers.insert(&write.name, Item::Table(table));
    }
    Ok(document.to_string())
}
