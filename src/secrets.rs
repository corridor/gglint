//! Finds secrets written out in the files that configure coding agents: MCP config files,
//! settings, and instruction files such as `CLAUDE.md`.
//!
//! Two kinds of finding:
//!
//! - `inline-secret`: text in the format of a well-known credential (a GitHub token, an AWS
//!   access key ID, a private key block).
//! - `inline-credential`: a value written out under a name such as `token`, `password` or
//!   `Authorization`, in a JSON or TOML file, that isn't a reference to an environment
//!   variable.
//!
//! Nothing here prints, stores or sends a secret's value: findings carry the kind, the key
//! it's under and where it is. Where the agent can read a value from the environment,
//! `apply_fixes` replaces the secret with a reference to a variable, such as `${env:GITHUB_TOKEN}`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::harness::{HARNESSES, Harness, Location, ignored_by_git};
use crate::report::{Diagnostic, Severity};

/// Files bigger than this aren't agent configuration, and aren't read.
const MAX_FILE_SIZE: u64 = 1 << 20;

/// Instruction and settings files of the agents, relative to the root.
const EXTRA_FILES: &[&str] = &[
    "CLAUDE.md",
    "AGENTS.md",
    "GEMINI.md",
    "QWEN.md",
    ".cursorrules",
    ".windsurfrules",
    ".clinerules",
    ".claude/settings.json",
    ".github/copilot-instructions.md",
];

/// Agent files meant for one person's machine and never for git, and what each one is.
/// Committing one is a problem whatever is in it, so the `local-file` rule doesn't depend on
/// recognising a secret.
const LOCAL_FILES: &[(&str, &str)] = &[
    (
        ".claude/settings.local.json",
        "Claude Code's personal settings, where it saves the commands you allow, secrets and all",
    ),
    ("CLAUDE.local.md", "Claude Code's personal notes for this project"),
    (".claude/.credentials.json", "Claude Code's login"),
    (".codex/auth.json", "Codex's login"),
];

/// Directories of the agents' rules, commands and skills, whose files are read too.
const EXTRA_DIRECTORIES: &[&str] = &[
    ".claude/commands",
    ".claude/agents",
    ".cursor/rules",
    ".github/instructions",
    ".kiro/steering",
    ".junie",
];

/// What to scan.
#[derive(Clone, Debug)]
pub enum Scope {
    /// Every agent file in the repository.
    Known,
    /// The agent files with changes staged in git.
    Staged,
    /// These files, whatever they are: what a pre-commit hook passes.
    Files(Vec<PathBuf>),
}

/// A secret that `apply_fixes` can replace with a reference to an environment variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingFix {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    /// 1-based.
    pub line: usize,
    /// Byte offsets of the secret in its line.
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub fixes: Vec<PendingFix>,
    /// Personal agent files that git doesn't ignore, relative to the root, for
    /// `ignore_local_files` to add to `.gitignore`.
    pub ignore: Vec<PathBuf>,
}

/// One secret found in a text.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Hit {
    /// 1-based.
    line: usize,
    /// Byte offsets in the line.
    start: usize,
    end: usize,
    /// 1-based, in characters.
    column: usize,
    rule: &'static str,
    kind: String,
    /// The name it's written under: `GITHUB_TOKEN` in `"GITHUB_TOKEN": "ghp_…"`.
    key: Option<String>,
    /// The environment variable to read it from instead.
    env: String,
    /// Whether the key is one whose value an agent reads as an environment or header value, so
    /// a reference works there.
    replaceable: bool,
    /// Whether it's written in a command line, such as a command Claude Code saved as allowed.
    command: bool,
}

struct Detector {
    prefixes: &'static [&'static str],
    kind: &'static str,
    env: &'static str,
    /// How many characters of `charset` follow the prefix.
    min: usize,
    max: Option<usize>,
    charset: fn(char) -> bool,
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn dash(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn upper(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

/// Earlier entries win where two match the same text: `sk-ant-` before `sk-`.
const DETECTORS: &[Detector] = &[
    Detector {
        prefixes: &["ghp_", "gho_", "ghu_", "ghs_", "ghr_"],
        kind: "GitHub token",
        env: "GITHUB_TOKEN",
        min: 36,
        max: None,
        charset: alnum,
    },
    Detector {
        prefixes: &["github_pat_"],
        kind: "GitHub token",
        env: "GITHUB_TOKEN",
        min: 22,
        max: None,
        charset: word,
    },
    Detector {
        prefixes: &["sk-ant-"],
        kind: "Anthropic API key",
        env: "ANTHROPIC_API_KEY",
        min: 20,
        max: None,
        charset: dash,
    },
    Detector {
        prefixes: &["sk-"],
        kind: "API key",
        env: "API_KEY",
        min: 20,
        max: None,
        charset: dash,
    },
    Detector {
        prefixes: &["AKIA", "ASIA"],
        kind: "AWS access key ID",
        env: "AWS_ACCESS_KEY_ID",
        min: 16,
        max: Some(16),
        charset: upper,
    },
    Detector {
        prefixes: &["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"],
        kind: "Slack token",
        env: "SLACK_TOKEN",
        min: 10,
        max: None,
        charset: dash,
    },
    Detector {
        prefixes: &["AIza"],
        kind: "Google API key",
        env: "GOOGLE_API_KEY",
        min: 35,
        max: Some(35),
        charset: dash,
    },
    Detector {
        prefixes: &["sk_live_", "rk_live_"],
        kind: "Stripe key",
        env: "STRIPE_API_KEY",
        min: 16,
        max: None,
        charset: alnum,
    },
    Detector {
        prefixes: &["glpat-"],
        kind: "GitLab token",
        env: "GITLAB_TOKEN",
        min: 20,
        max: None,
        charset: dash,
    },
    Detector {
        prefixes: &["npm_"],
        kind: "npm token",
        env: "NPM_TOKEN",
        min: 36,
        max: Some(36),
        charset: alnum,
    },
];

/// Words in a key's name that say its value is a credential.
const SENSITIVE_WORDS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "apikey",
    "credential",
    "credentials",
    "authorization",
    "bearer",
    "passphrase",
];

/// Pairs of consecutive words that say the same: `api_key`, `privateKey`.
const SENSITIVE_PAIRS: &[(&str, &str)] = &[
    ("api", "key"),
    ("private", "key"),
    ("access", "key"),
    ("secret", "key"),
    ("auth", "key"),
];

/// Words that say the value names or locates a credential instead of being one:
/// `bearer_token_env_var = "GITHUB_TOKEN"`, `tokenFile`.
const REFERENCE_WORDS: &[&str] = &[
    "env", "var", "vars", "file", "path", "url", "uri", "endpoint", "header", "headers", "name", "type", "helper",
    "command", "cmd",
];

/// Keys under which an agent reads a command, arguments or a URL, where it writes an
/// environment variable another way than in the value of a variable or header.
const COMMAND_LINE_KEYS: &[&str] = &["url", "serverurl", "httpurl", "command", "args"];

/// The words in a key's name, lowercase: `apiKey` and `API_KEY` are both `api`, `key`.
fn words(key: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for c in key.chars() {
        if !c.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current).to_ascii_lowercase());
            }
            previous_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && previous_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current).to_ascii_lowercase());
        }
        current.push(c);
        previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
    }
    if !current.is_empty() {
        words.push(current.to_ascii_lowercase());
    }
    words
}

