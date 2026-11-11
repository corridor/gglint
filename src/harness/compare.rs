//! Which MCP servers each coding agent's config file is missing.
//!
//! Only the agents that have an MCP config file in the repository are compared. A server
//! set up in one of those files and missing from another is reported; an agent without a
//! config file isn't, since its users may not want MCP servers.
//!
//! Servers are matched by name, then by what they run, so a server under another name in
//! one file is still the same server. Files can start a server differently and still run
//! the same thing: `npx -y pkg`, `pnpm dlx pkg` and `bunx pkg` all run the npm package
//! `pkg`, and `npx mcp-remote <url>` reaches the server at `<url>`. How each file sets a
//! server up (its version, arguments, environment or headers) isn't compared: those
//! differences are often deliberate.

use std::path::{Path, PathBuf};

use super::Location;
use super::mcp::{Syntax, translate};
use super::{Harness, McpConfig, McpServer, Transport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// A server set up in some of the repository's MCP config files, but not all.
    MissingServer,
}

impl Rule {
    pub fn as_str(self) -> &'static str {
        match self {
            Rule::MissingServer => "missing-server",
        }
    }
}

/// Where a server is set up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub harness: Harness,
    pub path: PathBuf,
    /// `path` relative to the repository's root.
    pub relative_path: PathBuf,
    pub location: Location,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: Rule,
    /// The setup the server is copied from to fix it.
    pub site: Site,
    /// What's wrong, naming the server, for reading on its own.
    pub message: String,
    /// What's wrong, for reading under the server's name.
    pub detail: String,
    /// How to fix it.
    pub fix: String,
    /// The server's other setups.
    pub related: Vec<Site>,
}

/// Whether a server is in one agent's config file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Present,
    Missing,
    /// There, and turned off.
    Off,
}

/// One server across the repository's MCP config files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerCheck {
    /// Its name in the first file that has it.
    pub name: String,
    /// Each agent with a config file, in the order agents are reported, and whether the
    /// server is in it.
    pub cells: Vec<(Harness, Cell)>,
    pub findings: Vec<Finding>,
    /// The setup a file missing the server gets: the first one that's turned on. `None`
    /// when the server is turned off everywhere it's set up, which isn't a problem.
    pub source: Option<Setup>,
}

/// One agent's setup of a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    pub site: Site,
    pub server: McpServer,
}

/// Checks each server in `configs` across the agents that have a config file.
pub fn check_mcp(configs: &[McpConfig]) -> Vec<ServerCheck> {
    let mut harnesses: Vec<Harness> = configs.iter().map(|config| config.harness).collect();
    harnesses.dedup();
    group_servers(configs)
        .iter()
        .map(|group| group.check(&harnesses, configs))
        .collect()
}

/// The findings of `check_mcp`, server by server.
pub fn compare_mcp(configs: &[McpConfig]) -> Vec<Finding> {
    check_mcp(configs)
        .into_iter()
        .flat_map(|check| check.findings)
        .collect()
}

// ---------------------------------------------------------------------------
// What a server runs
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Registry {
    Npm,
    PyPI,
}

impl Registry {
    fn as_str(self) -> &'static str {
        match self {
            Registry::Npm => "npm",
            Registry::PyPI => "PyPI",
        }
    }
}

/// What a server runs, whatever starts it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Launch {
    /// A package that a runner downloads and starts, such as `npx -y pkg@1.0.0 --flag`.
    /// `version` is `None` when it's the newest, written as no version or `latest`.
    Package {
        registry: Registry,
        name: String,
        version: Option<String>,
        args: Vec<String>,
    },
    /// Any other command.
    Program {
        command: String,
        args: Vec<String>,
    },
    Remote {
        url: String,
    },
}

/// Commands that download a package and run it, with the words that come before the
/// package (`pnpm dlx pkg`).
const RUNNERS: &[(&str, &[&str], Registry)] = &[
    ("npx", &[], Registry::Npm),
    ("pnpx", &[], Registry::Npm),
    ("bunx", &[], Registry::Npm),
    ("pnpm", &["dlx"], Registry::Npm),
    ("yarn", &["dlx"], Registry::Npm),
    ("bun", &["x"], Registry::Npm),
    ("uvx", &[], Registry::PyPI),
    ("uv", &["tool", "run"], Registry::PyPI),
    ("pipx", &["run"], Registry::PyPI),
];

/// Runner options followed by a value, as in `--python 3.12`.
const OPTIONS_WITH_VALUES: &[&str] = &[
    "--package",
    "-p",
    "--from",
    "--spec",
    "--with",
    "--python",
    "--index",
    "--index-url",
    "--registry",
    "--cache",
];

