//! The local server behind `gglint watch`, on one port of 127.0.0.1. Each connection is one of:
//!
//! - **A model request**, sent to a route such as `/anthropic/v1/messages` because the agent's
//!   base URL points here. It's recorded in full and sent on over HTTPS to the real API, and
//!   the answer is streamed back and recorded too.
//! - **A proxy tunnel** (`CONNECT host:443`), from the agent's `HTTPS_PROXY`. The bytes pass
//!   through untouched; only the host, the time and the amount sent and received are recorded.
//! - **A plain-HTTP request through the proxy** (`GET http://host/...`), such as one to a local
//!   MCP server. Recorded in full and sent on.
//! - **The viewer**: `/` and `/__gglint/...`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::record::{self, Recorder, now_ms};

const VIEWER: &str = include_str!("viewer.html");

/// Where a route sends its requests: `/anthropic/v1/messages` goes to
/// `https://api.anthropic.com/v1/messages`.
#[derive(Clone, Debug)]
pub struct Route {
    pub prefix: String,
    pub upstream: String,
}

pub fn default_routes() -> Vec<Route> {
    [
        ("anthropic", "https://api.anthropic.com"),
        ("openai", "https://api.openai.com"),
        ("chatgpt", "https://chatgpt.com"),
    ]
    .into_iter()
    .map(|(prefix, upstream)| Route {
        prefix: prefix.to_string(),
        upstream: upstream.to_string(),
    })
    .collect()
}

pub struct Server {
    pub port: u16,
    listener: TcpListener,
    state: Arc<State>,
}

struct State {
    recorder: Arc<Recorder>,
    routes: Vec<Route>,
    port: u16,
    agent: ureq::Agent,
    /// Only the viewer, for an earlier recording.
    view_only: bool,
}

impl Server {
    pub fn bind(port: u16, recorder: Arc<Recorder>, routes: Vec<Route>, view_only: bool) -> Result<Server, String> {
        let listener =
            TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("can't listen on 127.0.0.1:{port}: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .timeout_connect(Some(Duration::from_secs(30)))
            .build()
            .into();
        Ok(Server {
            port,
            listener,
            state: Arc::new(State {
                recorder,
                routes,
                port,
                agent,
                view_only,
            }),
        })
    }

    /// Serves until the process ends.
    pub fn spawn(self) {
        std::thread::spawn(move || {
            for stream in self.listener.incoming().flatten() {
                let state = Arc::clone(&self.state);
                std::thread::spawn(move || {
                    let _ = handle(stream, &state);
                });
            }
        });
    }
}

struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Reads one request, or `None` when the client closed the connection.
fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(std::io::Error::other("bad request line"));
    };
    let (method, target) = (method.to_string(), target.to_string());
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    let mut request = Request {
        method,
        target,
        headers,
        body: Vec::new(),
    };
    if request.method == "CONNECT" {
        return Ok(Some(request));
    }
    if request
        .header("transfer-encoding")
        .is_some_and(|te| te.to_ascii_lowercase().contains("chunked"))
    {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size)?;
            let size = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| std::io::Error::other("bad chunk size"))?;
            if size == 0 {
                // Trailers, then the empty line.
                loop {
                    let mut trailer = String::new();
                    if reader.read_line(&mut trailer)? == 0 || trailer.trim().is_empty() {
                        break;
                    }
                }
                break;
            }
            let mut chunk = vec![0; size];
            reader.read_exact(&mut chunk)?;
            request.body.extend_from_slice(&chunk);
            let mut end = [0; 2];
            reader.read_exact(&mut end)?;
        }
    } else if let Some(length) = request.header("content-length").and_then(|n| n.parse::<usize>().ok()) {
        request.body = vec![0; length];
        reader.read_exact(&mut request.body)?;
    }
    Ok(Some(request))
}

