//! How the MCP setups of the coding agents in a repository differ.
//!
//! The agents compared are those with an MCP config file in the repository, or with their
//! own directory (`.claude/`, `.codex/`, `.cursor/`, `.opencode/`). Three kinds of problem
//! are reported:
//!
//! - a server set up in one config file and missing from another (`missing-server`);
//! - an agent with its own directory and no MCP config file, while other agents have servers
//!   (`missing-config`);
//! - a server set up differently in two files: another program, version, arguments,
//!   environment variables or headers, another name, or turned off in one
//!   (`different-server`, `renamed-server`).
//!
//! Servers are matched by name, then by what they run, so a server under another name in
//! one file is still the same server. Files can start a server differently and still run
//! the same thing: `npx -y pkg`, `pnpm dlx pkg` and `bunx pkg` all run the npm package
//! `pkg`, and `npx mcp-remote <url>` reaches the server at `<url>`, so that isn't reported.

use std::path::{Path, PathBuf};

use super::Location;
use super::mcp::{Syntax, translate};
use super::{HARNESSES, Harness, McpConfig, McpServer, Transport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// A server set up in some of the repository's MCP config files, but not all.
    MissingServer,
    /// An agent with its own directory in the repository and no MCP config file.
    MissingConfig,
    /// A server set up differently in two config files.
    DifferentServer,
    /// A server with a different name in two config files.
    RenamedServer,
}

impl Rule {
    pub fn as_str(self) -> &'static str {
        match self {
            Rule::MissingServer => "missing-server",
            Rule::MissingConfig => "missing-config",
            Rule::DifferentServer => "different-server",
            Rule::RenamedServer => "renamed-server",
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
    /// Where it's found: a setup of the server, or for `missing-config` the first setup of
    /// the servers the agent doesn't get.
    pub site: Site,
    /// What's wrong, naming the server or agent, for reading on its own.
    pub message: String,
    /// What's wrong, for reading under the server's name.
    pub detail: String,
    /// How to fix it, when gglint can.
    pub fix: Option<String>,
    /// The server's other setups.
    pub related: Vec<Site>,
}

/// How a server is set up in one agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Set up the same way as in the first file that has it.
    Same,
    /// Set up differently from the first file that has it.
    Different,
    Missing,
    /// Set up, and turned off.
    Off,
}

/// What `check_mcp` finds in a repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpCheck {
    /// The agents compared, in the order agents are reported: those with an MCP config file
    /// or their own directory.
    pub harnesses: Vec<Harness>,
    /// Each server, in each of those agents.
    pub servers: Vec<ServerCheck>,
    /// One `missing-config` finding for each agent with its own directory and no MCP config
    /// file, when other agents have servers it doesn't get.
    pub missing_configs: Vec<Finding>,
}

impl McpCheck {
    /// Every finding: server by server, then the missing config files.
    pub fn findings(&self) -> impl Iterator<Item = &Finding> {
        self.servers
            .iter()
            .flat_map(|check| &check.findings)
            .chain(&self.missing_configs)
    }
}

/// One server across the agents compared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerCheck {
    /// Its name in the first file that has it.
    pub name: String,
    /// Each agent compared, in the order agents are reported, with how the server is set
    /// up in it.
    pub cells: Vec<(Harness, Cell)>,
    pub findings: Vec<Finding>,
    /// The setup a file missing the server gets, and the one `--overwrite` copies: the
    /// first one that's turned on. `None` when the server is turned off everywhere it's set
    /// up, so it isn't missing anywhere.
    pub source: Option<Setup>,
    /// Every agent's setup of the server, in the order agents are reported.
    pub setups: Vec<Setup>,
}

/// One agent's setup of a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    pub site: Site,
    pub server: McpServer,
}

/// Whether `a` and `b` run the same package, program or URL, however they're started.
pub(super) fn same_program(a: &Setup, b: &Setup) -> bool {
    Launch::of(&a.server.transport, a.site.harness).identity()
        == Launch::of(&b.server.transport, b.site.harness).identity()
}

