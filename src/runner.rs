//! Runs the proxy, optionally wrapping a command, and reports the outcome.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;

use crate::config::{Config, Mode};
use crate::proxy::{self, AppState};
use crate::store::Stats;

/// Placeholder keys so SDKs that insist on a key don't fail during replay.
const REPLAY_KEY: &str = "retake-replay-placeholder";

/// Returns the process exit code.
pub async fn run(config: Config, bind: SocketAddr, command: Vec<String>) -> anyhow::Result<i32> {
    let state = Arc::new(AppState::new(&config)?);
    let server = proxy::start(state.clone(), bind).await?;
    let base = format!("http://{}", server.addr);
    let env = child_env(&config, &base);

    let exit_code = if command.is_empty() {
        eprintln!("retake {} proxy listening on {base}", config.mode.as_str());
        eprintln!("Point your app at it with:");
        for (key, value) in &env {
            eprintln!("  {key}={value}");
        }
        eprintln!("Press Ctrl+C to stop.");
        tokio::signal::ctrl_c().await.context("waiting for Ctrl+C")?;
        0
    } else {
        run_child(&command, &env).await?
    };

    server.stop().await?;
    let stats = state.store.finish();
    report(&config, &stats);

    if exit_code == 0 && config.mode == Mode::Replay && !stats.misses.is_empty() {
        return Ok(1);
    }
    Ok(exit_code)
}

async fn run_child(command: &[String], env: &[(String, String)]) -> anyhow::Result<i32> {
    // `which` resolves `npm` to `npm.cmd` on Windows, which spawning by name doesn't.
    let program = which::which(&command[0]).with_context(|| format!("command not found: {}", command[0]))?;
    let mut child = tokio::process::Command::new(&program)
        .args(&command[1..])
        .envs(env.iter().map(|(k, v)| (k, v)))
        .spawn()
        .with_context(|| format!("starting {}", program.display()))?;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status?,
            // The child gets Ctrl+C too; let it shut down and report its own status.
            _ = tokio::signal::ctrl_c() => continue,
        }
    };
    Ok(status.code().unwrap_or(1))
}

/// Environment variables that point SDKs at the proxy.
pub fn child_env(config: &Config, base: &str) -> Vec<(String, String)> {
    let mut env = vec![
        ("RETAKE_URL".to_string(), base.to_string()),
        ("RETAKE_MODE".to_string(), config.mode.as_str().to_string()),
    ];
    for name in config.upstreams.keys() {
        let var = format!("RETAKE_{}_URL", name.to_ascii_uppercase().replace('-', "_"));
        env.push((var, format!("{base}/{name}")));
    }
    if config.upstreams.contains_key("openai") {
        // The OpenAI SDKs expect the `/v1` prefix in the base URL; Anthropic's don't.
        env.push(("OPENAI_BASE_URL".into(), format!("{base}/openai/v1")));
    }
    if config.upstreams.contains_key("anthropic") {
        env.push(("ANTHROPIC_BASE_URL".into(), format!("{base}/anthropic")));
    }
    if config.mode == Mode::Replay {
        for key in ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"] {
            if std::env::var_os(key).is_none() {
                env.push((key.into(), REPLAY_KEY.into()));
            }
        }
    }
    env
}

fn report(config: &Config, stats: &Stats) {
    eprintln!("retake: {} replayed, {} recorded, {} unmatched", stats.replayed, stats.recorded, stats.misses.len());
    if stats.recorded > 0 {
        eprintln!("retake: cassettes saved in {}", config.cassette_dir.display());
    }
    for miss in &stats.misses {
        let first_line = miss.request.lines().next().unwrap_or_default();
        eprintln!("retake: ✗ session `{}`: no recording for {first_line}", miss.session);
    }
    if !stats.misses.is_empty() {
        eprintln!("retake: re-record with `retake record`, or use `retake auto` to record only what's missing");
    }
    for (session, count) in &stats.unused {
        eprintln!("retake: ! session `{session}`: {count} recorded interaction(s) were never replayed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_points_sdks_at_the_proxy() {
        let mut config = Config::new(Mode::Record, "cassettes");
        config.upstreams.insert("my-llm".into(), "http://localhost:8000".into());
        let env = child_env(&config, "http://127.0.0.1:9000");
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("OPENAI_BASE_URL"), Some("http://127.0.0.1:9000/openai/v1"));
        assert_eq!(get("ANTHROPIC_BASE_URL"), Some("http://127.0.0.1:9000/anthropic"));
        assert_eq!(get("RETAKE_MY_LLM_URL"), Some("http://127.0.0.1:9000/my-llm"));
        assert_eq!(get("RETAKE_MODE"), Some("record"));
        assert_eq!(get("OPENAI_API_KEY"), None);
    }
}
