//! The MCP servers each harness is set up to use, read from its config file.
//!
//! Each harness reads its servers from its own file, in its own format, at a fixed path
//! under the repository's root. So nothing is searched for: each path is opened once,
//! and a missing file costs one failed open. The files are parsed, never acted on:
//! commands aren't started, packages aren't downloaded and references to environment
//! variables, such as `${KEY}`, aren't expanded.

use std::path::{Path, PathBuf};

use jsonc_parser::ast::{Array, Object, Value};
use jsonc_parser::{CollectOptions, ParseOptions};
use toml::de::{DeTable, DeValue};
use yaml_rust2::{Yaml, YamlLoader};

use super::Harness;
use super::Location;
use super::agents::{AGENTS, Off, agent};

/// How a file lists its MCP servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    /// JSON: `"mcpServers": {name: {command, args, env} or {url, headers}}`, under the agent's
    /// own key.
    McpServers,
    /// TOML: `[mcp_servers.<name>]` tables.
    Codex,
    /// JSON with comments: `"mcp": {name: {command: [...], environment} or {url, headers}}`.
    OpenCode,
    /// TOML: `[[mcp_servers]]` tables, each with its `name` and `transport`: Mistral Vibe.
    Vibe,
    /// A folder of YAML files, each with an `mcpServers` list of servers with a `name`, and
    /// JSON files in the `mcpServers` format: Continue.
    Continue,
}

/// One harness's MCP config file.
pub struct McpConfig {
    pub harness: Harness,
    pub path: PathBuf,
    /// `path` relative to the repository's root.
    pub relative_path: PathBuf,
    /// In the order they're written.
    pub servers: Vec<McpServer>,
    /// What couldn't be read: the whole file, or one server.
    pub warnings: Vec<String>,
    /// Whether the agent doesn't read servers from it: it reads another file instead, or
    /// skips this one. It's listed for its warnings, and isn't compared or fixed.
    pub ignored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    /// Where its name is written.
    pub location: Location,
    pub transport: Transport,
    /// The environment variables it's started with, sorted by name.
    pub env: Vec<Setting>,
    /// The HTTP headers it's sent, sorted by name.
    pub headers: Vec<Setting>,
    /// `false` when the config turns it off (`enabled = false` in Codex and OpenCode).
    pub enabled: bool,
    /// The only tools the harness may use, when the config limits them (Codex's
    /// `enabled_tools`).
    pub enabled_tools: Option<Vec<String>>,
    /// The tools the harness may not use (Codex's `disabled_tools`).
    pub disabled_tools: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    /// A command the harness starts on the user's machine and talks to over standard
    /// input and output.
    Command { command: String, args: Vec<String> },
    /// A server the harness reaches over HTTP.
    Remote { url: String },
}

/// An environment variable or HTTP header given to a server.
///
/// Its value is split into text and references to the user's environment, read from each
/// harness's own way of writing them (`${KEY}`, `${env:KEY}`, `{env:KEY}`, Codex's
/// `env_vars`), so it can be compared, and written for another harness. The text can be a
/// secret written out in full, so don't print it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
    pub name: String,
    pub value: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Text(String),
    /// The value of the user's environment variable with this name.
    Env(String),
}

/// How a harness refers to an environment variable inside a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Syntax {
    /// `${KEY}`: Claude Code.
    Dollar,
    /// `${env:KEY}`: Cursor.
    DollarEnv,
    /// `{env:KEY}`: OpenCode.
    BraceEnv,
    /// `$KEY` or `${KEY}`, written `${KEY}`: Gemini CLI. Crush runs `$(command)` too.
    Shell,
    /// `${{ secrets.KEY }}`: Continue, which reads it from its `.env` files and then the
    /// environment.
    Secrets,
    /// It doesn't: Codex reads a whole value from the environment with other keys.
    Literal,
}

impl Syntax {
    /// How `harness` writes a reference in the value of an environment variable or header.
    pub(super) fn of(harness: Harness) -> Syntax {
        agent(harness).syntax
    }

    /// How `harness` writes a reference in a command, its arguments or a URL.
    pub(super) fn of_command_line(harness: Harness) -> Syntax {
        agent(harness).command_line_syntax
    }

