use std::collections::BTreeMap;
use std::path::PathBuf;

/// How the proxy treats each incoming request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// Forward every request upstream and save it, replacing the session's cassette.
    Record,
    /// Serve every request from the cassette. Unrecorded requests fail.
    Replay,
    /// Replay recorded requests; forward and append anything new.
    Auto,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Record => "record",
            Mode::Replay => "replay",
            Mode::Auto => "auto",
        }
    }
}

/// How replayed streams are paced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Timing {
    /// Send every recorded event immediately.
    #[default]
    Fast,
    /// Reproduce recorded event timing (needs `--record-timing` when recording).
    Realtime,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub mode: Mode,
    pub cassette_dir: PathBuf,
    pub default_session: String,
    /// Upstream name -> base URL without a trailing slash.
    pub upstreams: BTreeMap<String, String>,
    /// Dotted JSON paths removed from request bodies before matching.
    pub ignore_fields: Vec<String>,
    pub timing: Timing,
    pub record_timing: bool,
    pub record_errors: bool,
}

impl Config {
    pub fn new(mode: Mode, cassette_dir: impl Into<PathBuf>) -> Self {
        Config {
            mode,
            cassette_dir: cassette_dir.into(),
            default_session: "default".into(),
            upstreams: default_upstreams(),
            ignore_fields: Vec::new(),
            timing: Timing::Fast,
            record_timing: false,
            record_errors: false,
        }
    }
}

pub fn default_upstreams() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("anthropic".to_string(), "https://api.anthropic.com".to_string()),
        ("openai".to_string(), "https://api.openai.com".to_string()),
    ])
}

/// Parses `NAME=URL` as given to `--upstream`.
pub fn parse_upstream(s: &str) -> Result<(String, String), String> {
    let (name, url) = s.split_once('=').ok_or_else(|| format!("expected NAME=URL, got `{s}`"))?;
    let name = name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err(format!("upstream name `{name}` must use lowercase letters, digits, `-` or `_`"));
    }
    let url = url.trim().trim_end_matches('/');
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("upstream URL `{url}` must start with http:// or https://"));
    }
    Ok((name.to_string(), url.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_upstreams() {
        assert_eq!(
            parse_upstream("ollama=http://localhost:11434/").unwrap(),
            ("ollama".into(), "http://localhost:11434".into())
        );
        assert!(parse_upstream("no-equals").is_err());
        assert!(parse_upstream("Bad=http://x").is_err());
        assert!(parse_upstream("ok=ftp://x").is_err());
    }
}