/// Words that say a name is a credential when they end a name written as one word:
/// `PGPASSWORD`, `ACCESSTOKEN`.
const GLUED_ENDINGS: &[&str] = &["token", "secret", "password", "passwd", "apikey", "passphrase"];

fn names_a_credential(key: &str) -> bool {
    // The working directory, not a password.
    if key.eq_ignore_ascii_case("pwd") {
        return false;
    }
    let words = words(key);
    if words.iter().any(|w| REFERENCE_WORDS.contains(&w.as_str())) {
        return false;
    }
    words.iter().any(|w| SENSITIVE_WORDS.contains(&w.as_str()))
        || words
            .windows(2)
            .any(|pair| SENSITIVE_PAIRS.contains(&(pair[0].as_str(), pair[1].as_str())))
        || words.iter().any(|w| {
            GLUED_ENDINGS
                .iter()
                .any(|ending| w.len() > ending.len() && w.ends_with(ending))
        })
}

/// Where a run of characters that `keep` accepts, ending at byte `end` of `line`, starts.
fn run_start(line: &str, end: usize, keep: impl Fn(char) -> bool) -> usize {
    line[..end]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| keep(c))
        .last()
        .map_or(end, |(index, _)| index)
}

/// The number of spaces at the start of `text`.
fn spaces(text: &str) -> usize {
    text.len() - text.trim_start_matches(' ').len()
}

/// Whether `c` ends a value written without quotes in a command line.
fn ends_value(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '`' | ',' | ';' | ')' | '|' | '&')
}

/// The byte range of the value that starts at byte `at` of `line`: what's inside the quotes
/// of a quoted one (a JSON-escaped `\"` too), else the characters up to the end of the word.
fn command_value(line: &str, at: usize) -> Option<(usize, usize)> {
    let rest = &line[at..];
    for quote in ["\\\"", "\"", "'"] {
        if let Some(inner) = rest.strip_prefix(quote) {
            let start = at + quote.len();
            return inner.find(quote).map(|close| (start, start + close));
        }
    }
    let end = rest.find(ends_value).unwrap_or(rest.len());
    (end > 0).then_some((at, at + end))
}

