//! A local web page to watch gglint at work: type a path, scan it, and look at its
//! LangGraph graphs at the three tiers of `gglint diagram`, with what each box is made of
//! and where in the code it comes from. Like `visualize.rs`, it's a tool for working on
//! gglint, not part of the library.
//!
//! ```sh
//! cargo run --release --example ui -- [PATH] [PORT]
//! ```
//!
//! It only listens on 127.0.0.1. Every scan runs here, in Rust, when the page asks for
//! it: `GET /api/scan?path=...` scans and lists the graphs found, and
//! `GET /api/graph?index=...` draws one of them at every tier, as Mermaid and JSON, and
//! says how long each took. The last scan is kept until the next.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Instant;

use gglint::discovery::{FileResult, scan};
use gglint::views::{self, Format, Tier};

const PAGE: &str = include_str!("ui.html");

fn main() {
    let mut args = std::env::args().skip(1);
    let default_path = args.next().unwrap_or_else(|| ".".to_string());
    let port = args.next().and_then(|port| port.parse().ok()).unwrap_or(8770u16);
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind the port");
    eprintln!("gglint ui on http://127.0.0.1:{port}");
    let page = PAGE.replace("__DEFAULT_PATH__", &escape_html(&default_path));
    let mut scanned: Vec<FileResult> = Vec::new();
    for stream in listener.incoming().flatten() {
        if let Err(error) = respond(stream, &page, &mut scanned) {
            eprintln!("error: {error}");
        }
    }
}

fn respond(mut stream: TcpStream, page: &str, scanned: &mut Vec<FileResult>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    // The rest of the headers aren't needed.
    let mut line = String::new();
    while reader.read_line(&mut line)? > 2 {
        line.clear();
    }
    let target = request.split_whitespace().nth(1).unwrap_or("/");
    let (route, query) = target.split_once('?').unwrap_or((target, ""));
    let parameter = |name: &str| {
        query.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('=')).map(decode)
    };
    let (status, body) = match route {
        "/" => return send(&mut stream, "200 OK", "text/html; charset=utf-8", page),
        "/api/scan" => scan_json(&parameter("path").unwrap_or_default(), scanned),
        "/api/graph" => match parameter("index").and_then(|index| index.parse().ok()) {
            Some(index) => graph_json(scanned, index),
            None => ("400 Bad Request", error("index is missing")),
        },
        _ => return send(&mut stream, "404 Not Found", "text/plain", "not found"),
    };
    send(&mut stream, status, "application/json", &body)
}

fn error(message: &str) -> String {
    format!("{{\"error\":{}}}", string(message))
}

/// Scans `path`, keeps the results, and lists the graphs found: `{"path", "files",
/// "scan_ms", "graphs": [{"name", "path", "line", "nodes", "edges"}]}`.
fn scan_json(path: &str, scanned: &mut Vec<FileResult>) -> (&'static str, String) {
    let root = PathBuf::from(path);
    if path.is_empty() || !root.exists() {
        return ("404 Not Found", error(&format!("{path} doesn't exist")));
    }
    let start = Instant::now();
    *scanned = scan(std::slice::from_ref(&root), false);
    let scan_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut graphs = Vec::new();
    for result in scanned.iter() {
        let file = result.relative_path.to_string_lossy().replace('\\', "/");
        for graph in &result.graphs {
            graphs.push(format!(
                "{{\"name\":{},\"path\":{},\"line\":{},\"nodes\":{},\"edges\":{}}}",
                string(graph.name()),
                string(&file),
                graph.location.line,
                graph.nodes.len(),
                graph.edges.len()
            ));
        }
    }
    let body = format!(
        "{{\"path\":{},\"files\":{},\"scan_ms\":{scan_ms:.3},\"graphs\":[{}]}}",
        string(path),
        scanned.len(),
        graphs.join(",")
    );
    ("200 OK", body)
}

/// Draws the graph at `index` of the last scan at every tier: `{"tiers": {"code":
/// {"mermaid", "json", "ms"}, ...}}`.
fn graph_json(scanned: &[FileResult], index: usize) -> (&'static str, String) {
    let count: usize = scanned.iter().map(|result| result.graphs.len()).sum();
    if index >= count {
        return ("404 Not Found", error("no such graph in the last scan"));
    }
    let mut body = String::from("{\"tiers\":{");
    for (position, tier) in Tier::ALL.into_iter().enumerate() {
        let start = Instant::now();
        let mermaid = views::render_graph(scanned, tier, Format::Mermaid, index);
        let json = views::render_graph(scanned, tier, Format::Json, index);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        if position > 0 {
            body.push(',');
        }
        // The JSON is gglint's own, so it goes in as it is.
        body.push_str(&format!(
            "{}:{{\"mermaid\":{},\"json\":{json},\"ms\":{ms:.3}}}",
            string(tier.as_str()),
            string(&mermaid)
        ));
    }
    body.push_str("}}");
    ("200 OK", body)
}

fn send(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// `text` as a JSON string.
fn string(text: &str) -> String {
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

/// A query string value: `+` is a space and `%2F` a byte.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                match value.get(index + 1..index + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok()).ok_or(()) {
                    Ok(byte) => {
                        decoded.push(byte);
                        index += 2;
                    }
                    Err(_) => decoded.push(b'%'),
                }
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}