    fn prefix(self) -> Option<&'static str> {
        match self {
            Syntax::Dollar => Some("${"),
            Syntax::DollarEnv => Some("${env:"),
            Syntax::BraceEnv => Some("{env:"),
            Syntax::Shell => Some("$"),
            // Read by `parse_secrets`.
            Syntax::Secrets => None,
            Syntax::Literal => None,
        }
    }
}

/// Splits `text` into text and references to environment variables written in `syntax`.
/// Anything else, such as `${KEY:-default}`, stays text.
pub(super) fn parse_value(text: &str, syntax: Syntax) -> Vec<Part> {
    if syntax == Syntax::Secrets {
        return parse_secrets(text);
    }
    let mut parts = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    if let Some(prefix) = syntax.prefix() {
        while let Some(start) = rest.find(prefix) {
            let after = &rest[start + prefix.len()..];
            // In the shell's syntax, the braces around the name are optional.
            let (body, braced) = match (syntax, after.strip_prefix('{')) {
                (Syntax::Shell, Some(body)) => (body, true),
                (Syntax::Shell, None) => (after, false),
                _ => (after, true),
            };
            let length = body
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(body.len());
            let name = &body[..length];
            let starts_well = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
            if starts_well && (!braced || body[length..].starts_with('}')) {
                plain.push_str(&rest[..start]);
                if !plain.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut plain)));
                }
                parts.push(Part::Env(name.to_string()));
                rest = &body[length + usize::from(braced)..];
            } else {
                plain.push_str(&rest[..start + prefix.len()]);
                rest = after;
            }
        }
    }
    plain.push_str(rest);
    if !plain.is_empty() {
        parts.push(Part::Text(plain));
    }
    parts
}

/// Splits `text` into text and Continue's references to secrets, `${{ secrets.KEY }}`, with or
/// without the spaces.
fn parse_secrets(text: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${{") {
        let after = &rest[start + 3..];
        let body = after.trim_start();
        let reference = body.strip_prefix("secrets.").and_then(|name_on| {
            let length = name_on
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(name_on.len());
            let tail = name_on[length..].trim_start().strip_prefix("}}")?;
            (length > 0).then(|| (name_on[..length].to_string(), tail))
        });
        match reference {
            Some((name, tail)) => {
                plain.push_str(&rest[..start]);
                if !plain.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut plain)));
                }
                parts.push(Part::Env(name));
                rest = tail;
            }
            None => {
                plain.push_str(&rest[..start + 3]);
                rest = after;
            }
        }
    }
    plain.push_str(rest);
    if !plain.is_empty() {
        parts.push(Part::Text(plain));
    }
    parts
}

/// `text`, written in `from`'s syntax, rewritten in `to`'s, if it can be.
pub(super) fn translate(text: &str, from: Syntax, to: Syntax) -> Option<String> {
    if from == to {
        return Some(text.to_string());
    }
    write_value(&parse_value(text, from), to)
}

/// `parts` written in `syntax`, if it can be: text that looks like a reference in any
/// harness's syntax would mean something else once written for another harness.
pub(super) fn write_value(parts: &[Part], syntax: Syntax) -> Option<String> {
    let mut value = String::new();
    for part in parts {
        match part {
            Part::Text(text) if confusing(text, syntax).is_some() => return None,
            Part::Text(text) => value.push_str(text),
            Part::Env(name) => match syntax {
                Syntax::Dollar | Syntax::Shell => value.push_str(&format!("${{{name}}}")),
                Syntax::DollarEnv => value.push_str(&format!("${{env:{name}}}")),
                Syntax::BraceEnv => value.push_str(&format!("{{env:{name}}}")),
                Syntax::Secrets => value.push_str(&format!("${{{{ secrets.{name} }}}}")),
                Syntax::Literal => return None,
            },
        }
    }
    Some(value)
}

