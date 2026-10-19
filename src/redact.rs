//! Detects and removes secrets before anything is written to a cassette.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

pub const PLACEHOLDER: &str = "[REDACTED]";

struct Rule {
    name: &'static str,
    re: Regex,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    [
        ("Anthropic API key", r"\bsk-ant-[A-Za-z0-9_-]{20,}"),
        ("OpenAI-style API key", r"\bsk-[A-Za-z0-9_-]{20,}"),
        ("Google API key", r"\bAIza[0-9A-Za-z_-]{35}"),
        ("GitHub token", r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})"),
        ("AWS access key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
        ("Slack token", r"\bxox[baprs]-[A-Za-z0-9-]{10,}"),
        ("Bearer token", r"(?i)\bbearer\s+[A-Za-z0-9._~+/-]{20,}=*"),
        ("Private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    ]
    .into_iter()
    .map(|(name, pattern)| Rule { name, re: Regex::new(pattern).expect("valid secret pattern") })
    .collect()
});

pub fn redact_str(s: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(s);
    for rule in RULES.iter() {
        if rule.re.is_match(&out) {
            out = Cow::Owned(rule.re.replace_all(&out, PLACEHOLDER).into_owned());
        }
    }
    out
}

pub fn redact_value(value: &mut Value) {
    match value {
        Value::String(s) => {
            if let Cow::Owned(redacted) = redact_str(s) {
                *s = redacted;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_value),
        Value::Object(map) => map.values_mut().for_each(redact_value),
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub line: usize,
}

/// Lists every line that looks like it contains a secret.
pub fn scan(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for rule in RULES.iter() {
            if rule.re.is_match(line) {
                findings.push(Finding { rule: rule.name, line: index + 1 });
                break;
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_secrets() {
        let input = "key sk-proj-abcdefghijklmnopqrstuvwx and sk-ant-api03-abcdefghijklmnopqrstuv";
        assert_eq!(redact_str(input), "key [REDACTED] and [REDACTED]");
        assert_eq!(redact_str("Authorization: Bearer abcdefghijklmnopqrstuvwxyz"), "Authorization: [REDACTED]");
        assert_eq!(redact_str("AIzaSyA1234567890abcdefghijklmnopqrstuv"), "[REDACTED]");
    }

    #[test]
    fn leaves_normal_text_alone() {
        let input = "Ask the model to summarise the task-list and skip-steps";
        assert!(matches!(redact_str(input), Cow::Borrowed(_)));
    }

    #[test]
    fn redacts_nested_json() {
        let mut value = serde_json::json!({"a": ["sk-abcdefghijklmnopqrstuvwxyz"], "n": 1});
        redact_value(&mut value);
        assert_eq!(value, serde_json::json!({"a": ["[REDACTED]"], "n": 1}));
    }

    #[test]
    fn scans_lines() {
        let text = "ok\n\"token\": \"ghp_abcdefghijklmnopqrstuvwxyz0123456789\"\nok";
        assert_eq!(scan(text), vec![Finding { rule: "GitHub token", line: 2 }]);
        assert!(scan("[REDACTED]").is_empty());
    }
}
