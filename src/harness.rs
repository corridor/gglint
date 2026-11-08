//! The coding harnesses' config files in a repository: the MCP servers that Claude Code,
//! Codex, Cursor and OpenCode are each set up to use.
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

use crate::discovery::Location;

/// A program that runs a model as a coding agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Harness {
    ClaudeCode,
    Codex,
    Cursor,
    OpenCode,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Codex => "codex",
            Harness::Cursor => "cursor",
            Harness::OpenCode => "opencode",
        }
    }
}

/// How a file lists its MCP servers.
#[derive(Clone, Copy)]
enum Format {
    /// JSON: `"mcpServers": {name: {command, args, env} or {url, headers}}`.
    McpServers,
    /// TOML: `[mcp_servers.<name>]` tables.
    Codex,
    /// JSON with comments: `"mcp": {name: {command: [...], environment} or {url, headers}}`.
    OpenCode,
}

/// Where each harness reads a repository's MCP servers from, relative to its root, in the
/// order they're reported. Codex only reads `.codex/config.toml` in projects the user
/// has trusted.
const MCP_FILES: &[(Harness, &str, Format)] = &[
    (Harness::ClaudeCode, ".mcp.json", Format::McpServers),
    (Harness::Codex, ".codex/config.toml", Format::Codex),
    (Harness::Cursor, ".cursor/mcp.json", Format::McpServers),
    (Harness::OpenCode, "opencode.json", Format::OpenCode),
    (Harness::OpenCode, "opencode.jsonc", Format::OpenCode),
];

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
/// Values are kept as written, references such as `${KEY}` included. A config can hold a
/// secret written out in full, so don't print them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
    pub name: String,
    /// `None` when the value is taken from the user's environment by name, as with Codex's
    /// `env_vars` and `bearer_token_env_var`.
    pub value: Option<String>,
}

/// Reads every harness's MCP config file found under `root`.
pub fn read_mcp(root: &Path) -> Vec<McpConfig> {
    MCP_FILES
        .iter()
        .filter_map(|&(harness, relative, format)| {
            let path = root.join(relative);
            let mut config = McpConfig {
                harness,
                path,
                relative_path: PathBuf::from(relative),
                servers: Vec::new(),
                warnings: Vec::new(),
            };
            match std::fs::read_to_string(&config.path) {
                Ok(text) => match format {
                    Format::McpServers => read_json(&text, "mcpServers", mcp_servers_entry, &mut config),
                    Format::Codex => read_codex(&text, &mut config),
                    Format::OpenCode => read_json(&text, "mcp", opencode_entry, &mut config),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
                Err(error) => config.warnings.push(format!("couldn't read: {error}")),
            }
            Some(config)
        })
        .collect()
}

fn no_transport(name: &str) -> String {
    format!("`{name}` has neither a `command` nor a `url`")
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
// JSON: Claude Code, Cursor and OpenCode
// ---------------------------------------------------------------------------

/// Reads the servers in the object at `key`, one entry at a time with `entry`.
fn read_json(
    text: &str,
    key: &str,
    entry: fn(String, Location, &Object) -> Result<McpServer, String>,
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
    let Some(servers) = root.get_object(key) else { return };
    for property in &servers.properties {
        let name = property.name.as_str().to_string();
        let location = location(text, property.range.start);
        let Value::Object(object) = &property.value else {
            config.warnings.push(format!("`{name}` isn't an object"));
            continue;
        };
        match entry(name, location, object) {
            Ok(server) => config.servers.push(server),
            Err(warning) => config.warnings.push(warning),
        }
    }
}

fn json_string(object: &Object, key: &str) -> Option<String> {
    object.get_string(key).map(|string| string.value.to_string())
}

fn json_strings(array: &Array) -> Vec<String> {
    array
        .elements
        .iter()
        .filter_map(|element| element.as_string_lit().map(|string| string.value.to_string()))
        .collect()
}

fn json_settings(object: Option<&Object>) -> Vec<Setting> {
    let Some(object) = object else { return Vec::new() };
    sorted(
        object
            .properties
            .iter()
            .filter_map(|property| {
                let value = property.value.as_string_lit()?;
                Some(Setting {
                    name: property.name.as_str().to_string(),
                    value: Some(value.value.to_string()),
                })
            })
            .collect(),
    )
}

/// Claude Code's `.mcp.json` and Cursor's `.cursor/mcp.json`. A server is started with
/// `command` or reached at `url`, whatever its `type` says.
fn mcp_servers_entry(name: String, location: Location, object: &Object) -> Result<McpServer, String> {
    let transport = if let Some(command) = json_string(object, "command") {
        let args = object.get_array("args").map(json_strings).unwrap_or_default();
        Transport::Command { command, args }
    } else if let Some(url) = json_string(object, "url") {
        Transport::Remote { url }
    } else {
        return Err(no_transport(&name));
    };
    Ok(McpServer {
        name,
        location,
        transport,
        env: json_settings(object.get_object("env")),
        headers: json_settings(object.get_object("headers")),
        enabled: true,
        enabled_tools: None,
        disabled_tools: Vec::new(),
    })
}

/// OpenCode's `opencode.json`: `command` is one list, the program and its arguments.
fn opencode_entry(name: String, location: Location, object: &Object) -> Result<McpServer, String> {
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
        return Err(no_transport(&name));
    };
    let enabled = match object.get("enabled").map(|property| &property.value) {
        Some(Value::BooleanLit(enabled)) => enabled.value,
        _ => true,
    };
    Ok(McpServer {
        name,
        location,
        transport,
        env: json_settings(object.get_object("environment")),
        headers: json_settings(object.get_object("headers")),
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

/// The string values of the table at `key`, as settings.
fn toml_settings(table: &DeTable<'_>, key: &str) -> Vec<Setting> {
    let Some(DeValue::Table(settings)) = toml_get(table, key) else {
        return Vec::new();
    };
    settings
        .iter()
        .filter_map(|(name, value)| match value.get_ref() {
            DeValue::String(value) => Some(Setting {
                name: name.get_ref().to_string(),
                value: Some(value.to_string()),
            }),
            _ => None,
        })
        .collect()
}

/// Settings whose values come from the user's environment by name.
fn from_environment(names: impl IntoIterator<Item = String>) -> impl Iterator<Item = Setting> {
    names.into_iter().map(|name| Setting { name, value: None })
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
            config.warnings.push(no_transport(&name));
            continue;
        };

        let mut env = toml_settings(table, "env");
        env.extend(from_environment(toml_strings(table, "env_vars").unwrap_or_default()));
        let mut headers = toml_settings(table, "http_headers");
        // `env_http_headers` maps each header to the variable its value is read from.
        headers.extend(from_environment(
            toml_settings(table, "env_http_headers")
                .into_iter()
                .map(|setting| setting.name),
        ));
        if toml_string(table, "bearer_token_env_var").is_some() {
            headers.extend(from_environment(["Authorization".to_string()]));
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
