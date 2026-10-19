//! Decides whether two requests are "the same" for replay.
//!
//! JSON bodies are compared after removing ignored fields and sorting object
//! keys, so key order and whitespace never cause a mismatch.

use serde_json::{Map, Value};

use crate::cassette::{Payload, RecordedRequest};

#[derive(Debug, Clone, Default)]
pub struct Matcher {
    ignore: Vec<Vec<String>>,
}

impl Matcher {
    /// `ignore_fields` are dotted paths such as `metadata.request_id`.
    /// Arrays are traversed transparently, so `messages.id` matches every message.
    pub fn new(ignore_fields: &[String]) -> Self {
        let ignore = ignore_fields
            .iter()
            .filter(|f| !f.is_empty())
            .map(|f| f.split('.').map(str::to_string).collect())
            .collect();
        Matcher { ignore }
    }

    /// A canonical string: equal keys mean the requests match.
    pub fn key(&self, request: &RecordedRequest) -> String {
        format!("{}\n{}", request_line(request), self.body(&request.body, false))
    }

    /// A human-readable version of the key, used for divergence diffs.
    pub fn describe(&self, request: &RecordedRequest) -> String {
        let body = self.body(&request.body, true);
        if body.is_empty() {
            format!("{}\n", request_line(request))
        } else {
            format!("{}\n{}\n", request_line(request), body)
        }
    }

    fn body(&self, payload: &Payload, pretty: bool) -> String {
        if let Some(json) = &payload.json {
            let mut json = json.clone();
            for path in &self.ignore {
                remove_path(&mut json, path);
            }
            let json = canonical(&json);
            let rendered = if pretty { serde_json::to_string_pretty(&json) } else { serde_json::to_string(&json) };
            return rendered.unwrap_or_default();
        }
        if let Some(text) = &payload.text {
            return text.clone();
        }
        if let Some(b64) = &payload.base64 {
            return format!("<binary body, base64 {b64}>");
        }
        String::new()
    }
}

fn request_line(request: &RecordedRequest) -> String {
    match &request.query {
        Some(query) => format!("{} /{}{}?{}", request.method, request.upstream, request.path, query),
        None => format!("{} /{}{}", request.method, request.upstream, request.path),
    }
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonical(&map[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn remove_path(value: &mut Value, path: &[String]) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|item| remove_path(item, path)),
        Value::Object(map) => match path {
            [] => {}
            [last] => {
                map.remove(last);
            }
            [head, rest @ ..] => {
                if let Some(child) = map.get_mut(head) {
                    remove_path(child, rest);
                }
            }
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(body: &str) -> RecordedRequest {
        RecordedRequest {
            method: "POST".into(),
            upstream: "openai".into(),
            path: "/v1/chat/completions".into(),
            query: None,
            body: Payload::from_bytes(body.as_bytes(), Some("application/json")),
        }
    }

    #[test]
    fn key_order_and_whitespace_do_not_matter() {
        let m = Matcher::default();
        assert_eq!(
            m.key(&request(r#"{"a":1,"b":{"y":2,"x":1}}"#)),
            m.key(&request(r#"{ "b": {"x":1,"y":2}, "a": 1 }"#))
        );
        assert_ne!(m.key(&request(r#"{"a":1}"#)), m.key(&request(r#"{"a":2}"#)));
    }

    #[test]
    fn ignored_fields_are_removed() {
        let m = Matcher::new(&["metadata.request_id".into(), "messages.id".into()]);
        let a = request(r#"{"metadata":{"request_id":"1","user":"u"},"messages":[{"id":"a","content":"hi"}]}"#);
        let b = request(r#"{"metadata":{"request_id":"2","user":"u"},"messages":[{"id":"b","content":"hi"}]}"#);
        assert_eq!(m.key(&a), m.key(&b));
        let c = request(r#"{"metadata":{"request_id":"2","user":"other"},"messages":[{"id":"b","content":"hi"}]}"#);
        assert_ne!(m.key(&a), m.key(&c));
    }

    #[test]
    fn path_and_method_matter() {
        let m = Matcher::default();
        let mut other = request("{}");
        other.path = "/v1/embeddings".into();
        assert_ne!(m.key(&request("{}")), m.key(&other));
    }
}