fn handle(stream: TcpStream, state: &State) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    while let Some(request) = read_request(&mut reader)? {
        if request.method == "CONNECT" {
            return tunnel(reader, writer, &request.target, state);
        }
        // Through the proxy, the target is a whole URL; one for this server is a route.
        let own = [
            format!("http://127.0.0.1:{}", state.port),
            format!("http://localhost:{}", state.port),
        ];
        let path = own
            .iter()
            .find_map(|prefix| request.target.strip_prefix(prefix.as_str()))
            .map(str::to_string)
            .unwrap_or_else(|| request.target.clone());
        let keep_alive = !request
            .header("connection")
            .is_some_and(|c| c.eq_ignore_ascii_case("close"));
        if path.starts_with("http://") {
            forward(&mut writer, &request, path.clone(), "http", state)?;
        } else if path == "/" || path.starts_with("/__gglint/") {
            viewer(&mut writer, &path, state)?;
        } else if !state.view_only
            && let Some((route, rest)) = state.routes.iter().find_map(|route| {
                let prefix = format!("/{}", route.prefix);
                let rest = path.strip_prefix(&prefix)?;
                (rest.is_empty() || rest.starts_with('/') || rest.starts_with('?')).then(|| (route, rest.to_string()))
            })
        {
            forward(
                &mut writer,
                &request,
                format!("{}{rest}", route.upstream),
                "model",
                state,
            )?;
        } else {
            respond(
                &mut writer,
                404,
                "text/plain",
                b"gglint watch: no route for this path\n",
            )?;
        }
        if !keep_alive {
            break;
        }
    }
    Ok(())
}

fn respond(writer: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        writer,
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\r\n",
        reason(status),
        body.len()
    )?;
    writer.write_all(body)?;
    writer.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        502 => "Bad Gateway",
        _ => "",
    }
}

fn viewer(writer: &mut TcpStream, path: &str, state: &State) -> std::io::Result<()> {
    if path == "/" {
        return respond(writer, 200, "text/html; charset=utf-8", VIEWER.as_bytes());
    }
    if let Some(query) = path.strip_prefix("/__gglint/events") {
        let after = query
            .trim_start_matches('?')
            .split('&')
            .find_map(|pair| pair.strip_prefix("after="))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        return respond(writer, 200, "application/json", state.recorder.since(after).as_bytes());
    }
    if path == "/__gglint/session" {
        let session = json!({
            "dir": state.recorder.dir().display().to_string(),
            "live": !state.view_only,
            "version": env!("CARGO_PKG_VERSION"),
        });
        return respond(writer, 200, "application/json", session.to_string().as_bytes());
    }
    respond(writer, 404, "text/plain", b"not found\n")
}

/// Headers that belong to one connection, not passed on.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

