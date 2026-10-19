//! The on-disk cassette format: one pretty-printed JSON file per session.

use std::fs;
use std::io;
use std::path::Path;

use anyhow::{Context, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::redact;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cassette {
    /// Format version, so future releases can migrate old cassettes.
    pub retake: u32,
    pub session: String,
    pub interactions: Vec<Interaction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interaction {
    pub request: RecordedRequest,
    pub response: RecordedResponse,
}

/// A request as stored. Headers are never stored, so API keys can't leak.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedRequest {
    pub method: String,
    pub upstream: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(flatten)]
    pub body: Payload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(flatten)]
    pub body: Payload,
    /// Server-sent events, in order. Concatenated, they are the exact original body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<StreamEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEvent {
    /// Milliseconds since the request was sent. Only stored with `--record-timing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_ms: Option<u64>,
    pub data: String,
}

/// A body, stored in the most readable form that still round-trips.
/// At most one field is set; none means an empty body.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Payload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
}

impl Payload {
    pub fn from_bytes(bytes: &[u8], content_type: Option<&str>) -> Self {
        if bytes.is_empty() {
            return Payload::default();
        }
        let maybe_json = content_type.is_none_or(|ct| ct.to_ascii_lowercase().contains("json"));
        if maybe_json && let Ok(value) = serde_json::from_slice::<Value>(bytes) {
            return Payload { json: Some(value), ..Default::default() };
        }
        match std::str::from_utf8(bytes) {
            Ok(text) => Payload { text: Some(text.to_string()), ..Default::default() },
            Err(_) => Payload { base64: Some(STANDARD.encode(bytes)), ..Default::default() },
        }
    }

    pub fn to_bytes(&self) -> anyhow::Result<Bytes> {
        if let Some(json) = &self.json {
            return Ok(serde_json::to_vec(json)?.into());
        }
        if let Some(text) = &self.text {
            return Ok(Bytes::from(text.clone()));
        }
        if let Some(b64) = &self.base64 {
            return Ok(STANDARD.decode(b64).context("cassette body is not valid base64")?.into());
        }
        Ok(Bytes::new())
    }

    /// Replaces anything that looks like a secret.
    pub fn redact(&mut self) {
        if let Some(json) = &mut self.json {
            redact::redact_value(json);
        }
        if let Some(text) = &mut self.text {
            *text = redact::redact_str(text).into_owned();
        }
    }
}

impl Cassette {
    pub fn new(session: &str) -> Self {
        Cassette { retake: FORMAT_VERSION, session: session.to_string(), interactions: Vec::new() }
    }

    /// Loads a cassette, or `None` if the file doesn't exist.
    pub fn load(path: &Path) -> anyhow::Result<Option<Self>> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let cassette: Cassette = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a valid retake cassette", path.display()))?;
        if cassette.retake > FORMAT_VERSION {
            bail!("{} was written by a newer retake (format {}); please upgrade", path.display(), cassette.retake);
        }
        Ok(Some(cassette))
    }

    /// Writes atomically, so an interrupted run never leaves a half-written file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trips() {
        let json = Payload::from_bytes(br#"{"b":1,"a":[true]}"#, Some("application/json"));
        assert!(json.json.is_some());
        assert_eq!(&json.to_bytes().unwrap()[..], br#"{"b":1,"a":[true]}"#);

        let text = Payload::from_bytes(b"hello", Some("text/plain"));
        assert_eq!(text.text.as_deref(), Some("hello"));

        let binary = Payload::from_bytes(&[0xff, 0x00, 0xfe], None);
        assert!(binary.base64.is_some());
        assert_eq!(&binary.to_bytes().unwrap()[..], &[0xff, 0x00, 0xfe]);

        assert_eq!(Payload::from_bytes(b"", None), Payload::default());
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/session.json");
        let mut cassette = Cassette::new("nested/session");
        cassette.interactions.push(Interaction {
            request: RecordedRequest {
                method: "POST".into(),
                upstream: "openai".into(),
                path: "/v1/chat/completions".into(),
                query: None,
                body: Payload::from_bytes(br#"{"model":"m"}"#, None),
            },
            response: RecordedResponse {
                status: 200,
                content_type: Some("text/event-stream".into()),
                duration_ms: None,
                body: Payload::default(),
                events: vec![StreamEvent { t_ms: None, data: "data: {}\n\n".into() }],
            },
        });
        cassette.save(&path).unwrap();
        assert_eq!(Cassette::load(&path).unwrap(), Some(cassette));
        assert_eq!(Cassette::load(&dir.path().join("missing.json")).unwrap(), None);
    }
}