/// Whether `a` and `b` start the same version of the same program with the same arguments.
/// The runner doesn't count: `npx -y pkg` and `pnpm dlx pkg` start the same command line.
pub(super) fn same_command_line(a: &Setup, b: &Setup) -> bool {
    Launch::of(&a.server.transport, a.site.harness) == Launch::of(&b.server.transport, b.site.harness)
}

/// The agents compared in the repository at `root`, in the order agents are reported: those
/// with a config file in `configs`, or with their own directory. A `CLAUDE.md` or
/// `.cursorrules` alone says people use the agent, not that it's set up in the repository.
pub fn compared_harnesses(root: &Path, configs: &[McpConfig]) -> Vec<Harness> {
    HARNESSES
        .into_iter()
        .filter(|&harness| {
            configs.iter().any(|config| config.harness == harness) || root.join(harness.directory()).is_dir()
        })
        .collect()
}

/// Checks the MCP servers in `configs`, read from the repository at `root`.
pub fn check_mcp(root: &Path, configs: &[McpConfig]) -> McpCheck {
    let harnesses = compared_harnesses(root, configs);
    let servers: Vec<ServerCheck> = group_servers(configs)
        .iter()
        .map(|group| group.check(&harnesses, configs))
        .collect();
    let missing_configs = harnesses
        .iter()
        .filter(|&&harness| !configs.iter().any(|config| config.harness == harness))
        .filter_map(|&harness| missing_config(harness, &servers))
        .collect();
    McpCheck {
        harnesses,
        servers,
        missing_configs,
    }
}

/// Every finding of `check_mcp`.
pub fn compare_mcp(root: &Path, configs: &[McpConfig]) -> Vec<Finding> {
    check_mcp(root, configs).findings().cloned().collect()
}