/// The first part of `text` that would mean something else once written in `syntax`: what
/// looks like a reference in any harness's syntax (`${KEY:-default}`, `{env:KEY}`), or
/// `$KEY` in the shell's, where it's a reference even if it wasn't one where it was read.
pub(super) fn confusing(text: &str, syntax: Syntax) -> Option<&str> {
    let marked = ["${", "{env:", "{file:"]
        .iter()
        .filter_map(|marker| text.find(marker))
        .min();
    // `$(` too: Crush runs the command inside.
    let dollar = || {
        text.match_indices('$')
            .map(|(index, _)| index)
            .find(|&index| text[index + 1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '('))
    };
    let (start, closed) = match (marked, syntax) {
        (Some(start), _) => (start, true),
        (None, Syntax::Shell) => (dollar()?, false),
        (None, _) => return None,
    };
    let rest = &text[start..];
    let length = if closed {
        rest.find('}').map_or(rest.len(), |end| end + 1)
    } else if rest[1..].starts_with('(') {
        2
    } else {
        1 + rest[1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len() - 1)
    };
    Some(&rest[..length])
}

/// What some editors put at the start of a UTF-8 file.
pub(super) const BYTE_ORDER_MARK: char = '\u{feff}';

/// Reads every harness's MCP config file found under `root`, in the order they're reported.
pub fn read_mcp(root: &Path) -> Vec<McpConfig> {
    AGENTS
        .iter()
        .flat_map(|agent| {
            agent
                .files
                .iter()
                .flat_map(|&relative| files_at(root, relative))
                .map(move |relative| (agent, relative))
        })
        .filter_map(|(agent, relative)| {
            let relative = relative.as_str();
            let path = root.join(relative);
            let mut config = McpConfig {
                harness: agent.harness,
                path,
                relative_path: PathBuf::from(relative),
                servers: Vec::new(),
                warnings: Vec::new(),
                ignored: false,
            };
            let read_instead = agent.read_instead.filter(|other| root.join(other).is_file());
            match (std::fs::read_to_string(&config.path), read_instead) {
                // Its servers aren't the agent's while it reads another file instead.
                (Ok(_), Some(other)) => {
                    config.warnings.push(format!(
                        "{} doesn't read this file while {other} exists; it reads {other} instead",
                        agent.name
                    ));
                    config.ignored = true;
                }
                (Ok(text), None) => {
                    // A byte order mark, which editors on Windows often save, isn't JSON or TOML.
                    let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(&text);
                    match agent.format {
                        Format::McpServers => read_json(text, agent.servers_key, mcp_servers_entry, &mut config),
                        Format::Codex => read_codex(text, &mut config),
                        Format::OpenCode => read_json(text, agent.servers_key, opencode_entry, &mut config),
                        Format::Vibe => read_vibe(text, &mut config),
                        Format::Continue if relative.ends_with(".json") => {
                            read_json(text, agent.servers_key, continue_json_entry, &mut config)
                        }
                        Format::Continue => read_continue_yaml(text, &mut config),
                    }
                }
                // `NotADirectory`: `.codex/config.toml` where `.codex` is a file.
                (Err(error), _)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    return None;
                }
                (Err(error), _) => config.warnings.push(format!("couldn't read: {error}")),
            }
            for unreadable in agent.unreadable_files {
                if root.join(unreadable).is_file() {
                    config.warnings.push(format!(
                        "{} also reads servers from {unreadable}, a shell script gglint doesn't read",
                        agent.name
                    ));
                }
            }
            Some(config)
        })
        .collect()
}

/// The config files at `relative`: itself, or for a folder, written with a `/` at its end, its
/// YAML and JSON files in the order of their names.
fn files_at(root: &Path, relative: &str) -> Vec<String> {
    if !relative.ends_with('/') {
        return vec![relative.to_string()];
    }
    let Ok(entries) = std::fs::read_dir(root.join(relative)) else {
        return Vec::new();
    };
    let mut files: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|extension| ["yaml", "yml", "json"].contains(&extension.to_str().unwrap_or_default()))
        })
        .map(|name| format!("{relative}{name}"))
        .collect();
    files.sort();
    files
}

/// The warning for a server with neither a command nor a URL under one of `keys`.
fn no_transport(name: &str, keys: &[&str]) -> String {
    let keys: Vec<String> = keys.iter().map(|key| format!("`{key}`")).collect();
    format!("`{name}` has neither a `command` nor a {}", keys.join(" or "))
}