/// Whether `value` is code or the name of a variable rather than a secret: a call or an
/// index, a dotted name such as `process.env.KEY`, or a name such as `MY_TOKEN`.
fn looks_like_code(value: &str) -> bool {
    let identifier = |part: &str| {
        (1..=24).contains(&part.len())
            && part.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    value.contains(['(', '[', '{'])
        || value.contains('.') && value.split('.').all(identifier)
        || value.contains('_')
            && value.starts_with(|c: char| c.is_ascii_uppercase())
            && value
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// The secrets written into command lines in `line`, as the byte range of each and the name
/// it's given: a header (`Authorization: Bearer …`), a variable set for a command
/// (`PGPASSWORD=… psql`), an option (`--token …`, `sshpass -p …`, `mysql -p…`), or an option
/// and the value after it in a JSON list of arguments. This is how a command Claude Code saved
/// as allowed carries a secret.
fn command_secrets(line: &str) -> Vec<(usize, usize, String)> {
    let mut found = Vec::new();
    let mut accept = |name: &str, span: Option<(usize, usize)>| {
        let Some((start, end)) = span else {
            return;
        };
        let value = &line[start..end];
        if names_a_credential(name)
            && !is_reference(value)
            && !placeholder(value)
            && credential_like(value)
            && !looks_like_code(value)
        {
            found.push((start, end, name.to_string()));
        }
    };

    // A header: `Authorization: Bearer …`, `X-Api-Key: …`.
    for (colon, _) in line.match_indices(':') {
        let start = run_start(line, colon, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        let gap = spaces(&line[colon + 1..]);
        if start == colon || gap == 0 {
            continue;
        }
        let mut at = colon + 1 + gap;
        if let Some(scheme) = ["Bearer ", "Basic ", "Token "].iter().find(|scheme| {
            line[at..]
                .get(..scheme.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(scheme))
        }) {
            at += scheme.len();
        }
        accept(&line[start..colon], command_value(line, at));
    }

    // A variable set for a command: `PGPASSWORD=… psql`, `export API_TOKEN=…`.
    for (equals, _) in line.match_indices('=') {
        let start = run_start(line, equals, |c| c.is_ascii_alphanumeric() || c == '_');
        // `--password=…` is an option, read below; `a == b` and `key = "…"` aren't commands.
        if start == equals || line[..start].ends_with('-') || line[equals + 1..].starts_with(['=', ' ']) {
            continue;
        }
        accept(&line[start..equals], command_value(line, equals + 1));
    }

    // An option: `--token …`, `--password=…`, or `"--api-key", "…"` in a list of arguments.
    for (dashes, _) in line.match_indices("--") {
        if line[..dashes].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        let name_start = dashes + 2;
        let name_end = name_start
            + line[name_start..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(line.len() - name_start);
        let name = &line[name_start..name_end];
        let mut at = name_end;
        if line[at..].starts_with('=') {
            at += 1;
        } else {
            let quoted = ["\\\"", "\""].iter().find(|quote| line[at..].starts_with(*quote));
            if let Some(quote) = quoted {
                at += quote.len();
                at += spaces(&line[at..]);
                if !line[at..].starts_with(',') {
                    continue;
                }
                at += 1;
            } else if spaces(&line[at..]) == 0 {
                continue;
            }
            at += spaces(&line[at..]);
        }
        let value = command_value(line, at).filter(|&(start, _)| !line[start..].starts_with('-'));
        accept(name, value);
    }

    // A password given with `-p`: `sshpass -p …`, `mysql -p…`.
    if let Some(at) = line.find("sshpass -p") {
        let at = at + "sshpass -p".len();
        accept("password", command_value(line, at + spaces(&line[at..])));
    }
    if let Some(program) = line.find("mysql").or_else(|| line.find("mariadb")) {
        for (option, _) in line[program..].match_indices(" -p") {
            let at = program + option + " -p".len();
            if line[at..].starts_with(|c: char| !ends_value(c)) {
                accept("password", command_value(line, at));
            }
        }
    }
    found
}

/// Text that stands in for a secret in documentation and templates.
fn placeholder(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let distinct: BTreeSet<char> = text.chars().collect();
    distinct.len() < 5
        || [
            "your",
            "example",
            "changeme",
            "change-me",
            "placeholder",
            "xxxx",
            "****",
            "redacted",
            "todo",
            "dummy",
            "replace",
            "<",
            "1234567890",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn is_reference(value: &str) -> bool {
    value.contains("${") || value.contains("{env:") || value.contains("{file:") || value.starts_with(['$', '%'])
}

/// Whether `value` looks like a credential: long enough, no spaces, and not made of a
/// single kind of letters, which is how words and names look.
fn credential_like(value: &str) -> bool {
    let lower = value.chars().any(|c| c.is_ascii_lowercase());
    let upper = value.chars().any(|c| c.is_ascii_uppercase());
    let digit = value.chars().any(|c| c.is_ascii_digit());
    let symbol = value.chars().any(|c| !c.is_ascii_alphanumeric());
    value.len() >= 8 && !value.chars().any(char::is_whitespace) && (digit || symbol || (lower && upper))
}

/// A string, an identifier, a `:` or `=`, or anything else, in a line of JSON or TOML.
enum Token<'a> {
    Text { value: &'a str, start: usize, end: usize },
    Name(&'a str),
    Separator,
    Other,
}

fn tokens(line: &str) -> Vec<Token<'_>> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'"' | b'\'' => {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != c {
                    if bytes[j] == b'\\' && c == b'"' {
                        j += 1;
                    }
                    j += 1;
                }
                let end = j.min(bytes.len());
                tokens.push(Token::Text {
                    value: &line[start..end],
                    start,
                    end,
                });
                i = j + 1;
            }
            b':' | b'=' => {
                tokens.push(Token::Separator);
                i += 1;
            }
            c if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-') => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'.' | b'-')) {
                    i += 1;
                }
                tokens.push(Token::Name(&line[start..i]));
            }
            _ => {
                if !c.is_ascii_whitespace() {
                    tokens.push(Token::Other);
                }
                i += 1;
            }
        }
    }
    tokens
}

/// A key and the string written for it, in one line: `"key": "value"` or `key = "value"`.
struct Pair<'a> {
    key: &'a str,
    value: &'a str,
    /// Byte offsets of the value, without its quotes.
    start: usize,
    end: usize,
}

fn pairs(line: &str) -> Vec<Pair<'_>> {
    let tokens = tokens(line);
    tokens
        .windows(3)
        .filter_map(|window| match window {
            [
                Token::Text { value: key, .. } | Token::Name(key),
                Token::Separator,
                Token::Text { value, start, end },
            ] => Some(Pair {
                key,
                value,
                start: *start,
                end: *end,
            }),
            _ => None,
        })
        .collect()
}

/// The environment variable to read a secret under `key` from: the key itself when it's
/// written like a variable's name, else a name made from its words.
fn env_name(key: &str, fallback: &str) -> String {
    let like_variable = key.len() > 2
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && key.starts_with(|c: char| c.is_ascii_uppercase());
    if like_variable {
        return key.to_string();
    }
    let words = words(key);
    match words.as_slice() {
        [] => fallback.to_string(),
        [only] if only == "authorization" => "AUTH_TOKEN".to_string(),
        words => words.join("_").to_ascii_uppercase(),
    }
}

/// Hosts that are this machine: a password for one is a local setup, not a secret.
const LOCAL_HOSTS: &[&str] = &["localhost", "127.0.0.1", "0.0.0.0", "[::1]"];

/// Words that stand in for the password in an example URL.
const FILLER_PASSWORDS: &[&str] = &["password", "pass", "passwd", "pwd", "pw", "secret"];

/// Whether `value` could be a real password: 6 characters or more, with a digit, a symbol
/// or both cases of letters.
fn password_like(value: &str) -> bool {
    let lower = value.chars().any(|c| c.is_ascii_lowercase());
    let upper = value.chars().any(|c| c.is_ascii_uppercase());
    let digit = value.chars().any(|c| c.is_ascii_digit());
    let symbol = value.chars().any(|c| !c.is_ascii_alphanumeric());
    value.len() >= 6 && (digit || symbol || (lower && upper))
}