/// Runner options that name the package, as in `uvx --from pkg command`. The word after
/// the options is then the command to run from the package.
const PACKAGE_OPTIONS: &[&str] = &["--package", "-p", "--from", "--spec"];

impl Launch {
    /// What `transport`, as `harness` writes it, runs. References to the environment are
    /// rewritten in one syntax, so `${ROOT}` and Cursor's `${env:ROOT}` are the same argument.
    fn of(transport: &Transport, harness: Harness) -> Launch {
        let from = Syntax::of(harness);
        let common = |text: &String| translate(text, from, Syntax::Dollar).unwrap_or_else(|| text.clone());
        match transport {
            Transport::Command { command, args } => {
                let (command, args): (String, Vec<String>) = (common(command), args.iter().map(common).collect());
                match package(&command, &args) {
                    // `npx mcp-remote <url>` runs a local proxy to the server at `<url>`.
                    Some(Launch::Package {
                        registry: Registry::Npm,
                        name,
                        args,
                        ..
                    }) if name == "mcp-remote" && args.first().is_some_and(|url| is_url(url)) => Launch::Remote {
                        url: args[0].clone(),
                    },
                    Some(launch) => launch,
                    None => Launch::Program { command, args },
                }
            }
            Transport::Remote { url } => Launch::Remote { url: common(url) },
        }
    }

    /// What it runs, for telling two servers apart: the package, the program or the URL.
    fn identity(&self) -> String {
        match self {
            Launch::Package { registry, name, .. } => format!("the {} package `{name}`", registry.as_str()),
            Launch::Program { command, .. } => format!("`{command}`"),
            Launch::Remote { url } => url.clone(),
        }
    }

    /// What it runs, arguments included, for matching a server under another name.
    fn rename_key(&self) -> String {
        match self {
            Launch::Program { command, args } => format!("`{command} {}`", args.join(" ")),
            _ => self.identity(),
        }
    }
}

fn is_url(text: &str) -> bool {
    text.starts_with("https://") || text.starts_with("http://")
}

/// The package `command args` downloads and runs, if `command` is a runner.
fn package(command: &str, args: &[String]) -> Option<Launch> {
    let program = Path::new(command).file_name()?.to_str()?;
    let program = program.trim_end_matches(".cmd").trim_end_matches(".exe");
    let &(_, before, registry) = RUNNERS
        .iter()
        .find(|(runner, before, _)| *runner == program && args.starts_with_words(before))?;

    let mut words = args[before.len()..].iter();
    let mut from_option = None;
    let first = loop {
        let word = words.next()?;
        if !word.starts_with('-') {
            break word;
        }
        let (option, value) = match word.split_once('=') {
            Some((option, value)) => (option, Some(value.to_string())),
            None if OPTIONS_WITH_VALUES.contains(&word.as_str()) => (word.as_str(), words.next().cloned()),
            None => (word.as_str(), None),
        };
        if PACKAGE_OPTIONS.contains(&option) {
            from_option = value;
        }
    };
    let (spec, mut rest): (&str, Vec<String>) = match &from_option {
        Some(package) => (package, std::iter::once(first).chain(words).cloned().collect()),
        None => (first, words.cloned().collect()),
    };
    let (name, version) = split_version(spec, registry);
    // `uvx --from pkg pkg` names the package's own command: the same as `uvx pkg`.
    if from_option.is_some() && rest.first() == Some(&name) {
        rest.remove(0);
    }
    Some(Launch::Package {
        registry,
        name,
        version,
        args: rest,
    })
}

trait StartsWithWords {
    fn starts_with_words(&self, words: &[&str]) -> bool;
}

impl StartsWithWords for [String] {
    fn starts_with_words(&self, words: &[&str]) -> bool {
        self.len() >= words.len() && self.iter().zip(words).all(|(arg, word)| arg == word)
    }
}

/// Splits `pkg@1.0.0` (npm and uv) or `pkg==1.0.0` (PyPI) into the package's name and its
/// version. PyPI names are compared as PyPI does: case and `_` or `-` don't matter, and
/// extras such as `pkg[cli]` are left out.
fn split_version(spec: &str, registry: Registry) -> (String, Option<String>) {
    let (name, version) = match registry {
        // A scoped package's name starts with `@`, as in `@angular/cli@19`.
        Registry::Npm => match spec.get(1..).and_then(|rest| rest.find('@')) {
            Some(at) => (&spec[..at + 1], Some(&spec[at + 2..])),
            None => (spec, None),
        },
        Registry::PyPI => match spec
            .find("==")
            .map(|at| (at, 2))
            .or_else(|| spec.find('@').map(|at| (at, 1)))
        {
            Some((at, width)) => (&spec[..at], Some(&spec[at + width..])),
            None => (spec, None),
        },
    };
    let name = match registry {
        Registry::Npm => name.to_string(),
        Registry::PyPI => {
            let name = name.split('[').next().unwrap_or(name);
            name.to_lowercase().replace('_', "-")
        }
    };
    let version = version.filter(|version| !version.is_empty() && *version != "latest");
    (name, version.map(str::to_string))
}