/// The 1-based line and column of the byte at `offset`.
fn location(text: &str, offset: usize) -> Location {
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    Location {
        line: memchr::memchr_iter(b'\n', before.as_bytes()).count() + 1,
        column: before[line_start..].chars().count() + 1,
    }
}

fn sorted(mut settings: Vec<Setting>) -> Vec<Setting> {
    settings.sort_by(|a, b| a.name.cmp(&b.name));
    settings
}

// ---------------------------------------------------------------------------
// JSON: the `mcpServers` format, and OpenCode
// ---------------------------------------------------------------------------

/// Reads the servers in the object at `key`, one entry at a time with `entry`.
fn read_json(
    text: &str,
    key: &str,
    entry: fn(String, Location, &Object, Harness) -> Result<McpServer, String>,
    config: &mut McpConfig,
) {
    // Comments and trailing commas are allowed: OpenCode's `.jsonc` files have them.
    let parsed = match jsonc_parser::parse_to_ast(text, &CollectOptions::default(), &ParseOptions::default()) {
        Ok(parsed) => parsed,
        Err(error) => {
            config.warnings.push(format!("couldn't parse: {error}"));
            return;
        }
    };
    let Some(Value::Object(root)) = parsed.value else {
        return;
    };
    // Copilot CLI's file can list its servers right at its top instead, and nothing else: it
    // skips a file that does neither.
    let at_top = root.get_object(key).is_none() && agent(config.harness).servers_at_top;
    if at_top
        && !root
            .properties
            .iter()
            .all(|property| matches!(&property.value, Value::Object(entry) if is_server(entry)))
    {
        config.warnings.push(format!(
            "{} skips this file: it lists no servers under `{key}` or at its top",
            agent(config.harness).name
        ));
        config.ignored = true;
        return;
    }
    let mut servers = match root.get_object(key) {
        Some(servers) => servers,
        None if at_top => &root,
        None => return,
    };
    // OpenCode's current layout keeps servers one level down, under `mcp.servers`. In the
    // older one they're right under `mcp`, so a server there can be called `servers`.
    if let Some(nested_key) = agent(config.harness).nested_key
        && let Some(nested) = servers.get_object(nested_key)
        && !is_server(nested)
    {
        servers = nested;
    }
    for property in &servers.properties {
        // At the top of a file, only an object is a server.
        if at_top && !matches!(property.value, Value::Object(_)) {
            continue;
        }
        let name = property.name.as_str().to_string();
        let location = location(text, property.range.start);
        let Value::Object(object) = &property.value else {
            config.warnings.push(format!("`{name}` isn't an object"));
            continue;
        };
        match entry(name, location, object, config.harness) {
            Ok(server) => config.servers.push(server),
            Err(warning) => config.warnings.push(warning),
        }
    }
    if agent(config.harness).off == Off::Lists
        && let Some(lists) = root.get_object("mcp")
    {
        let list = |key: &str| lists.get_array(key).map(json_strings);
        let (allowed, excluded) = (list("allowed"), list("excluded").unwrap_or_default());
        for server in &mut config.servers {
            if excluded.contains(&server.name)
                || allowed.as_ref().is_some_and(|allowed| !allowed.contains(&server.name))
            {
                server.enabled = false;
            }
        }
    }
}

/// Whether `object` is a server's entry rather than a group of them.
fn is_server(object: &Object) -> bool {
    ["type", "command", "url"].iter().any(|key| object.get(key).is_some())
}

fn json_string(object: &Object, key: &str) -> Option<String> {
    object.get_string(key).map(|string| string.value.to_string())
}

fn json_bool(object: &Object, key: &str) -> Option<bool> {
    match object.get(key).map(|property| &property.value) {
        Some(Value::BooleanLit(value)) => Some(value.value),
        _ => None,
    }
}

fn json_strings(array: &Array) -> Vec<String> {
    array
        .elements
        .iter()
        .filter_map(|element| element.as_string_lit().map(|string| string.value.to_string()))
        .collect()
}