/// The `missing-config` finding for `harness`, which has its own directory and no config
/// file, if other agents have servers turned on.
fn missing_config(harness: Harness, servers: &[ServerCheck]) -> Option<Finding> {
    let sources: Vec<&Setup> = servers.iter().filter_map(|check| check.source.as_ref()).collect();
    let first = sources.first()?;
    // The agents that have any of those servers.
    let from: Vec<Harness> = HARNESSES
        .into_iter()
        .filter(|&other| {
            servers.iter().filter(|check| check.source.is_some()).any(|check| {
                check
                    .cells
                    .iter()
                    .any(|&(harness, cell)| harness == other && cell != Cell::Missing)
            })
        })
        .collect();
    let (count, gets_none_of) = match sources.len() {
        1 => ("the MCP server".to_string(), "doesn't get the MCP server".to_string()),
        n => (
            format!("the {n} MCP servers"),
            format!("gets none of the {n} MCP servers"),
        ),
    };
    let (name, directory, file) = (harness.display_name(), harness.directory(), harness.mcp_file());
    Some(Finding {
        rule: Rule::MissingConfig,
        site: first.site.clone(),
        message: format!(
            "{name} has a {directory} directory but no {file}, so it {gets_none_of} set up for {}",
            names(&from, "and")
        ),
        detail: format!("{name} has a {directory} directory but no {file}."),
        fix: Some(format!(
            "`gglint mcp --fix --create` creates {file} with {count} set up for {}",
            names(&from, "and")
        )),
        related: Vec::new(),
    })
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
                    }) if name == "mcp-remote" && args.first().is_some_and(|url| is_url(url)) => {
                        Launch::Remote { url: args[0].clone() }
                    }
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

    fn args(&self) -> Option<&[String]> {
        match self {
            Launch::Package { args, .. } | Launch::Program { args, .. } => Some(args),
            Launch::Remote { .. } => None,
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

    /// A finding at `harness`'s setup of the server, with the others as related.
    fn finding_at(&self, harness: Harness, rule: Rule, message: String, detail: String) -> Finding {
        let (here, others): (Vec<&Entry>, Vec<&Entry>) =
            self.entries.iter().partition(|entry| entry.site.harness == harness);
        Finding {
            rule,
            site: here[0].site.clone(),
            message,
            detail,
            fix: None,
            related: others.iter().map(|entry| entry.site.clone()).collect(),
        }
    }

    /// A finding at the first file's setup of the server, with the others as related.
    fn finding(&self, rule: Rule, message: String, detail: String) -> Finding {
        self.finding_at(self.entries[0].site.harness, rule, message, detail)
    }

    /// Each value of `value` with the agents that have it, as `a in X; b in Y and Z`, if
    /// the agents don't all agree. The agents whose value isn't the first file's are added
    /// to `different`.
    fn disagreement<T: PartialEq>(
        &self,
        value: impl Fn(&Entry) -> T,
        show: impl Fn(&T) -> String,
        different: &mut Vec<Harness>,
    ) -> Option<String> {
        let mut values: Vec<(T, Vec<Harness>)> = Vec::new();
        for entry in &self.entries {
            let this = value(entry);
            match values.iter_mut().find(|(other, _)| *other == this) {
                Some((_, harnesses)) => harnesses.push(entry.site.harness),
                None => values.push((this, vec![entry.site.harness])),
            }
        }
        if values.len() < 2 {
            return None;
        }
        different.extend(values[1..].iter().flat_map(|(_, harnesses)| harnesses));
        Some(
            values
                .iter()
                .map(|(value, harnesses)| format!("{} in {}", show(value), names(harnesses, "and")))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    fn check(&self, harnesses: &[Harness], configs: &[McpConfig]) -> ServerCheck {
        let name = self.name();
        let mut findings = Vec::new();
        // The agents where the server isn't set up as in the first file.
        let mut different = Vec::new();

        if let Some(names) = self.disagreement(
            |entry| entry.server.name.clone(),
            |name| format!("`{name}`"),
            &mut different,
        ) {
            findings.push(self.finding(
                Rule::RenamedServer,
                format!("One server has different names: {names}"),
                format!("Named differently: {names}."),
            ));
        }

        // A server turned off may be off on purpose, so only one that's on is copied, and
        // one turned off everywhere isn't missing anywhere. An agent without a config file
        // gets its own `missing-config` finding instead.
        let source = self.entries.iter().find(|entry| entry.server.enabled);
        let present: Vec<Harness> = self.entries.iter().map(|entry| entry.site.harness).collect();
        let missing: Vec<Harness> = harnesses
            .iter()
            .copied()
            .filter(|harness| !present.contains(harness) && configs.iter().any(|config| config.harness == *harness))
            .collect();
        if let Some(source) = source
            && !missing.is_empty()
        {
            let files: Vec<String> = missing.iter().map(|&harness| config_file(harness, configs)).collect();
            let with_files: Vec<String> = missing
                .iter()
                .zip(&files)
                .map(|(harness, file)| format!("{} ({file})", harness.display_name()))
                .collect();
            let mut finding = self.finding_at(
                source.site.harness,
                Rule::MissingServer,
                format!(
                    "`{name}` is set up for {} but not for {}",
                    names(&present, "and"),
                    join(&with_files, "or")
                ),
                format!("Missing from {}.", names(&missing, "and")),
            );
            finding.fix = Some(format!(
                "Add it to {}, as set up in {}:{}",
                join(&files, "and"),
                source.site.relative_path.display(),
                source.site.location.line
            ));
            findings.push(finding);
        }

        // A different version or other arguments of the same program has one fix: start it
        // the same way everywhere, as the setup missing files get it from.
        let overwrite = source.map(|entry| {
            format!(
                "`gglint mcp --fix --overwrite` starts it everywhere as in {}:{}",
                entry.site.relative_path.display(),
                entry.site.location.line
            )
        });
        let mut differ = |message: String, detail: String, fix: Option<String>| {
            let mut finding = self.finding(Rule::DifferentServer, message, detail);
            finding.fix = fix;
            findings.push(finding);
        };
        if let Some(servers) = self.disagreement(|entry| entry.launch.identity(), String::clone, &mut different) {
            // Versions and arguments of different programs can't be compared.
            differ(
                format!("`{name}` is a different server in each file: {servers}"),
                format!("A different server in each file: {servers}."),
                None,
            );
        } else {
            if let Launch::Package {
                registry,
                name: package,
                ..
            } = &self.entries[0].launch
            {
                let version = |entry: &Entry| match &entry.launch {
                    Launch::Package { version, .. } => version.clone(),
                    _ => None,
                };
                let show = |version: &Option<String>| version.clone().unwrap_or_else(|| "the newest".to_string());
                if let Some(versions) = self.disagreement(version, show, &mut different) {
                    let registry = registry.as_str();
                    differ(
                        format!("`{name}` runs different versions of the {registry} package `{package}`: {versions}"),
                        format!("Runs different versions of the {registry} package `{package}`: {versions}."),
                        overwrite.clone(),
                    );
                }
            }
            let args = |entry: &Entry| entry.launch.args().map(<[String]>::to_vec);
            let show = |args: &Option<Vec<String>>| match args {
                Some(args) if !args.is_empty() => format!("`{}`", args.join(" ")),
                _ => "no arguments".to_string(),
            };
            if let Some(args) = self.disagreement(args, show, &mut different) {
                differ(
                    format!("`{name}` is started with different arguments: {args}"),
                    format!("Started with different arguments: {args}."),
                    overwrite.clone(),
                );
            }
        }

        // Names only: values are written differently in each agent (`${KEY}`, `{env:KEY}`),
        // and can be secrets.
        let show = |names: &Vec<String>| {
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        };
        let env = |entry: &Entry| {
            entry
                .server
                .env
                .iter()
                .map(|setting| setting.name.clone())
                .collect::<Vec<_>>()
        };
        if let Some(env) = self.disagreement(env, show, &mut different) {
            differ(
                format!("`{name}` is given different environment variables: {env}"),
                format!("Given different environment variables: {env}."),
                None,
            );
        }
        let headers = |entry: &Entry| {
            entry
                .server
                .headers
                .iter()
                .map(|setting| setting.name.clone())
                .collect::<Vec<_>>()
        };
        if let Some(headers) = self.disagreement(headers, show, &mut different) {
            differ(
                format!("`{name}` is sent different HTTP headers: {headers}"),
                format!("Sent different HTTP headers: {headers}."),
                None,
            );
        }

        let (on, off): (Vec<&Entry>, Vec<&Entry>) = self.entries.iter().partition(|entry| entry.server.enabled);
        if !on.is_empty() && !off.is_empty() {
            let harnesses = |entries: &[&Entry]| entries.iter().map(|entry| entry.site.harness).collect::<Vec<_>>();
            let (off_names, on_names) = (names(&harnesses(&off), "and"), names(&harnesses(&on), "or"));
            // At the first file it's turned off in: that's the setup to look at.
            findings.push(self.finding_at(
                off[0].site.harness,
                Rule::DifferentServer,
                format!("`{name}` is turned off in {off_names} but not in {on_names}"),
                format!("Turned off in {off_names} but not in {on_names}."),
            ));
        }

        let cells = harnesses
            .iter()
            .map(|&harness| {
                let cell = match self.entry(harness) {
                    None => Cell::Missing,
                    Some(entry) if !entry.server.enabled => Cell::Off,
                    Some(_) if different.contains(&harness) => Cell::Different,
                    Some(_) => Cell::Same,
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
            setups: self
                .entries
                .iter()
                .map(|entry| Setup {
                    site: entry.site.clone(),
                    server: entry.server.clone(),
                })
                .collect(),
        }
    }
}

/// The config file `harness` has in the repository.
fn config_file(harness: Harness, configs: &[McpConfig]) -> String {
    configs.iter().find(|config| config.harness == harness).map_or_else(
        || harness.mcp_file().to_string(),
        |config| config.relative_path.display().to_string(),
    )
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