// ---------------------------------------------------------------------------
// Comparing
// ---------------------------------------------------------------------------

/// One agent's setup of a server.
struct Entry<'a> {
    site: Site,
    server: &'a McpServer,
    launch: Launch,
}

/// One server's setups, at most one per agent, in the order agents are reported.
struct Group<'a> {
    entries: Vec<Entry<'a>>,
}

fn group_servers(configs: &[McpConfig]) -> Vec<Group<'_>> {
    let mut groups: Vec<Group> = Vec::new();
    for config in configs {
        for server in &config.servers {
            let entry = Entry {
                site: Site {
                    harness: config.harness,
                    path: config.path.clone(),
                    relative_path: config.relative_path.clone(),
                    location: server.location,
                },
                server,
                launch: Launch::of(&server.transport, config.harness),
            };
            match groups.iter_mut().find(|group| group.name() == server.name) {
                // An agent reading two files (OpenCode's `.json` and `.jsonc`) keeps the first.
                Some(group) if group.has(config.harness) => {}
                Some(group) => group.entries.push(entry),
                None => groups.push(Group { entries: vec![entry] }),
            }
        }
    }

    // A server under another name in files that don't have it under this one.
    let mut keys: Vec<String> = groups
        .iter()
        .map(|group| group.entries[0].launch.rename_key())
        .collect();
    let mut index = 0;
    while index < groups.len() {
        let renamed = (index + 1..groups.len()).find(|&other| {
            keys[other] == keys[index]
                && groups[other]
                    .entries
                    .iter()
                    .all(|entry| !groups[index].has(entry.site.harness))
        });
        match renamed {
            Some(other) => {
                keys.remove(other);
                let merged = groups.remove(other);
                groups[index].entries.extend(merged.entries);
                groups[index].entries.sort_by_key(|entry| entry.site.harness);
            }
            None => index += 1,
        }
    }
    groups
}

impl Group<'_> {
    fn name(&self) -> &str {
        &self.entries[0].server.name
    }

    fn has(&self, harness: Harness) -> bool {
        self.entries.iter().any(|entry| entry.site.harness == harness)
    }

    fn entry(&self, harness: Harness) -> Option<&Entry<'_>> {
        self.entries.iter().find(|entry| entry.site.harness == harness)
    }

    fn check(&self, harnesses: &[Harness], configs: &[McpConfig]) -> ServerCheck {
        let name = self.name();
        // A server turned off may be off on purpose, so only one that's on is copied.
        let source = self.entries.iter().find(|entry| entry.server.enabled);
        let present: Vec<Harness> = self.entries.iter().map(|entry| entry.site.harness).collect();
        let missing: Vec<Harness> = harnesses
            .iter()
            .copied()
            .filter(|harness| !present.contains(harness))
            .collect();

        let mut findings = Vec::new();
        if let Some(source) = source
            && !missing.is_empty()
        {
            let files: Vec<String> = missing.iter().map(|&harness| config_file(harness, configs)).collect();
            let with_files: Vec<String> = missing
                .iter()
                .zip(&files)
                .map(|(harness, file)| format!("{} ({file})", harness.display_name()))
                .collect();
            findings.push(Finding {
                rule: Rule::MissingServer,
                site: source.site.clone(),
                message: format!(
                    "`{name}` is set up for {} but not for {}",
                    names(&present, "and"),
                    join(&with_files, "or")
                ),
                detail: format!("Missing from {}.", names(&missing, "and")),
                fix: format!(
                    "Add it to {}, as set up in {}:{}",
                    join(&files, "and"),
                    source.site.relative_path.display(),
                    source.site.location.line
                ),
                related: self
                    .entries
                    .iter()
                    .filter(|entry| entry.site.harness != source.site.harness)
                    .map(|entry| entry.site.clone())
                    .collect(),
            });
        }

        let cells = harnesses
            .iter()
            .map(|&harness| {
                let cell = match self.entry(harness) {
                    None => Cell::Missing,
                    Some(entry) if !entry.server.enabled => Cell::Off,
                    Some(_) => Cell::Present,
                };
                (harness, cell)
            })
            .collect();
        ServerCheck {
            name: name.to_string(),
            cells,
            findings,
            source: source.map(|entry| Setup {
                site: entry.site.clone(),
                server: entry.server.clone(),
            }),
        }
    }
}

/// The config file `harness` has in the repository.
fn config_file(harness: Harness, configs: &[McpConfig]) -> String {
    configs
        .iter()
        .find(|config| config.harness == harness)
        .map_or_else(String::new, |config| config.relative_path.display().to_string())
}