/// The byte ranges in `line` of passwords written into URLs: `secret` in
/// `postgresql://user:secret@host/db`.
fn url_passwords(line: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for (separator, _) in line.match_indices("://") {
        if !line[..separator].ends_with(|c: char| c.is_ascii_alphanumeric()) {
            continue;
        }
        let start = separator + 3;
        let rest = &line[start..];
        let end = rest
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '/' | '?' | '#' | '"' | '\'' | '`' | '<' | '>' | '\\' | ')' | ']')
            })
            .unwrap_or(rest.len());
        let authority = &rest[..end];
        let Some(at) = authority.rfind('@') else {
            continue;
        };
        let Some((user, password)) = authority[..at].split_once(':') else {
            continue;
        };
        let host = &authority[at + 1..];
        let host = match host.find(']') {
            Some(close) if host.starts_with('[') => &host[..=close],
            _ => host.split(':').next().unwrap_or(host),
        };
        if password == user
            || LOCAL_HOSTS.contains(&host.to_ascii_lowercase().as_str())
            || FILLER_PASSWORDS.contains(&password.to_ascii_lowercase().as_str())
            || is_reference(password)
            || placeholder(password)
            || !password_like(password)
        {
            continue;
        }
        let begin = start + user.len() + 1;
        found.push((begin, begin + password.len()));
    }
    found
}

/// The variable for the password in the URL under `key`: `DATABASE_PASSWORD` for
/// `DATABASE_URL`. `None` when the key isn't written like a variable's name.
fn password_variable(key: &str) -> Option<String> {
    if env_name(key, "") != key {
        return None;
    }
    ["_URL", "_URI", "_DSN"]
        .iter()
        .find_map(|suffix| key.strip_suffix(suffix))
        .map(|stem| format!("{stem}_PASSWORD"))
}

fn column(line: &str, offset: usize) -> usize {
    line[..offset].chars().count() + 1
}

/// Every secret in `text`. `config` is whether it's JSON or TOML, where a value under a
/// key such as `token` counts too.
fn scan_text(text: &str, config: bool) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let pairs = pairs(line);
        // The hits in this line start here.
        let first = hits.len();

        let mut spans: Vec<(usize, usize, &Detector)> = Vec::new();
        for detector in DETECTORS {
            for prefix in detector.prefixes {
                for (start, _) in line.match_indices(prefix) {
                    if line[..start].chars().next_back().is_some_and(alnum) {
                        continue;
                    }
                    let tail_start = start + prefix.len();
                    let run: usize = line[tail_start..]
                        .chars()
                        .take_while(|&c| (detector.charset)(c))
                        .map(char::len_utf8)
                        .sum();
                    let tail = &line[tail_start..tail_start + run];
                    if run < detector.min || detector.max.is_some_and(|max| run != max) || placeholder(tail) {
                        continue;
                    }
                    // Many names start with `sk-` (`sk-image-generation-v2`); a key has a long
                    // random part with a digit in it.
                    let random_part = |part: &str| part.len() >= 16 && part.contains(|c: char| c.is_ascii_digit());
                    if *prefix == "sk-" && !tail.split(['-', '_']).any(random_part) {
                        continue;
                    }
                    let end = tail_start + run;
                    if spans.iter().any(|&(s, e, _)| start < e && s < end) {
                        continue;
                    }
                    spans.push((start, end, detector));
                }
            }
        }
        spans.sort_by_key(|span| span.0);
        for (start, end, detector) in spans {
            let pair = pairs.iter().find(|pair| pair.start <= start && end <= pair.end);
            let key = pair.map(|pair| pair.key.to_string());
            let replaceable = key
                .as_deref()
                .is_some_and(|key| !COMMAND_LINE_KEYS.contains(&key.to_ascii_lowercase().as_str()));
            hits.push(Hit {
                line: number,
                start,
                end,
                column: column(line, start),
                rule: "inline-secret",
                kind: detector.kind.to_string(),
                env: match &key {
                    Some(key) if key.len() > 2 && env_name(key, "") == *key => key.clone(),
                    _ => detector.env.to_string(),
                },
                key,
                replaceable,
                command: false,
            });
        }

        if let Some(start) = line.find("-----BEGIN ")
            && line[start..].contains("PRIVATE KEY-----")
        {
            hits.push(Hit {
                line: number,
                start,
                end: start + "-----BEGIN ".len(),
                column: column(line, start),
                rule: "inline-secret",
                kind: "private key".to_string(),
                key: None,
                env: "PRIVATE_KEY".to_string(),
                replaceable: false,
                command: false,
            });
        }

        for (start, end) in url_passwords(line) {
            if hits[first..].iter().any(|hit| start < hit.end && hit.start < end) {
                continue;
            }
            let key = pairs
                .iter()
                .find(|pair| pair.start <= start && end <= pair.end)
                .map(|pair| pair.key.to_string());
            hits.push(Hit {
                line: number,
                start,
                end,
                column: column(line, start),
                rule: "inline-credential",
                kind: "password in a URL".to_string(),
                env: key
                    .as_deref()
                    .and_then(password_variable)
                    .unwrap_or_else(|| "DB_PASSWORD".to_string()),
                replaceable: key
                    .as_deref()
                    .is_some_and(|key| !COMMAND_LINE_KEYS.contains(&key.to_ascii_lowercase().as_str())),
                key,
                command: false,
            });
        }

        if config {
            for pair in pairs {
                if !names_a_credential(pair.key) {
                    continue;
                }
                let (offset, credential) = ["Bearer ", "Basic ", "Token "]
                    .iter()
                    .find_map(|scheme| {
                        pair.value
                            .get(..scheme.len())
                            .filter(|head| head.eq_ignore_ascii_case(scheme))
                            .map(|_| (scheme.len(), &pair.value[scheme.len()..]))
                    })
                    .unwrap_or((0, pair.value));
                if is_reference(pair.value)
                    || credential.starts_with("http")
                    || placeholder(credential)
                    || !credential_like(credential)
                {
                    continue;
                }
                // Another secret in the line doesn't hide this one, unless it's the same text.
                let start = pair.start + offset;
                if hits[first..].iter().any(|hit| start < hit.end && hit.start < pair.end) {
                    continue;
                }
                hits.push(Hit {
                    line: number,
                    start: pair.start + offset,
                    end: pair.end,
                    column: column(line, pair.start + offset),
                    rule: "inline-credential",
                    kind: "credential".to_string(),
                    key: Some(pair.key.to_string()),
                    env: env_name(pair.key, "API_TOKEN"),
                    replaceable: !COMMAND_LINE_KEYS.contains(&pair.key.to_ascii_lowercase().as_str()),
                    command: false,
                });
            }
        }

        for (start, end, name) in command_secrets(line) {
            if hits[first..].iter().any(|hit| start < hit.end && hit.start < end) {
                continue;
            }
            hits.push(Hit {
                line: number,
                start,
                end,
                column: column(line, start),
                rule: "inline-credential",
                kind: "credential".to_string(),
                env: env_name(&name, "API_TOKEN"),
                key: Some(name),
                replaceable: false,
                command: true,
            });
        }
    }
    hits
}

