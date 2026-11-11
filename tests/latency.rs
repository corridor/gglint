//! The latency budget for `gglint mcp`: reading and comparing every coding agent's MCP config.
//!
//! Timing needs an optimized build and the machine to itself, so this is ignored by
//! default:
//!
//! ```sh
//! cargo test --release --test latency -- --ignored --nocapture
//! ```

mod common;

use std::time::{Duration, Instant};

use common::TempDir;
use gglint::harness::{compare_mcp, read_mcp, used_harnesses};

fn slowest(times: impl IntoIterator<Item = Duration>) -> Duration {
    times.into_iter().max().unwrap_or_default()
}

/// Every harness's MCP config, as in a repository set up for all of them.
fn mcp_repo() -> TempDir {
    let dir = TempDir::new();
    let mut json = String::from("{\n  \"mcpServers\": {\n");
    let mut opencode = String::from("{\n  \"mcp\": {\n");
    let mut toml = String::new();
    for n in 0..20 {
        opencode.push_str(&format!(
            "    \"server{n}\": {{ \"type\": \"local\", \"command\": [\"npx\", \"-y\", \"server{n}@1.0.0\"], \"environment\": {{ \"KEY\": \"{{env:KEY}}\" }} }},\n"
        ));
        json.push_str(&format!(
            "    \"server{n}\": {{ \"command\": \"npx\", \"args\": [\"-y\", \"server{n}@1.0.0\"], \"env\": {{ \"KEY\": \"${{KEY}}\" }} }},\n"
        ));
        toml.push_str(&format!("[mcp_servers.server{n}]\ncommand = \"npx\"\nargs = [\"-y\", \"server{n}@1.0.0\"]\nenv_vars = [\"KEY\"]\n\n"));
    }
    let docs = "    \"docs\": { \"url\": \"https://docs.example.com/mcp\" }\n  }\n}\n";
    json.push_str(docs);
    opencode.push_str(docs);
    dir.write(".mcp.json", &json);
    dir.write(".cursor/mcp.json", &json);
    dir.write(".codex/config.toml", toml);
    dir.write("opencode.json", opencode);
    dir
}

#[test]
#[ignore = "timing; run with --release"]
fn reads_and_compares_mcp_configs_within_budget() {
    let dir = mcp_repo();
    assert_eq!(read_mcp(dir.path()).len(), 4);

    let times: Vec<Duration> = (0..200)
        .map(|_| {
            let start = Instant::now();
            let configs = read_mcp(dir.path());
            let findings = compare_mcp(&configs, &used_harnesses(dir.path()));
            let elapsed = start.elapsed();
            assert_eq!(configs.iter().map(|config| config.servers.len()).sum::<usize>(), 83);
            // Codex is the only one without `docs`.
            assert_eq!(findings.len(), 1, "{findings:?}");
            elapsed
        })
        .collect();
    let slowest = slowest(times);
    eprintln!("slowest read and comparison of 4 files, 83 servers: {slowest:?}");
    assert!(slowest < Duration::from_millis(1), "slowest: {slowest:?}");
}