fn names(harnesses: &[Harness], last: &str) -> String {
    let names: Vec<String> = harnesses
        .iter()
        .map(|harness| harness.display_name().to_string())
        .collect();
    join(&names, last)
}

/// `a`, `a and b`, `a, b and c`, with `last` before the last item.
fn join(items: &[String], last: &str) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., final_item] => format!("{} {last} {final_item}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(command: &str, args: &[&str]) -> Launch {
        Launch::of(
            &Transport::Command {
                command: command.to_string(),
                args: args.iter().map(|arg| arg.to_string()).collect(),
            },
            Harness::ClaudeCode,
        )
    }

    fn package(registry: Registry, name: &str, version: Option<&str>, args: &[&str]) -> Launch {
        Launch::Package {
            registry,
            name: name.to_string(),
            version: version.map(str::to_string),
            args: args.iter().map(|arg| arg.to_string()).collect(),
        }
    }

    #[test]
    fn reads_npm_runners() {
        let expected = package(Registry::Npm, "@angular/cli", None, &["mcp"]);
        assert_eq!(launch("npx", &["-y", "@angular/cli", "mcp"]), expected);
        assert_eq!(launch("npx", &["--yes", "@angular/cli@latest", "mcp"]), expected);
        assert_eq!(launch("pnpm", &["dlx", "@angular/cli", "mcp"]), expected);
        assert_eq!(launch("bunx", &["@angular/cli", "mcp"]), expected);
        assert_eq!(launch("bun", &["x", "@angular/cli", "mcp"]), expected);
        assert_eq!(launch("/usr/local/bin/npx", &["@angular/cli", "mcp"]), expected);
        assert_eq!(launch("npx.cmd", &["-y", "@angular/cli", "mcp"]), expected);
    }

    #[test]
    fn reads_npm_versions() {
        assert_eq!(
            launch("npx", &["-y", "@angular/cli@19.2.0", "mcp"]),
            package(Registry::Npm, "@angular/cli", Some("19.2.0"), &["mcp"])
        );
        assert_eq!(
            launch("npx", &["chrome-devtools-mcp@0.6", "--no-usage-statistics"]),
            package(
                Registry::Npm,
                "chrome-devtools-mcp",
                Some("0.6"),
                &["--no-usage-statistics"]
            )
        );
    }

    #[test]
    fn reads_a_package_named_by_an_option() {
        assert_eq!(
            launch("npx", &["--package=@acme/tools@2", "acme-mcp", "--port", "1"]),
            package(Registry::Npm, "@acme/tools", Some("2"), &["acme-mcp", "--port", "1"])
        );
        assert_eq!(
            launch("uvx", &["--from", "code-review-graph", "code-review-graph", "serve"]),
            package(Registry::PyPI, "code-review-graph", None, &["serve"])
        );
    }

    #[test]
    fn reads_python_runners() {
        let expected = package(Registry::PyPI, "mcp-server-git", Some("1.2"), &["--repository", "."]);
        assert_eq!(launch("uvx", &["mcp-server-git==1.2", "--repository", "."]), expected);
        assert_eq!(
            launch("uvx", &["--python", "3.12", "mcp-server-git@1.2", "--repository", "."]),
            expected
        );
        assert_eq!(
            launch("uv", &["tool", "run", "MCP_Server_Git[cli]==1.2", "--repository", "."]),
            expected
        );
        assert_eq!(
            launch("uvx", &["code-review-graph@latest", "serve"]),
            package(Registry::PyPI, "code-review-graph", None, &["serve"])
        );
    }

    #[test]
    fn other_commands_are_programs() {
        assert_eq!(
            launch("docker", &["run", "-i", "--rm", "mcp/git"]),
            Launch::Program {
                command: "docker".to_string(),
                args: vec![
                    "run".to_string(),
                    "-i".to_string(),
                    "--rm".to_string(),
                    "mcp/git".to_string()
                ],
            }
        );
        // `pnpm` without `dlx` runs a script, not a package.
        assert!(matches!(launch("pnpm", &["run", "mcp"]), Launch::Program { .. }));
        // A runner with only options has no package.
        assert!(matches!(launch("npx", &["--version"]), Launch::Program { .. }));
    }

    #[test]
    fn joins_lists_as_prose() {
        let items = |items: &[&str]| items.iter().map(|item| item.to_string()).collect::<Vec<_>>();
        assert_eq!(join(&items(&["a"]), "and"), "a");
        assert_eq!(join(&items(&["a", "b"]), "or"), "a or b");
        assert_eq!(join(&items(&["a", "b", "c"]), "and"), "a, b and c");
    }
}
