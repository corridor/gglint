//! What `gglint watch` keeps: one JSON event per line in `events.jsonl`, and the same events in
//! memory for the viewer.
//!
//! Credentials never reach the recording. Headers that carry them (`Authorization`, API keys,
//! cookies) are replaced by their length, and a secret found in a body (by `gglint secrets`'
//! detectors) is replaced by a marker naming its kind. Everything else is kept as sent, since
//! showing it is the point.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::secrets::find_secrets;

/// Headers whose values are credentials, in lower case.
const CREDENTIAL_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "cookie",
    "set-cookie",
    "x-goog-api-key",
];

/// A body larger than this is kept as its size only.
const MAX_BODY: usize = 32 << 20;

pub struct Recorder {
    dir: PathBuf,
    file: Mutex<Option<File>>,
    events: Mutex<Vec<Value>>,
    next_id: AtomicU64,
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Recorder {
    /// Records into `dir/events.jsonl`, creating `dir`.
    pub fn create(dir: &Path) -> Result<Recorder, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
        let path = dir.join("events.jsonl");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("can't write {}: {e}", path.display()))?;
        Ok(Recorder {
            dir: dir.to_path_buf(),
            file: Mutex::new(Some(file)),
            events: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        })
    }

    /// Reads an earlier recording, to show it; nothing more is written.
    pub fn open(dir: &Path) -> Result<Recorder, String> {
        let path = dir.join("events.jsonl");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let events: Vec<Value> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        Ok(Recorder {
            dir: dir.to_path_buf(),
            file: Mutex::new(None),
            events: Mutex::new(events),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A new id for an exchange or a connection.
    pub fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Adds an event, numbering it.
    pub fn push(&self, mut event: Value) {
        let mut events = self.events.lock().unwrap();
        event["seq"] = json!(events.len() + 1);
        if event.get("time").is_none() {
            event["time"] = json!(now_ms());
        }
        if let Some(file) = self.file.lock().unwrap().as_mut() {
            let _ = writeln!(file, "{event}");
        }
        events.push(event);
    }

    /// The events after number `after`, as a JSON array.
    pub fn since(&self, after: usize) -> String {
        let events = self.events.lock().unwrap();
        Value::Array(events.iter().skip(after).cloned().collect()).to_string()
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Headers as a JSON object of lists, with credentials replaced, and what they reveal.
pub fn headers(pairs: &[(String, String)], findings: &mut Vec<Value>) -> Value {
    let mut map = Map::new();
    for (name, value) in pairs {
        let lower = name.to_ascii_lowercase();
        let shown = if CREDENTIAL_HEADERS.contains(&lower.as_str()) {
            format!("[removed by gglint: {} characters]", value.len())
        } else {
            let mut value = value.clone();
            mask(&mut value, &format!("header {name}"), findings);
            value
        };
        match map.entry(lower).or_insert_with(|| json!([])) {
            Value::Array(values) => values.push(json!(shown)),
            _ => unreachable!(),
        }
    }
    Value::Object(map)
}

/// A body as JSON where it parses, else as text, with secrets masked and findings added.
pub fn body(bytes: &[u8], content_type: Option<&str>, findings: &mut Vec<Value>) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    if bytes.len() > MAX_BODY {
        return json!({ "gglint": "too large to keep", "bytes": bytes.len() });
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return json!({ "gglint": "binary", "bytes": bytes.len(), "content-type": content_type });
    };
    if let Ok(mut value) = serde_json::from_str::<Value>(text) {
        walk(&mut value, "$", findings);
        return value;
    }
    let mut text = text.to_string();
    mask(&mut text, "body", findings);
    Value::String(text)
}

fn walk(value: &mut Value, path: &str, findings: &mut Vec<Value>) {
    match value {
        Value::String(text) => mask(text, path, findings),
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                walk(item, &format!("{path}[{index}]"), findings);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                walk(item, &format!("{path}.{key}"), findings);
            }
        }
        _ => {}
    }
}

/// Replaces each secret in `text` with a marker, and notes it and any email address.
fn mask(text: &mut String, place: &str, findings: &mut Vec<Value>) {
    let secrets = find_secrets(text);
    for (range, kind) in secrets.iter().rev() {
        findings.push(json!({ "kind": "secret", "what": kind, "where": place }));
        text.replace_range(range.clone(), &format!("[{kind} removed by gglint]"));
    }
    for email in emails(text) {
        findings.push(json!({ "kind": "email", "what": email, "where": place }));
    }
}

/// Email addresses in a text: `name@domain.tld`, the domain with at least one dot and a
/// letters-only ending.
pub fn emails(text: &str) -> Vec<String> {
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
    let mut found: Vec<String> = Vec::new();
    for (at, _) in text.match_indices('@') {
        let start = text[..at]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| local(c))
            .last()
            .map_or(at, |(i, _)| i);
        let rest = &text[at + 1..];
        let length = rest.find(|c: char| !domain(c)).unwrap_or(rest.len());
        let host = rest[..length].trim_end_matches('.');
        let user = text[start..at].trim_start_matches('.');
        let Some((_, tld)) = host.rsplit_once('.') else {
            continue;
        };
        if user.is_empty() || tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let email = format!("{user}@{host}");
        if !found.contains(&email) {
            found.push(email);
        }
    }
    found
}
