//! What every check reports, and how it's printed. A check turns what it finds into
//! `Diagnostic`s, so the one-line format, the colours and the exit status are the same for
//! `gglint mcp`, `gglint secrets` and the checks after them.

use std::io::IsTerminal;
use std::path::PathBuf;

use crate::harness::{Finding, Location};

/// How serious a diagnostic is. Both make `gglint` exit with status 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// One problem found in one place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The rule's name, such as `missing-server` or `inline-secret`.
    pub rule: &'static str,
    pub severity: Severity,
    /// The file, as it can be opened from the current directory.
    pub path: PathBuf,
    /// `path` relative to the repository's root.
    pub relative_path: PathBuf,
    pub location: Location,
    /// What's wrong, for reading on its own. Never contains a secret's value.
    pub message: String,
    /// How to fix it, when gglint can.
    pub fix: Option<String>,
    /// Other places the same problem shows up.
    pub related: Vec<(PathBuf, Location)>,
}

impl From<&Finding> for Diagnostic {
    fn from(finding: &Finding) -> Self {
        Diagnostic {
            rule: finding.rule.as_str(),
            severity: Severity::Error,
            path: finding.site.path.clone(),
            relative_path: finding.site.relative_path.clone(),
            location: finding.site.location,
            message: finding.message.clone(),
            fix: finding.fix.clone(),
            related: finding
                .related
                .iter()
                .map(|site| (site.path.clone(), site.location))
                .collect(),
        }
    }
}

impl Diagnostic {
    /// The `--format lines` form: where it is, the rule, a description, then the other places.
    pub fn line(&self) -> String {
        let mut out = format!(
            "{}:{}  {:<16} {}\n",
            self.path.display(),
            self.location,
            self.rule,
            self.message
        );
        for (path, location) in &self.related {
            out.push_str(&format!("    also at {}:{}\n", path.display(), location));
        }
        out
    }
}

/// ANSI colours, when standard output is a terminal and `NO_COLOR` isn't set.
pub struct Paint {
    on: bool,
}

impl Paint {
    pub fn new() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
        Self {
            on: std::io::stdout().is_terminal() && !no_color,
        }
    }

    pub fn paint(&self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }

    pub fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }

    pub fn red(&self, text: &str) -> String {
        self.paint("31", text)
    }

    pub fn yellow(&self, text: &str) -> String {
        self.paint("33", text)
    }

    pub fn green(&self, text: &str) -> String {
        self.paint("32", text)
    }
}

impl Default for Paint {
    fn default() -> Self {
        Self::new()
    }
}

/// The exit status for a run: 1 when anything was found.
pub fn exit_code(diagnostics: &[Diagnostic]) -> std::process::ExitCode {
    if diagnostics.is_empty() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