fn json_settings(object: Option<&Object>, syntax: Syntax) -> Vec<Setting> {
    let Some(object) = object else { return Vec::new() };
    sorted(
        object
            .properties
            .iter()
            .filter_map(|property| {
                let value = property.value.as_string_lit()?;
                Some(Setting {
                    name: property.name.as_str().to_string(),
                    value: parse_value(&value.value, syntax),
                })
            })
            .collect(),
    )
}

/// An entry in an `mcpServers` file, such as Claude Code's `.mcp.json`. A server is started
/// with `command` or reached at `url`, whatever its `type` says.
fn mcp_servers_entry(name: String, location: Location, object: &Object, harness: Harness) -> Result<McpServer, String> {
    let syntax = Syntax::of(harness);
    entry_with(name, location, object, harness, syntax, syntax)
}

/// An entry in a JSON file in Continue's folder, written as Claude Code writes it: Continue
/// reads `${KEY}` in `env` as one of its secrets, and nothing else.
fn continue_json_entry(
    name: String,
    location: Location,
    object: &Object,
    harness: Harness,
) -> Result<McpServer, String> {
    entry_with(name, location, object, harness, Syntax::Dollar, Syntax::Literal)
}

/// An `mcpServers` entry whose environment variables are written in `env_syntax` and its
/// headers in `header_syntax`.
fn entry_with(
    name: String,
    location: Location,
    object: &Object,
    harness: Harness,
    env_syntax: Syntax,
    header_syntax: Syntax,
) -> Result<McpServer, String> {
    // Zed's older layout nests the command: `"command": { "path", "args", "env" }`.
    let nested = object
        .get_object("command")
        .filter(|nested| nested.get("path").is_some());
    let (launch, program) = match nested {
        Some(nested) => (nested, "path"),
        None => (object, "command"),
    };
    let transport = if let Some(command) = json_string(launch, program) {
        let args = launch.get_array("args").map(json_strings).unwrap_or_default();
        Transport::Command { command, args }
    } else if object.get("command").is_some() {
        return Err(format!("`{name}` has a `command` that isn't a string"));
    } else if let Some(url) = agent(harness)
        .remote
        .keys()
        .iter()
        .find_map(|key| json_string(object, key))
    {
        Transport::Remote { url }
    } else if object.get("settings").is_some() {
        // Zed: a server an extension starts, configured by its `settings`.
        return Err(format!(
            "`{name}` is started by a {} extension, so gglint can't compare it",
            agent(harness).name
        ));
    } else {
        return Err(no_transport(&name, agent(harness).remote.keys()));
    };
    let mut headers = json_settings(object.get_object("headers"), header_syntax);
    if let Some(key) = agent(harness).bearer_token_key
        && let Some(variable) = json_string(object, key)
    {
        headers.push(Setting {
            name: "Authorization".to_string(),
            value: vec![Part::Text("Bearer ".to_string()), Part::Env(variable)],
        });
    }
    Ok(McpServer {
        name,
        location,
        transport,
        env: json_settings(launch.get_object("env"), env_syntax),
        headers: sorted(headers),
        enabled: match agent(harness).off {
            Off::Disabled => json_bool(object, "disabled") != Some(true),
            Off::NotEnabled => json_bool(object, "enabled") != Some(false),
            // Read from the whole file in `read_json`.
            Off::Lists | Off::Never => true,
        },
        enabled_tools: None,
        disabled_tools: Vec::new(),
    })
}

