//! `gglint watch`: records what a program sends, keeps credentials out of the recording, and
//! passes everything on unchanged.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use common::TempDir;
use gglint::watch::record::emails;
use gglint::watch::{Agent, launch};

/// Answers one request with a small JSON body, keeping what it received.
fn fake_api() -> (u16, Arc<Mutex<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(String::new()));
    let keep = Arc::clone(&seen);
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut head = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = n.trim().parse().unwrap();
            }
            head.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        *keep.lock().unwrap() = head + &String::from_utf8(body).unwrap();
        let answer = r#"{"content":[{"type":"text","text":"hi"}]}"#;
        let mut stream = stream;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{answer}",
            answer.len()
        )
        .unwrap();
    });
    (port, seen)
}

#[cfg(unix)]
#[test]
fn records_a_request_without_its_credentials_and_forwards_it_unchanged() {
    let dir = TempDir::new();
    let (port, seen) = fake_api();
    let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
    let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB";
    dir.write(
        "body.json",
        format!(
            r#"{{"model":"m","system":"You are helpful. Contact me@example.com","messages":[{{"role":"user","content":"my token is {token}"}}]}}"#
        ),
    );
    let script = format!(
        r#"curl -s -o "{out}" -H "x-api-key: {key}" -H "content-type: application/json" --data-binary @"{body}" "$ANTHROPIC_BASE_URL/v1/messages" && curl -s -o "{page}" "$GGLINT_WATCH_URL/"; exit 3"#,
        out = dir.path().join("answer.json").display(),
        body = dir.path().join("body.json").display(),
        page = dir.path().join("page.html").display(),
    );
    let recording = dir.path().join("recording");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gglint"))
        .args(["watch", "--no-open", "--dir"])
        .arg(&recording)
        .args([
            "--upstream",
            &format!("anthropic=http://127.0.0.1:{port}"),
            "--",
            "sh",
            "-c",
            &script,
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The API got the request exactly as sent, key and all.
    let received = seen.lock().unwrap().clone();
    assert!(received.starts_with("POST /v1/messages "), "{received}");
    assert!(
        received.to_ascii_lowercase().contains(&format!("x-api-key: {key}")),
        "{received}"
    );
    assert!(received.contains(token));
    // And the program got the API's answer.
    let answer = std::fs::read_to_string(dir.path().join("answer.json")).unwrap();
    assert_eq!(answer, r#"{"content":[{"type":"text","text":"hi"}]}"#);
    // The viewer was served.
    assert!(
        std::fs::read_to_string(dir.path().join("page.html"))
            .unwrap()
            .contains("<title>gglint watch</title>")
    );

    // The recording has neither the key nor the token, and says what was removed.
    let events = std::fs::read_to_string(recording.join("events.jsonl")).unwrap();
    assert!(!events.contains(key), "{events}");
    assert!(!events.contains(token), "{events}");
    let events: Vec<serde_json::Value> = events.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let request = events.iter().find(|e| e["type"] == "request").unwrap();
    assert_eq!(request["kind"], "model");
    assert_eq!(request["url"], format!("http://127.0.0.1:{port}/v1/messages"));
    assert_eq!(
        request["headers"]["x-api-key"][0],
        format!("[removed by gglint: {} characters]", key.len())
    );
    assert_eq!(
        request["body"]["messages"][0]["content"],
        "my token is [GitHub token removed by gglint]"
    );
    assert_eq!(request["body"]["system"], "You are helpful. Contact me@example.com");
    let findings = request["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f["kind"] == "secret" && f["what"] == "GitHub token")
    );
    assert!(
        findings
            .iter()
            .any(|f| f["kind"] == "email" && f["what"] == "me@example.com")
    );
    let response = events.iter().find(|e| e["type"] == "response").unwrap();
    assert_eq!(response["status"], 200);
    assert_eq!(response["body"]["content"][0]["text"], "hi");
}

#[test]
fn codex_is_pointed_at_gglint_for_its_sign_in() {
    let base = "http://127.0.0.1:9";
    let chatgpt = launch(Agent::Codex, base, &["exec".to_string()], true);
    assert_eq!(chatgpt.program, "codex");
    assert!(chatgpt.args[1].contains("base_url=\"http://127.0.0.1:9/chatgpt/backend-api/codex\""));
    assert!(chatgpt.args[1].contains("requires_openai_auth=true"));
    assert_eq!(chatgpt.args[3], "model_provider=\"gglint_watch\"");
    assert_eq!(chatgpt.args.last().unwrap(), "exec");
    let api_key = launch(Agent::Codex, base, &[], false);
    assert!(api_key.args[1].contains("base_url=\"http://127.0.0.1:9/openai/v1\""));

    let claude = launch(Agent::Claude, base, &["-p".to_string(), "hi".to_string()], false);
    assert_eq!(claude.args, ["-p", "hi"]);
    assert!(claude.env.contains(&(
        "ANTHROPIC_BASE_URL".to_string(),
        "http://127.0.0.1:9/anthropic".to_string()
    )));
    assert!(claude.env.contains(&("HTTPS_PROXY".to_string(), base.to_string())));
}

#[test]
fn finds_email_addresses_but_not_package_scopes_or_versions() {
    assert_eq!(
        emails("mail me@example.com or a.b+c@mail.example.co.uk."),
        ["me@example.com", "a.b+c@mail.example.co.uk"]
    );
    assert!(emails("@angular/cli and pkg@1.2.3 and user@localhost").is_empty());
}

#[test]
fn a_private_key_is_found_to_its_end() {
    let text = "key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEabc\n-----END RSA PRIVATE KEY-----\nafter";
    let found = gglint::secrets::find_secrets(text);
    assert_eq!(found.len(), 1);
    let (range, kind) = &found[0];
    assert_eq!(kind, "private key");
    assert_eq!(
        &text[range.clone()],
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEabc\n-----END RSA PRIVATE KEY-----"
    );
}
