//! Splits a server-sent event stream into events for storage.

use crate::cassette::StreamEvent;
use crate::redact;

pub fn is_event_stream(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| ct.trim_start().to_ascii_lowercase().starts_with("text/event-stream"))
}

/// Collects network chunks and cuts them at event boundaries, so each stored
/// event is a complete, readable SSE event no matter how the bytes arrived.
#[derive(Debug, Default)]
pub struct EventSplitter {
    pending: Vec<u8>,
    events: Vec<(Option<u64>, Vec<u8>)>,
}

impl EventSplitter {
    pub fn push(&mut self, chunk: &[u8], t_ms: Option<u64>) {
        self.pending.extend_from_slice(chunk);
        while let Some(end) = boundary(&self.pending) {
            let event: Vec<u8> = self.pending.drain(..end).collect();
            self.events.push((t_ms, event));
        }
    }

    /// Returns the redacted events, or the raw bytes if the stream wasn't UTF-8.
    pub fn finish(mut self, t_ms: Option<u64>) -> Result<Vec<StreamEvent>, Vec<u8>> {
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            self.events.push((t_ms, rest));
        }
        if self.events.iter().any(|(_, bytes)| std::str::from_utf8(bytes).is_err()) {
            return Err(self.events.into_iter().flat_map(|(_, bytes)| bytes).collect());
        }
        Ok(self
            .events
            .into_iter()
            .map(|(t_ms, bytes)| {
                let data = String::from_utf8(bytes).expect("checked above");
                StreamEvent { t_ms, data: redact::redact_str(&data).into_owned() }
            })
            .collect())
    }
}

/// End index (exclusive) of the first complete event in `buf`.
fn boundary(buf: &[u8]) -> Option<usize> {
    [&b"\r\n\r\n"[..], b"\n\n", b"\r\r"]
        .iter()
        .filter_map(|sep| buf.windows(sep.len()).position(|w| w == *sep).map(|i| i + sep.len()))
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_event_streams() {
        assert!(is_event_stream(Some("text/event-stream; charset=utf-8")));
        assert!(!is_event_stream(Some("application/json")));
        assert!(!is_event_stream(None));
    }

    #[test]
    fn splits_events_across_chunks() {
        let mut s = EventSplitter::default();
        s.push(b"event: a\ndata: 1\n", None);
        s.push(b"\ndata: 2\n\ndata: [DO", None);
        s.push(b"NE]\n\n", None);
        let events = s.finish(None).unwrap();
        let data: Vec<&str> = events.iter().map(|e| e.data.as_str()).collect();
        assert_eq!(data, ["event: a\ndata: 1\n\n", "data: 2\n\n", "data: [DONE]\n\n"]);
    }

    #[test]
    fn keeps_trailing_bytes_and_crlf() {
        let mut s = EventSplitter::default();
        s.push(b"data: 1\r\n\r\ndata: tail", Some(5));
        let events = s.finish(Some(9)).unwrap();
        assert_eq!(events[0].data, "data: 1\r\n\r\n");
        assert_eq!(events[1].data, "data: tail");
        assert_eq!(events[1].t_ms, Some(9));
    }

    #[test]
    fn falls_back_to_raw_bytes() {
        let mut s = EventSplitter::default();
        s.push(&[0xff, b'\n', b'\n'], None);
        assert_eq!(s.finish(None).unwrap_err(), vec![0xff, b'\n', b'\n']);
    }
}
