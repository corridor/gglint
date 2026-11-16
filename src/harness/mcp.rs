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

use super::Harness;
use super::Location;
use super::agents::{AGENTS, Off, agent};

/// How a file lists its MCP servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    /// JSON: `"mcpServers": {name: {command, args, env} or {url, headers}}`.
    McpServers,
    /// TOML: `[mcp_servers.<name>]` tables.
    Codex,
    /// JSON with comments: `"mcp": {name: {command: [...], environment} or {url, headers}}`.
    OpenCode,
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
    /// `$KEY` or `${KEY}`, written `${KEY}`: Gemini CLI.
    Shell,
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
            Syntax::Literal => None,
        }
    }
}

/// Splits `text` into text and references to environment variables written in `syntax`.
/// Anything else, such as `${KEY:-default}`, stays text.
pub(super) fn parse_value(text: &str, syntax: Syntax) -> Vec<Part> {
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
    let dollar = || {
        text.match_indices('$')
            .map(|(index, _)| index)
            .find(|&index| text[index + 1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
    };
    let (start, closed) = match (marked, syntax) {
        (Some(start), _) => (start, true),
        (None, Syntax::Shell) => (dollar()?, false),
        (None, _) => return None,
    };
    let rest = &text[start..];
    let length = if closed {
        rest.find('}').map_or(rest.len(), |end| end + 1)
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
        .flat_map(|agent| agent.files.iter().map(move |&relative| (agent, relative)))
        .filter_map(|(agent, relative)| {
            let path = root.join(relative);
            let mut config = McpConfig {
                harness: agent.harness,
                path,
                relative_path: PathBuf::from(relative),
                servers: Vec::new(),
                warnings: Vec::new(),
            };
            let read_instead = agent.read_instead.filter(|other| root.join(other).is_file());
            match (std::fs::read_to_string(&config.path), read_instead) {
                // Its servers aren't the agent's while it reads another file instead.
                (Ok(_), Some(other)) => config.warnings.push(format!(
                    "{} doesn't read this file while {other} exists; it reads {other} instead",
                    agent.name
                )),
                (Ok(text), None) => {
                    // A byte order mark, which editors on Windows often save, isn't JSON or TOML.
                    let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(&text);
                    match agent.format {
                        Format::McpServers => read_json(text, "mcpServers", mcp_servers_entry, &mut config),
                        Format::Codex => read_codex(text, &mut config),
                        Format::OpenCode => read_json(text, "mcp", opencode_entry, &mut config),
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
            Some(config)
        })
        .collect()
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
    // Copilot CLI's file can list its servers right at its top.
    let at_top = root.get_object(key).is_none()
        && agent(config.harness).servers_at_top
        && root
            .properties
            .iter()
            .any(|property| matches!(&property.value, Value::Object(entry) if is_server(entry)));
    let mut servers = match root.get_object(key) {
        Some(servers) => servers,
        None if at_top => &root,
        None => return,
    };
    // OpenCode's current layout keeps servers one level down, under `mcp.servers`. In the
    // older one they're right under `mcp`, so a server there can be called `servers`.
    if key == "mcp"
        && let Some(nested) = servers.get_object("servers")
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
    let transport = if let Some(command) = json_string(object, "command") {
        let args = object.get_array("args").map(json_strings).unwrap_or_default();
        Transport::Command { command, args }
    } else if let Some(url) = agent(harness)
        .remote
        .keys()
        .iter()
        .find_map(|key| json_string(object, key))
    {
        Transport::Remote { url }
    } else {
        return Err(no_transport(&name, agent(harness).remote.keys()));
    };
    let mut headers = json_settings(object.get_object("headers"), syntax);
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
        env: json_settings(object.get_object("env"), syntax),
        headers: sorted(headers),
        enabled: match agent(harness).off {
            Off::Disabled => json_bool(object, "disabled") != Some(true),
            Off::NotEnabled => json_bool(object, "enabled") != Some(false),
            Off::Never => true,
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
