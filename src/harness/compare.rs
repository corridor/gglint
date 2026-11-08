//! Whether every harness a repository is set up for gets the same MCP servers, set up the
//! same way.
//!
//! Servers are matched by name, then by what they run, so a server renamed in one harness
//! is still the same server. Harnesses can start a server differently and still run the
//! same thing: `npx -y pkg`, `pnpm dlx pkg` and `bunx pkg` all run the npm package `pkg`,
//! so that isn't reported. What changes what the agent can do is: a server missing from a
//! harness, another program or version, other arguments, other environment variables or
//! headers, and a server turned off.

use std::path::{Path, PathBuf};

use super::{Harness, McpConfig, McpServer, Transport};
use crate::discovery::Location;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// A server set up for some of the harnesses the repository uses, but not all.
    MissingServer,
    /// A server set up differently in two harnesses.
    DifferentServer,
    /// A server with a different name in two harnesses.
    RenamedServer,
}

impl Rule {
    pub fn as_str(self) -> &'static str {
        match self {
            Rule::MissingServer => "missing-server",
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
    /// The first harness's setup of the server.
    pub site: Site,
    pub message: String,
    /// The other harnesses' setups of the server.
    pub related: Vec<Site>,
}

/// Compares the servers in `configs` across `harnesses`, the harnesses the repository is
/// set up for. A server missing from a harness that isn't one of them isn't reported.
pub fn compare_mcp(configs: &[McpConfig], harnesses: &[Harness]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for group in group_servers(configs) {
        group.compare(harnesses, &mut findings);
    }
    findings
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
    fn of(transport: &Transport) -> Launch {
        match transport {
            Transport::Command { command, args } => package(command, args).unwrap_or_else(|| Launch::Program {
                command: command.clone(),
                args: args.clone(),
            }),
            Transport::Remote { url } => Launch::Remote { url: url.clone() },
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

/// One harness's setup of a server.
struct Entry<'a> {
    site: Site,
    server: &'a McpServer,
    launch: Launch,
}

/// One server's setups, at most one per harness, in the order harnesses are reported.
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
                launch: Launch::of(&server.transport),
            };
            match groups.iter_mut().find(|group| group.name() == server.name) {
                // A harness reading two files (OpenCode's `.json` and `.jsonc`) keeps the first.
                Some(group) if group.has(config.harness) => {}
                Some(group) => group.entries.push(entry),
                None => groups.push(Group { entries: vec![entry] }),
            }
        }
    }

    // A server under another name in harnesses that don't have it under this one.
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

    fn finding(&self, rule: Rule, message: String) -> Finding {
        self.finding_at(self.entries[0].site.harness, rule, message)
    }

    /// A finding at `harness`'s setup of the server, with the others as related.
    fn finding_at(&self, harness: Harness, rule: Rule, message: String) -> Finding {
        let (here, others): (Vec<&Entry>, Vec<&Entry>) =
            self.entries.iter().partition(|entry| entry.site.harness == harness);
        Finding {
            rule,
            site: here[0].site.clone(),
            message,
            related: others.iter().map(|entry| entry.site.clone()).collect(),
        }
    }

    /// Each value of `value` with the harnesses that have it, as `a in X; b in Y and Z`,
    /// if the harnesses don't all agree.
    fn disagreement<T: PartialEq>(&self, value: impl Fn(&Entry) -> T, show: impl Fn(&T) -> String) -> Option<String> {
        let mut values: Vec<(T, Vec<Harness>)> = Vec::new();
        for entry in &self.entries {
            let this = value(entry);
            match values.iter_mut().find(|(other, _)| *other == this) {
                Some((_, harnesses)) => harnesses.push(entry.site.harness),
                None => values.push((this, vec![entry.site.harness])),
            }
        }
        (values.len() > 1).then(|| {
            values
                .iter()
                .map(|(value, harnesses)| format!("{} in {}", show(value), names(harnesses, "and")))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    fn compare(&self, harnesses: &[Harness], findings: &mut Vec<Finding>) {
        let name = self.name();

        if let Some(names) = self.disagreement(|entry| entry.server.name.clone(), |name| format!("`{name}`")) {
            findings.push(self.finding(Rule::RenamedServer, format!("One server has different names: {names}")));
        }

        let present: Vec<Harness> = self.entries.iter().map(|entry| entry.site.harness).collect();
        let missing: Vec<String> = harnesses
            .iter()
            .filter(|harness| !present.contains(harness))
            .map(|harness| format!("{} ({})", harness.display_name(), harness.mcp_file()))
            .collect();
        if !missing.is_empty() {
            findings.push(self.finding(
                Rule::MissingServer,
                format!(
                    "`{name}` is set up for {} but not for {}",
                    names(&present, "and"),
                    join(&missing, "or")
                ),
            ));
        }

        let mut different = |message: String| findings.push(self.finding(Rule::DifferentServer, message));
        if let Some(servers) = self.disagreement(|entry| entry.launch.identity(), String::clone) {
            // Versions and arguments of different programs can't be compared.
            different(format!("`{name}` is a different server in each harness: {servers}"));
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
                if let Some(versions) = self.disagreement(version, show) {
                    different(format!(
                        "`{name}` runs different versions of the {} package `{package}`: {versions}",
                        registry.as_str()
                    ));
                }
            }
            let args = |entry: &Entry| entry.launch.args().map(<[String]>::to_vec);
            let show = |args: &Option<Vec<String>>| match args {
                Some(args) if !args.is_empty() => format!("`{}`", args.join(" ")),
                _ => "no arguments".to_string(),
            };
            if let Some(args) = self.disagreement(args, show) {
                different(format!("`{name}` is started with different arguments: {args}"));
            }
        }

        // Names only: values are written differently in each harness (`${KEY}`,
        // `{env:KEY}`), and can be secrets.
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
        if let Some(env) = self.disagreement(env, show) {
            different(format!("`{name}` is given different environment variables: {env}"));
        }
        let headers = |entry: &Entry| {
            entry
                .server
                .headers
                .iter()
                .map(|setting| setting.name.clone())
                .collect::<Vec<_>>()
        };
        if let Some(headers) = self.disagreement(headers, show) {
            different(format!("`{name}` is sent different HTTP headers: {headers}"));
        }

        let (on, off): (Vec<&Entry>, Vec<&Entry>) = self.entries.iter().partition(|entry| entry.server.enabled);
        if !on.is_empty() && !off.is_empty() {
            let harnesses = |entries: &[&Entry]| entries.iter().map(|entry| entry.site.harness).collect::<Vec<_>>();
            // At the first harness it's turned off in: that's the setup to look at.
            findings.push(self.finding_at(
                off[0].site.harness,
                Rule::DifferentServer,
                format!(
                    "`{name}` is turned off in {} but not in {}",
                    names(&harnesses(&off), "and"),
                    names(&harnesses(&on), "or")
                ),
            ));
        }
    }
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
        Launch::of(&Transport::Command {
            command: command.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
        })
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
