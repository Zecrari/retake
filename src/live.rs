//! A feed of what the proxy is doing, for the dashboard.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::{broadcast, watch};

/// Events kept for dashboards that connect after the run started.
const HISTORY: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Served from a cassette.
    Replayed,
    /// Forwarded upstream and saved.
    Recorded,
    /// No recording matched (replay mode).
    Missed,
    /// Forwarded upstream but not saved, e.g. a 5xx response.
    Passed,
    /// The upstream couldn't be reached.
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveEvent {
    pub id: u64,
    /// Unix time in milliseconds.
    pub at: u64,
    pub kind: Kind,
    pub session: String,
    pub method: String,
    /// `/<upstream><path>`, as the client sent it.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Index of the interaction in the session's cassette.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    /// Diff or error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub struct Live {
    tx: broadcast::Sender<LiveEvent>,
    next_id: AtomicU64,
    history: Mutex<VecDeque<LiveEvent>>,
    closed: watch::Sender<bool>,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            tx: broadcast::channel(256).0,
            next_id: AtomicU64::new(1),
            history: Mutex::new(VecDeque::new()),
            closed: watch::channel(false).0,
        }
    }
}

/// What happened to one request; `Live::emit` fills in the id and time.
pub struct Update<'a> {
    pub kind: Kind,
    pub session: &'a str,
    pub method: &'a str,
    pub path: String,
    pub status: Option<u16>,
    pub index: Option<usize>,
    pub detail: Option<String>,
}

impl Live {
    pub fn emit(&self, update: Update<'_>) {
        let event = LiveEvent {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default(),
            kind: update.kind,
            session: update.session.to_string(),
            method: update.method.to_string(),
            path: update.path,
            status: update.status,
            index: update.index,
            detail: update.detail,
        };
        {
            let mut history = self.history.lock().unwrap_or_else(|p| p.into_inner());
            if history.len() == HISTORY {
                history.pop_front();
            }
            history.push_back(event.clone());
        }
        // No subscribers is fine: nobody has the dashboard open.
        let _ = self.tx.send(event);
    }

    /// Past events plus a receiver for new ones. Subscribing while holding the
    /// history lock means no event is missed or delivered twice.
    pub fn subscribe(&self) -> (Vec<LiveEvent>, broadcast::Receiver<LiveEvent>) {
        let history = self.history.lock().unwrap_or_else(|p| p.into_inner());
        let receiver = self.tx.subscribe();
        (history.iter().cloned().collect(), receiver)
    }

    /// Ends every open feed, so dashboards don't hold up shutdown.
    pub fn close(&self) {
        self.closed.send_replace(true);
    }

    /// Completes once `close` has been called.
    pub fn closed(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut rx = self.closed.subscribe();
        async move {
            let _ = rx.wait_for(|closed| *closed).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(kind: Kind) -> Update<'static> {
        Update {
            kind,
            session: "s",
            method: "POST",
            path: "/openai/v1/x".into(),
            status: Some(200),
            index: None,
            detail: None,
        }
    }

    #[test]
    fn keeps_history_and_streams_new_events() {
        let live = Live::default();
        live.emit(update(Kind::Recorded));
        let (history, mut rx) = live.subscribe();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, 1);
        live.emit(update(Kind::Replayed));
        let next = rx.try_recv().unwrap();
        assert_eq!((next.id, next.kind), (2, Kind::Replayed));
    }

    #[test]
    fn history_is_bounded() {
        let live = Live::default();
        for _ in 0..HISTORY + 10 {
            live.emit(update(Kind::Replayed));
        }
        let (history, _) = live.subscribe();
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history[0].id, 11);
    }
}