fn is_config(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("json" | "jsonc" | "toml")
    )
}

/// `path` relative to `root`, written with `/`.
fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The agent files gglint reads at fixed paths, relative to the root.
fn fixed_files() -> impl Iterator<Item = &'static str> {
    HARNESSES
        .iter()
        .flat_map(|harness| harness.mcp_files().iter().copied())
        .chain(EXTRA_FILES.iter().copied())
        .chain(LOCAL_FILES.iter().map(|&(file, _)| file))
}

/// Whether a file in one of `EXTRA_DIRECTORIES` is a kind gglint reads.
fn wanted_in_directory(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("md" | "mdc" | "json" | "toml")
    )
}

/// Whether `relative`, a path under the root, is one of the agent files gglint reads.
fn is_agent_file(relative: &Path) -> bool {
    let name = slashed(relative);
    fixed_files().any(|file| file == name)
        || relative
            .parent()
            .is_some_and(|parent| EXTRA_DIRECTORIES.contains(&slashed(parent).as_str()))
            && wanted_in_directory(relative)
}

/// Where an agent file named by its path belongs: the folder it's in and its path under that
/// folder, read from the end of the path, so `/repo/.cursor/mcp.json` is `.cursor/mcp.json` in
/// `/repo`. `None` for any other file, as a hook tool such as lint-staged passes every file.
fn agent_file_in(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let text = slashed(path);
    let split = |relative: &str| {
        let base = match text.strip_suffix(relative)? {
            "" => ".",
            rest => rest.strip_suffix('/')?,
        };
        Some((
            PathBuf::from(if base.is_empty() { "/" } else { base }),
            PathBuf::from(relative),
        ))
    };
    fixed_files().find_map(split).or_else(|| {
        let name = path.file_name()?.to_str()?;
        if !wanted_in_directory(path) {
            return None;
        }
        EXTRA_DIRECTORIES
            .iter()
            .find_map(|directory| split(&format!("{directory}/{name}")))
    })
}

/// The agent files that exist under `root`, relative to it.
fn known_files(root: &Path) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    for candidate in fixed_files() {
        if root.join(candidate).is_file() {
            files.insert(PathBuf::from(candidate));
        }
    }
    for directory in EXTRA_DIRECTORIES {
        let Ok(entries) = std::fs::read_dir(root.join(directory)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && wanted_in_directory(&path)
                && let Some(name) = path.file_name()
            {
                files.insert(Path::new(directory).join(name));
            }
        }
    }
    files
}

/// The files with changes staged in git, relative to `root`, which can be below the top of
/// the repository.
fn staged_files(root: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let output = Command::new("git")
        .args([
            "diff",
            "--cached",
            "--name-only",
            "--relative",
            "--diff-filter=ACMR",
            "-z",
        ])
        .current_dir(root)
        .output()
        .map_err(|error| format!("can't run git to list the staged files: {error}"))?;
    if !output.status.success() {
        return Err("`git diff --cached` failed; is this a git repository?".to_string());
    }
    Ok(output
        .stdout
        .split(|&byte| byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| PathBuf::from(String::from_utf8_lossy(name).into_owned()))
        .collect())
}

