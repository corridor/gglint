//! `gglint watch`: start a coding agent with its traffic routed through gglint on localhost,
//! record what it sends, and show it on a local page.
//!
//! The agent's model requests come to gglint because its base URL is pointed here, a setting
//! each agent supports (`ANTHROPIC_BASE_URL`; a Codex model provider's `base_url`). Its other
//! connections come through `HTTPS_PROXY`, where only the host is visible. Nothing is
//! decrypted, and no certificate is installed.
//!
//! This is the one gglint command that starts programs and reaches the network: the agent,
//! and the APIs the agent itself would have called.

pub mod proxy;
pub mod record;

use std::path::{Path, PathBuf};

/// An agent `gglint watch` knows how to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub fn parse(name: &str) -> Option<Agent> {
        match name {
            "claude" | "claude-code" => Some(Agent::Claude),
            "codex" => Some(Agent::Codex),
            _ => None,
        }
    }

    pub fn program(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

/// The program, arguments and environment to start.
#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The environment every watched program gets: the proxy, and where the viewer is.
fn proxy_env(base: &str) -> Vec<(String, String)> {
    let mut env = Vec::new();
    for name in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        env.push((name.to_string(), base.to_string()));
    }
    env.push(("GGLINT_WATCH_URL".to_string(), base.to_string()));
    env
}

/// Whether Codex signs in with a ChatGPT account (its requests go to chatgpt.com) rather than
/// an API key (api.openai.com), from `auth.json` in `codex_home`.
pub fn codex_uses_chatgpt(codex_home: &Path) -> bool {
    std::fs::read_to_string(codex_home.join("auth.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|auth| {
            auth.get("auth_mode").and_then(|mode| mode.as_str()) == Some("chatgpt")
                || (auth.get("auth_mode").is_none() && auth.get("tokens").is_some_and(|t| !t.is_null()))
        })
}

/// How to start `agent` with its traffic sent to `base` (`http://127.0.0.1:PORT`).
pub fn launch(agent: Agent, base: &str, args: &[String], codex_chatgpt: bool) -> Launch {
    let mut env = proxy_env(base);
    let mut all_args = Vec::new();
    match agent {
        Agent::Claude => {
            env.push(("ANTHROPIC_BASE_URL".to_string(), format!("{base}/anthropic")));
        }
        Agent::Codex => {
            let url = if codex_chatgpt {
                format!("{base}/chatgpt/backend-api/codex")
            } else {
                format!("{base}/openai/v1")
            };
            all_args.push("-c".to_string());
            all_args.push(format!(
                "model_providers.gglint_watch={{name=\"OpenAI (through gglint watch)\",base_url=\"{url}\",\
                 wire_api=\"responses\",requires_openai_auth=true}}"
            ));
            all_args.push("-c".to_string());
            all_args.push("model_provider=\"gglint_watch\"".to_string());
        }
    }
    all_args.extend(args.iter().cloned());
    Launch {
        program: agent.program().to_string(),
        args: all_args,
        env,
    }
}

/// How to start any other command: every base URL gglint knows, and the proxy.
pub fn launch_command(command: &[String], base: &str) -> Launch {
    let mut env = proxy_env(base);
    env.push(("ANTHROPIC_BASE_URL".to_string(), format!("{base}/anthropic")));
    env.push(("OPENAI_BASE_URL".to_string(), format!("{base}/openai/v1")));
    Launch {
        program: command[0].clone(),
        args: command[1..].to_vec(),
        env,
    }
}

/// Where a new recording goes: `~/.gglint/watch/2026-11-25T10-36-44-claude`.
pub fn recording_dir(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".gglint")
        .join("watch")
        .join(format!("{}-{name}", timestamp()))
}

/// The UTC time now, as `2026-11-25T10-36-44`.
fn timestamp() -> String {
    let seconds = record::now_ms() / 1000;
    let (days, rest) = ((seconds / 86_400) as i64, seconds % 86_400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}