/// OpenCode's `opencode.json`: `command` is one list, the program and its arguments.
fn opencode_entry(name: String, location: Location, object: &Object, harness: Harness) -> Result<McpServer, String> {
    let syntax = Syntax::of(harness);
    let transport = if let Some(command) = object.get_array("command") {
        let mut parts = json_strings(command).into_iter();
        let Some(command) = parts.next() else {
            return Err(format!("`{name}` has an empty `command`"));
        };
        Transport::Command {
            command,
            args: parts.collect(),
        }
    } else if object.get_string("command").is_some() {
        return Err(format!("`{name}` has a `command` that isn't a list"));
    } else if let Some(url) = json_string(object, "url") {
        Transport::Remote { url }
    } else {
        return Err(no_transport(&name, &["url"]));
    };
    let enabled = json_bool(object, "enabled") != Some(false);
    Ok(McpServer {
        name,
        location,
        transport,
        env: json_settings(object.get_object("environment"), syntax),
        headers: json_settings(object.get_object("headers"), syntax),
        enabled,
        enabled_tools: None,
        disabled_tools: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// TOML: Codex
// ---------------------------------------------------------------------------

fn toml_get<'a>(table: &'a DeTable<'a>, key: &str) -> Option<&'a DeValue<'a>> {
    table
        .iter()
        .find(|(name, _)| name.get_ref() == key)
        .map(|(_, value)| value.get_ref())
}

fn toml_string(table: &DeTable<'_>, key: &str) -> Option<String> {
    match toml_get(table, key)? {
        DeValue::String(string) => Some(string.to_string()),
        _ => None,
    }
}

fn toml_strings(table: &DeTable<'_>, key: &str) -> Option<Vec<String>> {
    let DeValue::Array(array) = toml_get(table, key)? else {
        return None;
    };
    Some(
        array
            .iter()
            .filter_map(|element| match element.get_ref() {
                DeValue::String(string) => Some(string.to_string()),
                _ => None,
            })
            .collect(),
    )
}

/// The string values of the table at `key`: each setting's name and value as written.
fn toml_pairs(table: &DeTable<'_>, key: &str) -> Vec<(String, String)> {
    let Some(DeValue::Table(settings)) = toml_get(table, key) else {
        return Vec::new();
    };
    settings
        .iter()
        .filter_map(|(name, value)| match value.get_ref() {
            DeValue::String(value) => Some((name.get_ref().to_string(), value.to_string())),
            _ => None,
        })
        .collect()
}

fn text_setting((name, value): (String, String)) -> Setting {
    Setting {
        name,
        value: vec![Part::Text(value)],
    }
}

/// A setting whose value is the user's environment variable `variable`.
fn env_setting(name: String, variable: String) -> Setting {
    Setting {
        name,
        value: vec![Part::Env(variable)],
    }
}

/// Codex's `.codex/config.toml`.
fn read_codex(text: &str, config: &mut McpConfig) {
    let root = match DeTable::parse(text) {
        Ok(root) => root.into_inner(),
        Err(error) => {
            config.warnings.push(format!("couldn't parse: {error}"));
            return;
        }
    };
    let Some(DeValue::Table(servers)) = toml_get(&root, "mcp_servers") else {
        return;
    };
    for (key, value) in servers {
        let name = key.get_ref().to_string();
        let DeValue::Table(table) = value.get_ref() else {
            config.warnings.push(format!("`{name}` isn't a table"));
            continue;
        };
        let transport = if let Some(command) = toml_string(table, "command") {
            let args = toml_strings(table, "args").unwrap_or_default();
            Transport::Command { command, args }
        } else if let Some(url) = toml_string(table, "url") {
            Transport::Remote { url }
        } else {
            config.warnings.push(no_transport(&name, &["url"]));
            continue;
        };

        let mut env: Vec<Setting> = toml_pairs(table, "env").into_iter().map(text_setting).collect();
        // `env_vars` pass variables through from the user's environment, under their names.
        env.extend(
            toml_strings(table, "env_vars")
                .unwrap_or_default()
                .into_iter()
                .map(|name| env_setting(name.clone(), name)),
        );
        let mut headers: Vec<Setting> = toml_pairs(table, "http_headers")
            .into_iter()
            .map(text_setting)
            .collect();
        // `env_http_headers` maps each header to the variable its value is read from.
        headers.extend(
            toml_pairs(table, "env_http_headers")
                .into_iter()
                .map(|(header, variable)| env_setting(header, variable)),
        );
        if let Some(variable) = toml_string(table, "bearer_token_env_var") {
            headers.push(Setting {
                name: "Authorization".to_string(),
                value: vec![Part::Text("Bearer ".to_string()), Part::Env(variable)],
            });
        }
        let enabled = !matches!(toml_get(table, "enabled"), Some(DeValue::Boolean(false)));

        config.servers.push(McpServer {
            location: location(text, key.span().start),
            name,
            transport,
            env: sorted(env),
            headers: sorted(headers),
            enabled,
            enabled_tools: toml_strings(table, "enabled_tools"),
            disabled_tools: toml_strings(table, "disabled_tools").unwrap_or_default(),
        });
    }
    // A TOML table's keys come out sorted by name.
    config
        .servers
        .sort_by_key(|server| (server.location.line, server.location.column));
}

// ---------------------------------------------------------------------------
// TOML: Mistral Vibe
// ---------------------------------------------------------------------------

/// A command written as one string, split into words as a POSIX shell does: by spaces,
/// keeping what's quoted together. Mistral Vibe splits it with Python's `shlex`.
pub(super) fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                word.extend(chars.by_ref().take_while(|&c| c != '\''));
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.extend(chars.next());
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// A header read from the environment with `api_key_env`: its name and value, from
/// `api_key_header` and `api_key_format`, `Authorization: Bearer {token}` by default.
fn vibe_api_key(table: &DeTable<'_>) -> Option<Setting> {
    let variable = toml_string(table, "api_key_env").filter(|variable| !variable.is_empty())?;
    let header = toml_string(table, "api_key_header").unwrap_or_else(|| "Authorization".to_string());
    let format = toml_string(table, "api_key_format").unwrap_or_else(|| "Bearer {token}".to_string());
    let (before, after) = format.split_once("{token}")?;
    let mut value = Vec::new();
    if !before.is_empty() {
        value.push(Part::Text(before.to_string()));
    }
    value.push(Part::Env(variable));
    if !after.is_empty() {
        value.push(Part::Text(after.to_string()));
    }
    Some(Setting { name: header, value })
}

/// Mistral Vibe's `.vibe/config.toml`: a list of `[[mcp_servers]]` tables.
fn read_vibe(text: &str, config: &mut McpConfig) {
    let root = match DeTable::parse(text) {
        Ok(root) => root.into_inner(),
        Err(error) => {
            config.warnings.push(format!("couldn't parse: {error}"));
            return;
        }
    };
    let Some(DeValue::Array(servers)) = toml_get(&root, "mcp_servers") else {
        return;
    };
    for value in servers.iter() {
        let DeValue::Table(table) = value.get_ref() else {
            config
                .warnings
                .push("an entry of `mcp_servers` isn't a table".to_string());
            continue;
        };
        let Some((key, name)) = table.iter().find_map(|(key, value)| match value.get_ref() {
            DeValue::String(name) if key.get_ref() == "name" => Some((key, name.to_string())),
            _ => None,
        }) else {
            config
                .warnings
                .push("an entry of `mcp_servers` has no `name`".to_string());
            continue;
        };
        let transport = match toml_string(table, "transport").as_deref() {
            Some("stdio") => {
                let mut words = match toml_get(table, "command") {
                    Some(DeValue::String(command)) => split_command(command),
                    Some(DeValue::Array(_)) => toml_strings(table, "command").unwrap_or_default(),
                    _ => Vec::new(),
                };
                if words.is_empty() {
                    config.warnings.push(format!("`{name}` has no `command`"));
                    continue;
                }
                let command = words.remove(0);
                words.extend(toml_strings(table, "args").unwrap_or_default());
                Transport::Command { command, args: words }
            }
            Some("http" | "streamable-http") => match toml_string(table, "url") {
                Some(url) => Transport::Remote { url },
                None => {
                    config.warnings.push(format!("`{name}` has no `url`"));
                    continue;
                }
            },
            other => {
                let other = other.map_or("none".to_string(), |other| format!("`{other}`"));
                config.warnings.push(format!(
                    "`{name}` has the transport {other}, not `stdio`, `http` or `streamable-http`"
                ));
                continue;
            }
        };
        // The headers and API key are right in the entry in the older layout, and in its
        // `auth` table, of type `static`, in the current one.
        let auth = match toml_get(table, "auth") {
            Some(DeValue::Table(auth)) => Some(auth),
            _ => None,
        };
        let source = auth.unwrap_or(table);
        let mut headers: Vec<Setting> = toml_pairs(source, "headers").into_iter().map(text_setting).collect();
        if let Some(api_key) = vibe_api_key(source)
            && !headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case(&api_key.name))
        {
            headers.push(api_key);
        }
        config.servers.push(McpServer {
            location: location(text, key.span().start),
            name,
            transport,
            env: sorted(toml_pairs(table, "env").into_iter().map(text_setting).collect()),
            headers: sorted(headers),
            enabled: !matches!(toml_get(table, "disabled"), Some(DeValue::Boolean(true))),
            enabled_tools: None,
            disabled_tools: toml_strings(table, "disabled_tools").unwrap_or_default(),
        });
    }
}

// ---------------------------------------------------------------------------
// YAML: Continue
// ---------------------------------------------------------------------------

fn yaml_strings(value: &Yaml) -> Vec<String> {
    value
        .as_vec()
        .map(|items| items.iter().filter_map(yaml_text).collect())
        .unwrap_or_default()
}

/// A scalar as the text Continue reads, whatever YAML takes it for: `1` is `"1"`.
fn yaml_text(value: &Yaml) -> Option<String> {
    match value {
        Yaml::String(text) | Yaml::Real(text) => Some(text.clone()),
        Yaml::Integer(number) => Some(number.to_string()),
        Yaml::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn yaml_settings(value: &Yaml, syntax: Syntax) -> Vec<Setting> {
    let Some(map) = value.as_hash() else {
        return Vec::new();
    };
    sorted(
        map.iter()
            .filter_map(|(name, value)| {
                Some(Setting {
                    name: yaml_text(name)?,
                    value: parse_value(&yaml_text(value)?, syntax),
                })
            })
            .collect(),
    )
}

/// Where the server named `name` starts: the first line after `after` that's an item of a
/// list with that `name`. The parser doesn't keep where it read each value.
fn yaml_location(text: &str, name: &str, after: usize) -> (Location, usize) {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if start < after {
            continue;
        }
        let item = line.trim_start().strip_prefix("- ").map(str::trim_start);
        let Some(value) = item.and_then(|item| item.strip_prefix("name:")) else {
            continue;
        };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        if value == name {
            let column = line.len() - line.trim_start().len();
            return (location(text, start + column), offset);
        }
    }
    (Location { line: 1, column: 1 }, after)
}

/// A YAML file in Continue's folder: a block with its `mcpServers` list.
fn read_continue_yaml(text: &str, config: &mut McpConfig) {
    let documents = match YamlLoader::load_from_str(text) {
        Ok(documents) => documents,
        Err(error) => {
            config.warnings.push(format!("couldn't parse: {error}"));
            return;
        }
    };
    let syntax = Syntax::of(config.harness);
    let mut after = 0;
    for document in &documents {
        let Some(servers) = document["mcpServers"].as_vec() else {
            continue;
        };
        for entry in servers {
            // A block from Continue's hub, `uses: owner/name`, is set up there.
            if let Some(block) = entry["uses"].as_str() {
                config.warnings.push(format!(
                    "`{block}` comes from Continue's hub, so gglint can't compare it"
                ));
                continue;
            }
            let Some(name) = entry["name"].as_str().map(str::to_string) else {
                config
                    .warnings
                    .push("an entry of `mcpServers` has no `name`".to_string());
                continue;
            };
            let transport = if let Some(command) = yaml_text(&entry["command"]) {
                Transport::Command {
                    command,
                    args: yaml_strings(&entry["args"]),
                }
            } else if let Some(url) = yaml_text(&entry["url"]) {
                Transport::Remote { url }
            } else {
                config.warnings.push(no_transport(&name, &["url"]));
                continue;
            };
            let (location, next) = yaml_location(text, &name, after);
            after = next;
            config.servers.push(McpServer {
                name,
                location,
                transport,
                env: yaml_settings(&entry["env"], syntax),
                headers: yaml_settings(&entry["requestOptions"]["headers"], syntax),
                enabled: true,
                enabled_tools: None,
                disabled_tools: Vec::new(),
            });
        }
    }
}