/// The text of `relative` as it's staged in git: what the next commit gets, which can differ
/// from the file on disk.
fn read_staged(root: &Path, relative: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("show")
        .arg(format!(":./{}", slashed(relative)))
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() as u64 > MAX_FILE_SIZE {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// The text of the file at `path`, unless it's too big to be agent configuration.
fn read_on_disk(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_SIZE {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Whether `relative` is in git's index under `root`: committed, or staged to be. `None`
/// outside a repository, or without git.
fn in_git_index(root: &Path, relative: &Path) -> Option<bool> {
    let output = Command::new("git")
        .args(["ls-files", "-z", "--"])
        .arg(relative)
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(!output.stdout.is_empty())
}

/// The `local-file` finding for a personal agent file: an error once it's in git, a warning
/// while git doesn't ignore it, and nothing once it's ignored.
fn check_local_file(root: &Path, path: &Path, relative: &Path, description: &str, check: &mut ScanResult) {
    let Some(in_index) = in_git_index(root, relative) else {
        return;
    };
    let name = slashed(relative);
    let (severity, message, fix) = if in_index {
        (
            Severity::Error,
            format!("this file is in git, but it's {description}, meant for one person's machine"),
            format!(
                "run `git rm --cached {name}` and add it to .gitignore (and to .dockerignore if you build images from this folder); if it was ever pushed, rotate what's in it"
            ),
        )
    } else if !ignored_by_git(root, relative) {
        (
            Severity::Warning,
            format!("git doesn't ignore this file, so it can be committed by accident; it's {description}"),
            "add it to .gitignore (and to .dockerignore if you build images from this folder); `gglint secrets --fix` adds it to .gitignore".to_string(),
        )
    } else {
        return;
    };
    check.ignore.push(relative.to_path_buf());
    check.diagnostics.push(Diagnostic {
        rule: "local-file",
        severity,
        path: path.to_path_buf(),
        relative_path: relative.to_path_buf(),
        location: Location { line: 1, column: 1 },
        message,
        fix: Some(fix),
        related: Vec::new(),
    });
}

/// Scans the files in `scope` under `root`.
pub fn scan_files(root: &Path, scope: &Scope) -> Result<ScanResult, String> {
    // Each file, as opened from here, under the folder it belongs to, and that folder.
    let files: Vec<(PathBuf, PathBuf, PathBuf)> = match scope {
        Scope::Known => known_files(root)
            .into_iter()
            .map(|relative| (root.join(&relative), relative, root.to_path_buf()))
            .collect(),
        Scope::Staged => staged_files(root)?
            .into_iter()
            .filter(|relative| is_agent_file(relative))
            .map(|relative| (root.join(&relative), relative, root.to_path_buf()))
            .collect(),
        Scope::Files(paths) => paths
            .iter()
            .filter_map(|path| agent_file_in(path).map(|(base, relative)| (path.clone(), relative, base)))
            .collect(),
    };

    let mut check = ScanResult::default();
    for (path, relative_path, root) in files {
        let root = root.as_path();
        let local = LOCAL_FILES.iter().find(|&&(file, _)| file == slashed(&relative_path));
        if let Some(&(_, description)) = local {
            check_local_file(root, &path, &relative_path, description, &mut check);
        }
        let text = match scope {
            Scope::Staged => read_staged(root, &relative_path),
            Scope::Known | Scope::Files(_) => read_on_disk(&path),
        };
        let Some(text) = text else {
            continue;
        };
        check.files += 1;
        let harness = Harness::of_mcp_file(&slashed(&relative_path));
        let hits = scan_text(&text, is_config(&path));
        // Asked once per file with a finding: it runs git.
        let ignored = !hits.is_empty() && ignored_by_git(root, &relative_path);
        for hit in hits {
            let reference = harness
                .filter(|_| hit.replaceable)
                .and_then(|harness| harness.env_reference(&hit.env));
            let mut message = match &hit.key {
                Some(key) => format!("{} in \"{key}\" is written out in the file", hit.kind),
                None => format!("{} is written out in the file", hit.kind),
            };
            if ignored {
                message.push_str(" (the file is gitignored, so it isn't committed)");
            }
            let fix = match (&reference, harness) {
                _ if hit.command => format!(
                    "take it out of the command and pass it from your environment, as ${}; if Claude Code saved the command as allowed, remove it with /permissions; rotate it, as it stays in git history once committed",
                    hit.env
                ),
                (Some(reference), _) => format!(
                    "use {reference} and set {} in your environment; rotate it, as it stays in git history once committed",
                    hit.env
                ),
                // Codex has no `${VAR}` in a value: it names the variable in another key.
                (None, Some(Harness::Codex)) => format!(
                    "in Codex, use `bearer_token_env_var = \"{0}\"` for a Bearer token, or `env_http_headers = {{ Header = \"{0}\" }}` for another header, and set {0} in your environment; rotate it",
                    hit.env
                ),
                (None, _) => "read it from an environment variable instead, and rotate it, as it stays in git history once committed"
                    .to_string(),
            };
            if let Some(replacement) = reference {
                check.fixes.push(PendingFix {
                    path: path.clone(),
                    relative_path: relative_path.clone(),
                    line: hit.line,
                    start: hit.start,
                    end: hit.end,
                    replacement,
                });
            }
            check.diagnostics.push(Diagnostic {
                rule: hit.rule,
                // A file git ignores isn't committed, so the secret stays on this machine.
                severity: if ignored { Severity::Warning } else { Severity::Error },
                path: path.clone(),
                relative_path: relative_path.clone(),
                location: Location {
                    line: hit.line,
                    column: hit.column,
                },
                message,
                fix: Some(fix),
                related: Vec::new(),
            });
        }
    }
    Ok(check)
}

/// Adds each personal agent file in `check.ignore` to the `.gitignore` at `root`, unless it's
/// there already, and returns the ones added. It doesn't take a file out of git: that's
/// `git rm --cached`, which the finding names.
pub fn ignore_local_files(root: &Path, check: &ScanResult) -> Result<Vec<PathBuf>, String> {
    if check.ignore.is_empty() {
        return Ok(Vec::new());
    }
    let path = root.join(".gitignore");
    let mut text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("can't read {}: {error}", path.display())),
    };
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let present: BTreeSet<String> = text.lines().map(|line| line.trim().to_string()).collect();
    let mut added = Vec::new();
    for relative in &check.ignore {
        let line = slashed(relative);
        if present.contains(&line) || present.contains(&format!("/{line}")) || added.contains(relative) {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push_str(newline);
        }
        text.push_str(&line);
        text.push_str(newline);
        added.push(relative.clone());
    }
    if !added.is_empty() {
        std::fs::write(&path, text).map_err(|error| format!("can't write {}: {error}", path.display()))?;
    }
    Ok(added)
}

/// Replaces each secret `check` can with a reference to an environment variable, and returns
/// the files changed with how many secrets each had.
pub fn apply_fixes(check: &ScanResult) -> Result<Vec<(PathBuf, usize)>, String> {
    let mut changed = Vec::new();
    let paths: BTreeSet<&PathBuf> = check.fixes.iter().map(|fix| &fix.path).collect();
    for path in paths {
        let text = std::fs::read_to_string(path).map_err(|error| format!("can't read {}: {error}", path.display()))?;
        let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
        let mut fixes: Vec<&PendingFix> = check.fixes.iter().filter(|fix| &fix.path == path).collect();
        // Right to left, so a replacement doesn't move the ones still to do in its line.
        fixes.sort_by_key(|fix| (fix.line, std::cmp::Reverse(fix.start)));
        let mut count = 0;
        for fix in fixes {
            let Some(line) = lines.get_mut(fix.line - 1) else {
                continue;
            };
            if line.get(fix.start..fix.end).is_none() {
                continue;
            }
            line.replace_range(fix.start..fix.end, &fix.replacement);
            count += 1;
        }
        if count > 0 {
            std::fs::write(path, lines.concat()).map_err(|error| format!("can't write {}: {error}", path.display()))?;
            changed.push((path.clone(), count));
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text that has the mix of a real credential and is nothing else.
    fn fake(length: usize) -> String {
        let alphabet = b"aB3dE6gH9jK2mN5pQ8sT1vW4yZ7";
        (0..length).map(|i| alphabet[i % alphabet.len()] as char).collect()
    }

    #[test]
    fn finds_a_github_token_and_keeps_its_value_out_of_the_hit() {
        let token = format!("ghp_{}", fake(36));
        let text = format!("{{\n  \"env\": {{ \"GITHUB_PERSONAL_ACCESS_TOKEN\": \"{token}\" }}\n}}\n");
        let hits = scan_text(&text, true);
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!((hit.line, hit.rule), (2, "inline-secret"));
        assert_eq!(hit.kind, "GitHub token");
        assert_eq!(hit.key.as_deref(), Some("GITHUB_PERSONAL_ACCESS_TOKEN"));
        assert_eq!(hit.env, "GITHUB_PERSONAL_ACCESS_TOKEN");
        assert!(hit.replaceable);
        assert!(!format!("{hit:?}").contains(&token));
    }

    #[test]
    fn finds_a_literal_authorization_header_but_not_a_reference() {
        let literal = format!("  \"Authorization\": \"Bearer {}\"\n", fake(30));
        let hits = scan_text(&literal, true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule, "inline-credential");
        assert_eq!(hits[0].env, "AUTH_TOKEN");
        // The replacement starts after "Bearer ", so the scheme is kept.
        assert_eq!(&literal[hits[0].start..hits[0].start + 3], fake(30)[..3].to_string());

        for reference in [
            "Bearer ${GITHUB_TOKEN}",
            "Bearer ${env:GITHUB_TOKEN}",
            "Bearer {env:GITHUB_TOKEN}",
        ] {
            let text = format!("\"Authorization\": \"{reference}\"");
            assert!(scan_text(&text, true).is_empty(), "{reference}");
        }
    }

    #[test]
    fn leaves_names_of_variables_and_placeholders_alone() {
        for text in [
            "bearer_token_env_var = \"GITHUB_TOKEN\"",
            "\"max_tokens\": 4096",
            "\"tokenFile\": \"secrets/token.txt\"",
            "\"api_key\": \"your-api-key-here\"",
            "\"password\": \"<password>\"",
            "\"token\": \"short\"",
            "\"token\": \"lowercaseonlyword\"",
            "\"GITHUB_TOKEN\": \"$GITHUB_TOKEN\"",
            "the token is ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            assert!(scan_text(text, true).is_empty(), "{text}");
        }
    }

    #[test]
    fn finds_other_well_known_formats() {
        let cases = [
            (format!("sk-ant-{}", fake(40)), "Anthropic API key"),
            (format!("sk-{}", fake(40)), "API key"),
            (format!("AKIA{}", "ABCDEFGH23456789"), "AWS access key ID"),
            (format!("xoxb-{}", fake(24)), "Slack token"),
            (format!("AIza{}", fake(35)), "Google API key"),
            (format!("glpat-{}", fake(20)), "GitLab token"),
            ("-----BEGIN RSA PRIVATE KEY-----".to_string(), "private key"),
        ];
        for (text, kind) in cases {
            let hits = scan_text(&format!("key: {text}"), false);
            assert_eq!(hits.len(), 1, "{kind}");
            assert_eq!(hits[0].kind, kind);
        }
    }

    #[test]
    fn a_token_inside_a_word_isnt_one() {
        let text = format!("task-{}", fake(30));
        assert!(scan_text(&text, false).is_empty());
    }

    #[test]
    fn instruction_files_get_only_the_well_known_formats() {
        let text = "Set the token with \"token\": \"Abc12345Def\"\n";
        assert!(scan_text(text, false).is_empty());
    }

    #[test]
    fn finds_a_password_in_a_url_wherever_it_is() {
        for (line, config) in [
            (
                "\"args\": [\"-y\", \"@modelcontextprotocol/server-postgres\", \"postgresql://admin:Pr0dDbPass2024@db.internal/app\"]",
                true,
            ),
            ("\"DATABASE_URL\": \"mysql://root:Pr0dDbPass2024@db/app\"", true),
            (
                "Connect with mongodb+srv://app:Pr0dDbPass2024@cluster0.example.net/db",
                false,
            ),
        ] {
            let hits = scan_text(line, config);
            assert_eq!(hits.len(), 1, "{line}");
            assert_eq!(&line[hits[0].start..hits[0].end], "Pr0dDbPass2024", "{line}");
            assert_eq!(hits[0].rule, "inline-credential");
        }
    }

    #[test]
    fn a_password_in_a_variables_url_is_replaced_by_a_password_variable() {
        let hits = scan_text("\"DATABASE_URL\": \"mysql://root:Pr0dDbPass2024@db/app\"", true);
        assert_eq!(hits[0].env, "DATABASE_PASSWORD");
        assert!(hits[0].replaceable);

        let hits = scan_text("\"args\": [\"postgresql://admin:Pr0dDbPass2024@db/app\"]", true);
        assert!(!hits[0].replaceable);
    }

    #[test]
    fn leaves_urls_without_a_real_password_alone() {
        for url in [
            "postgresql://admin:${DB_PASSWORD}@db/app",
            "postgresql://admin:$DB_PASSWORD@db/app",
            "postgresql://admin:{env:DB_PASSWORD}@db/app",
            "postgresql://postgres:Dev12345@localhost:5432/dev",
            "postgresql://postgres:Dev12345@127.0.0.1/dev",
            "postgresql://user:password@db.example.com/app",
            "postgresql://username:<password>@host/db",
            "postgresql://user:your-password@host/db",
            "postgresql://app2024:app2024@db/app",
            "redis://cache.internal:6379",
            "https://github.com/org/repo",
            "ssh://git@github.com/org/repo.git",
            "git@github.com:org/repo.git",
        ] {
            assert!(scan_text(url, false).is_empty(), "{url}");
            assert!(
                scan_text(&format!("\"DATABASE_URL\": \"{url}\""), true).is_empty(),
                "{url}"
            );
        }
    }

    #[test]
    fn finds_secrets_inside_commands() {
        for (line, secret) in [
            (
                r#""Bash(curl -H \"Authorization: Bearer 9f8e7d6c5b4a39281706f5e4d3c2b1a0\" https://api.example.org/v1)","#,
                "9f8e7d6c5b4a39281706f5e4d3c2b1a0",
            ),
            (
                r#""Bash(curl -H 'X-Api-Key: Zx81Kq0Lm2Np' https://api.example.org)","#,
                "Zx81Kq0Lm2Np",
            ),
            (
                r#""Bash(PGPASSWORD=Pr0dDbPass2024 psql -h db.internal -U admin)","#,
                "Pr0dDbPass2024",
            ),
            (r#""Bash(export OPENAI_TOKEN='Zx81Kq0Lm2Np' && run)","#, "Zx81Kq0Lm2Np"),
            (
                r#""Bash(sshpass -p 'Sup3rS3cret!' ssh deploy@10.0.0.5)","#,
                "Sup3rS3cret!",
            ),
            (r#""Bash(mysql -u root -pPr0dDbPass2024 app)","#, "Pr0dDbPass2024"),
            ("deploy --api-token Zx81Kq0Lm2Np --env prod", "Zx81Kq0Lm2Np"),
            ("deploy --password=Pr0dDbPass2024", "Pr0dDbPass2024"),
            (r#""args": ["--api-key", "Zx81Kq0Lm2Np"]"#, "Zx81Kq0Lm2Np"),
        ] {
            for config in [true, false] {
                let hits = scan_text(line, config);
                assert_eq!(hits.len(), 1, "{line} (config: {config})");
                assert_eq!(&line[hits[0].start..hits[0].end], secret, "{line}");
                assert!(hits[0].command && !hits[0].replaceable, "{line}");
            }
        }
    }

    #[test]
    fn leaves_commands_without_a_real_secret_alone() {
        for line in [
            r#""Bash(curl -H \"Authorization: Bearer $API_TOKEN\" https://api.example.org)""#,
            r#""Bash(curl -H \"Authorization: Bearer ${API_TOKEN}\" https://x)""#,
            "Authorization: Bearer <token>",
            r#""Bash(PGPASSWORD=$PGPASSWORD psql)""#,
            "export GITHUB_TOKEN=\"$(gh auth token)\"",
            "password=os.environ[\"DB_PASSWORD\"]",
            "API_KEY=process.env.OPENAI_KEY node app.js",
            "deploy --token MY_DEPLOY_TOKEN",
            "deploy --password-file secrets/db.txt",
            "deploy --token-env DEPLOY_TOKEN",
            "Password: required",
            "sshpass -p \"$SSH_PASS\" ssh host",
            "mysql -p -u root",
            "run --max-tokens 4096",
        ] {
            assert!(scan_text(line, false).is_empty(), "{line}");
            assert!(scan_text(line, true).is_empty(), "{line}");
        }
    }

    #[test]
    fn a_secret_word_at_the_end_of_a_glued_name_counts() {
        assert!(names_a_credential("PGPASSWORD"));
        assert!(names_a_credential("ACCESSTOKEN"));
        assert!(!names_a_credential("TOKENIZER"));
        assert!(!names_a_credential("max_tokens"));

        let hits = scan_text("\"env\": { \"PGPASSWORD\": \"Pr0dDbPass2024\" }", true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].key.as_deref(), Some("PGPASSWORD"));
    }

    #[test]
    fn a_known_token_doesnt_hide_another_secret_in_its_line() {
        let line = format!(
            "\"env\": {{\"GITHUB_TOKEN\": \"ghp_{}\", \"DB_PASSWORD\": \"Pr0dDbPass2024\"}}",
            fake(36)
        );
        let keys: Vec<_> = scan_text(&line, true).into_iter().map(|hit| hit.key).collect();
        assert_eq!(
            keys,
            [Some("GITHUB_TOKEN".to_string()), Some("DB_PASSWORD".to_string())]
        );
    }

    #[test]
    fn the_working_directory_isnt_a_password() {
        for key in ["PWD", "OLDPWD"] {
            let text = format!("\"{key}\": \"/home/user/project\"");
            assert!(scan_text(&text, true).is_empty(), "{key}");
        }
        assert_eq!(scan_text("\"DB_PWD\": \"Pr0dDbPass2024\"", true).len(), 1);
    }

    #[test]
    fn a_name_that_starts_with_sk_isnt_an_api_key() {
        assert!(scan_text("Use the sk-image-generation-pipeline-v2 skill.", false).is_empty());
        for key in [
            format!("sk-proj-{}", fake(40)),
            // DeepSeek's: lowercase hex.
            "sk-1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d".to_string(),
        ] {
            assert_eq!(scan_text(&format!("key: {key}"), false).len(), 1, "{key}");
        }
    }

    #[test]
    fn replaces_every_secret_in_a_line_from_the_right() {
        let a = format!("ghp_{}", fake(36));
        let b = format!("ghp_{}", fake(40));
        let dir = std::env::temp_dir().join(format!("gglint-secrets-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".cursor")).unwrap();
        let file = dir.join(".cursor/mcp.json");
        std::fs::write(&file, format!("{{\"TOKEN_A\": \"{a}\", \"TOKEN_B\": \"{b}\"}}\n")).unwrap();

        let check = scan_files(&dir, &Scope::Known).unwrap();
        assert_eq!(check.diagnostics.len(), 2);
        let changed = apply_fixes(&check).unwrap();
        assert_eq!(changed.len(), 1);

        let fixed = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            fixed,
            "{\"TOKEN_A\": \"${env:TOKEN_A}\", \"TOKEN_B\": \"${env:TOKEN_B}\"}\n"
        );
        assert!(scan_files(&dir, &Scope::Known).unwrap().diagnostics.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
