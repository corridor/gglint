//! What every check reports, and how it's printed. A check turns what it finds into
//! `Diagnostic`s, so the one-line format, the colours and the exit status are the same for
//! `gglint mcp`, `gglint secrets` and the checks after them.

use std::io::IsTerminal;
use std::path::PathBuf;

use crate::harness::{Finding, Location};

/// How serious a diagnostic is. Only an error makes `gglint` exit with status 1: a warning
/// is something to look at that can't leak yet, such as a secret in a file git ignores.
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
    /// A warning says so before its description.
    pub fn line(&self) -> String {
        let level = match self.severity {
            Severity::Error => "",
            Severity::Warning => "warning: ",
        };
        let mut out = format!(
            "{}:{}  {:<16} {level}{}\n",
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

// ---------------------------------------------------------------------------
// Formats for CI: GitHub annotations, SARIF and Markdown
// ---------------------------------------------------------------------------

/// A path as the web writes it: relative, with `/`.
fn uri(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A workflow command's message: `%`, carriage returns and newlines escaped.
fn escape_data(text: &str) -> String {
    text.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A")
}

/// A workflow command's property: the message's escapes, and `:` and `,`.
fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}

/// GitHub Actions workflow commands, one per diagnostic: the Files changed tab of a pull
/// request shows each as an annotation on its line.
pub fn github(diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    for diagnostic in diagnostics {
        let level = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let mut message = diagnostic.message.clone();
        if let Some(fix) = &diagnostic.fix {
            message.push_str(&format!("\nFix: {fix}"));
        }
        out.push_str(&format!(
            "::{level} file={},line={},col={},title={}::{}\n",
            escape_property(&uri(&diagnostic.relative_path)),
            diagnostic.location.line,
            diagnostic.location.column,
            escape_property(&format!("gglint {}", diagnostic.rule)),
            escape_data(&message),
        ));
    }
    out
}

/// `text` as a JSON string, quotes included.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Every rule gglint reports, with what it says in one line.
const RULES: &[(&str, &str)] = &[
    (
        "missing-server",
        "An MCP server is set up for some coding agents but not others",
    ),
    (
        "missing-config",
        "A coding agent has its own directory but no MCP config file",
    ),
    (
        "different-server",
        "The same MCP server is set up differently for two coding agents",
    ),
    (
        "renamed-server",
        "The same MCP server has a different name for two coding agents",
    ),
    (
        "inline-secret",
        "A credential in a well-known format is written out in a file",
    ),
    (
        "inline-credential",
        "A value under a credential-like name is written out in a file",
    ),
    (
        "local-file",
        "An agent file meant for one person's machine is in git, or not ignored by it",
    ),
];

/// A SARIF 2.1.0 log, which GitHub shows under Security, Code scanning. `category` tells
/// GitHub which run it belongs to, so each command's findings are kept apart.
pub fn sarif(category: &str, diagnostics: &[Diagnostic]) -> String {
    let rules: Vec<String> = RULES
        .iter()
        .map(|(id, text)| {
            format!(
                "{{\"id\":{},\"shortDescription\":{{\"text\":{}}}}}",
                json_string(id),
                json_string(text)
            )
        })
        .collect();
    let results: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| {
            let level = match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            let mut message = diagnostic.message.clone();
            if let Some(fix) = &diagnostic.fix {
                message.push_str(&format!(" Fix: {fix}"));
            }
            format!(
                "{{\"ruleId\":{},\"level\":\"{level}\",\"message\":{{\"text\":{}}},\"locations\":[{{\"physicalLocation\":{{\"artifactLocation\":{{\"uri\":{},\"uriBaseId\":\"%SRCROOT%\"}},\"region\":{{\"startLine\":{},\"startColumn\":{}}}}}}}]}}",
                json_string(diagnostic.rule),
                json_string(&message),
                json_string(&uri(&diagnostic.relative_path)),
                diagnostic.location.line,
                diagnostic.location.column,
            )
        })
        .collect();
    format!(
        "{{\"$schema\":\"https://json.schemastore.org/sarif-2.1.0.json\",\"version\":\"2.1.0\",\"runs\":[{{\"tool\":{{\"driver\":{{\"name\":\"gglint\",\"version\":{},\"informationUri\":\"https://github.com/corridor/gglint\",\"rules\":[{}]}}}},\"automationDetails\":{{\"id\":{}}},\"results\":[{}]}}]}}\n",
        json_string(env!("CARGO_PKG_VERSION")),
        rules.join(","),
        json_string(&format!("gglint/{category}/")),
        results.join(","),
    )
}

/// `text` safe inside a Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// A Markdown section for a pull request comment: a heading with the count, then a table of
/// the problems.
pub fn markdown(title: &str, diagnostics: &[Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return format!("### ✅ {title}: no problems\n");
    }
    let count = diagnostics.len();
    let icon = if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        "❌"
    } else {
        "⚠️"
    };
    let mut out = format!(
        "### {icon} {title}: {count} problem{}\n\n| Where | Rule | Problem |\n| --- | --- | --- |\n",
        if count == 1 { "" } else { "s" }
    );
    for diagnostic in diagnostics {
        let mut problem = diagnostic.message.clone();
        if let Some(fix) = &diagnostic.fix {
            problem.push_str(&format!(" — {fix}"));
        }
        out.push_str(&format!(
            "| `{}:{}` | `{}` | {} |\n",
            cell(&uri(&diagnostic.relative_path)),
            diagnostic.location.line,
            diagnostic.rule,
            cell(&problem),
        ));
    }
    out
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

/// The exit status for a run: 1 when it found an error, 0 for warnings alone.
pub fn exit_code(diagnostics: &[Diagnostic]) -> std::process::ExitCode {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(message: &str) -> Diagnostic {
        Diagnostic {
            rule: "inline-secret",
            severity: Severity::Error,
            path: PathBuf::from("/repo/.cursor/mcp.json"),
            relative_path: PathBuf::from(".cursor/mcp.json"),
            location: Location { line: 5, column: 45 },
            message: message.to_string(),
            fix: Some("use ${env:GITHUB_TOKEN}".to_string()),
            related: Vec::new(),
        }
    }

    #[test]
    fn github_annotations_name_the_file_and_line_and_escape_the_message() {
        let out = github(&[diagnostic("100% of it, on\ntwo lines")]);
        assert!(
            out.starts_with("::error file=.cursor/mcp.json,line=5,col=45,title=gglint inline-secret::"),
            "{out}"
        );
        assert!(
            out.contains("100%25 of it") && out.contains("%0AFix: use ${env:GITHUB_TOKEN}"),
            "{out}"
        );
        assert_eq!(
            out.lines().count(),
            1,
            "a newline in the message must not end the command"
        );
    }

    #[test]
    fn colons_and_commas_in_a_path_are_escaped_in_a_property() {
        let mut one = diagnostic("x");
        one.relative_path = PathBuf::from("a,b:c.json");
        assert!(github(&[one]).contains("file=a%2Cb%3Ac.json,"));
    }

    #[test]
    fn json_strings_escape_quotes_backslashes_and_control_characters() {
        assert_eq!(json_string("a\"b\\c\n\u{1}"), "\"a\\\"b\\\\c\\n\\u0001\"");
    }

    #[test]
    fn sarif_has_the_results_the_rules_and_a_category() {
        let out = sarif("secrets", &[diagnostic("x \"quoted\"")]);
        assert!(out.contains("\"version\":\"2.1.0\""));
        assert!(out.contains("\"id\":\"gglint/secrets/\""));
        assert!(out.contains("\"ruleId\":\"inline-secret\""));
        assert!(out.contains("\"startLine\":5,\"startColumn\":45"));
        assert!(out.contains("\"uri\":\".cursor/mcp.json\""));
        assert!(out.contains("x \\\"quoted\\\""));
        assert!(out.contains("\"id\":\"missing-server\""), "every rule is listed");
    }

    #[test]
    fn markdown_has_a_table_or_says_there_are_no_problems() {
        assert_eq!(markdown("gglint secrets", &[]), "### ✅ gglint secrets: no problems\n");
        let out = markdown("gglint secrets", &[diagnostic("a | b")]);
        assert!(out.starts_with("### ❌ gglint secrets: 1 problem\n"), "{out}");
        assert!(
            out.contains("| `.cursor/mcp.json:5` | `inline-secret` | a \\| b — use ${env:GITHUB_TOKEN} |"),
            "{out}"
        );
    }
}