fn forward(writer: &mut TcpStream, request: &Request, url: String, kind: &str, state: &State) -> std::io::Result<()> {
    let id = state.recorder.id();
    let started = Instant::now();
    let mut findings = Vec::new();
    let headers = record::headers(&request.headers, &mut findings);
    let body = record::body(&request.body, request.header("content-type"), &mut findings);
    state.recorder.push(json!({
        "type": "request",
        "id": id,
        "kind": kind,
        "method": request.method,
        "url": url,
        "headers": headers,
        "bytes": request.body.len(),
        "body": body,
        "findings": findings,
    }));

    let mut builder = ureq::http::Request::builder().method(request.method.as_str()).uri(&url);
    for (name, value) in &request.headers {
        if !HOP_BY_HOP.contains(&name.to_ascii_lowercase().as_str()) {
            builder = builder.header(name.as_str(), value.as_str());
        }
    }
    let sent = if request.body.is_empty() && matches!(request.method.as_str(), "GET" | "HEAD" | "DELETE" | "OPTIONS") {
        builder
            .body(())
            .map_err(std::io::Error::other)
            .and_then(|r| state.agent.run(r).map_err(std::io::Error::other))
    } else {
        builder
            .body(request.body.clone())
            .map_err(std::io::Error::other)
            .and_then(|r| state.agent.run(r).map_err(std::io::Error::other))
    };
    let response = match sent {
        Ok(response) => response,
        Err(error) => {
            state.recorder.push(json!({
                "type": "response", "id": id, "error": error.to_string(),
                "ms": started.elapsed().as_millis() as u64,
            }));
            let message = format!("gglint watch: couldn't reach {url}: {error}\n");
            return respond(writer, 502, "text/plain", message.as_bytes());
        }
    };

    let status = response.status().as_u16();
    let mut pairs = Vec::new();
    write!(
        writer,
        "HTTP/1.1 {status} {}\r\n",
        response.status().canonical_reason().unwrap_or("")
    )?;
    for (name, value) in response.headers() {
        let value = String::from_utf8_lossy(value.as_bytes()).to_string();
        pairs.push((name.as_str().to_string(), value.clone()));
        if !HOP_BY_HOP.contains(&name.as_str()) {
            write!(writer, "{name}: {value}\r\n")?;
        }
    }
    let no_body = request.method == "HEAD" || status == 204 || status == 304;
    if !no_body {
        write!(writer, "Transfer-Encoding: chunked\r\n")?;
    }
    write!(writer, "\r\n")?;
    writer.flush()?;

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let encoded = response
        .headers()
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .filter(|e| !e.eq_ignore_ascii_case("identity"))
        .map(str::to_string);
    let mut kept = Vec::new();
    let mut total = 0usize;
    if !no_body {
        let mut reader = response.into_body().into_reader();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            total += n;
            if kept.len() < 8 << 20 {
                kept.extend_from_slice(&buffer[..n]);
            }
            if write!(writer, "{n:x}\r\n")
                .and_then(|_| writer.write_all(&buffer[..n]))
                .and_then(|_| writer.write_all(b"\r\n"))
                .and_then(|_| writer.flush())
                .is_err()
            {
                break;
            }
        }
        let _ = writer.write_all(b"0\r\n\r\n");
        let _ = writer.flush();
    }

    let mut findings = Vec::new();
    let headers = record::headers(&pairs, &mut findings);
    let body = match &encoded {
        Some(encoding) => json!({ "gglint": format!("compressed ({encoding})"), "bytes": total }),
        None => record::body(&kept, content_type.as_deref(), &mut findings),
    };
    state.recorder.push(json!({
        "type": "response",
        "id": id,
        "status": status,
        "headers": headers,
        "bytes": total,
        "body": body,
        "ms": started.elapsed().as_millis() as u64,
    }));
    Ok(())
}

/// Passes a `CONNECT` tunnel through, recording the host and the amount each way.
fn tunnel(reader: BufReader<TcpStream>, mut writer: TcpStream, target: &str, state: &State) -> std::io::Result<()> {
    let id = state.recorder.id();
    let (host, port) = target.rsplit_once(':').unwrap_or((target, "443"));
    let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
    let port: u16 = port.parse().unwrap_or(443);
    let opened = now_ms();
    state.recorder.push(json!({
        "type": "connect", "id": id, "kind": "tunnel", "host": host, "port": port, "time": opened,
    }));
    let upstream = match TcpStream::connect((host.as_str(), port)) {
        Ok(upstream) => upstream,
        Err(error) => {
            state
                .recorder
                .push(json!({ "type": "close", "id": id, "error": error.to_string() }));
            return respond(
                &mut writer,
                502,
                "text/plain",
                format!("can't reach {target}: {error}\n").as_bytes(),
            );
        }
    };
    writer.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")?;
    writer.flush()?;

    // Bytes the client already sent after the CONNECT line, then the rest of the stream.
    let buffered = reader.buffer().to_vec();
    let client = reader.into_inner();
    let mut to_upstream = upstream.try_clone()?;
    to_upstream.write_all(&buffered)?;
    let mut from_client = client.try_clone()?;
    let up = std::thread::spawn(move || {
        let copied = std::io::copy(&mut from_client, &mut to_upstream).unwrap_or(0);
        let _ = to_upstream.shutdown(Shutdown::Write);
        copied
    });
    let mut from_upstream = upstream;
    let mut to_client = client;
    let down = std::io::copy(&mut from_upstream, &mut to_client).unwrap_or(0);
    let _ = to_client.shutdown(Shutdown::Write);
    let up = up.join().unwrap_or(0) + buffered.len() as u64;
    state.recorder.push(json!({
        "type": "close", "id": id, "sent": up, "received": down, "ms": now_ms().saturating_sub(opened),
    }));
    Ok(())
}
