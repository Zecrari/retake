//! Sessions loaded from and saved to cassette files, and which interactions
//! have been replayed so far.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use similar::TextDiff;

use crate::cassette::{Cassette, Interaction, RecordedRequest, RecordedResponse};
use crate::config::Mode;
use crate::matcher::Matcher;

pub enum Lookup {
    /// `index` is the matched interaction's position in the cassette.
    Hit {
        index: usize,
        response: Box<RecordedResponse>,
    },
    Miss(Miss),
}

/// A request with no matching recording.
#[derive(Debug, Clone)]
pub struct Miss {
    pub session: String,
    pub request: String,
    /// 1-based step of the closest unused recording, if any.
    pub closest_step: Option<usize>,
    /// Unified diff from the closest recording to the actual request.
    pub diff: Option<String>,
}

impl Miss {
    pub fn message(&self) -> String {
        let first_line = self.request.lines().next().unwrap_or_default();
        let mut message = format!("no recording in session `{}` matches {first_line}", self.session);
        match (&self.closest_step, &self.diff) {
            (Some(step), Some(diff)) => {
                message.push_str(&format!(" (closest: step {step})\n{diff}"));
            }
            _ => message.push_str(" (the session has no unused recordings left)"),
        }
        message
    }
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub replayed: usize,
    pub recorded: usize,
    pub misses: Vec<Miss>,
    /// Session -> recorded interactions that were never replayed.
    pub unused: BTreeMap<String, usize>,
}

struct Session {
    path: PathBuf,
    cassette: Cassette,
    keys: Vec<String>,
    used: Vec<bool>,
    loaded: usize,
}

struct Inner {
    sessions: HashMap<String, Session>,
    stats: Stats,
}

pub struct Store {
    mode: Mode,
    dir: PathBuf,
    matcher: Matcher,
    inner: Mutex<Inner>,
}

impl Store {
    pub fn new(mode: Mode, dir: impl Into<PathBuf>, matcher: Matcher) -> Self {
        Store {
            mode,
            dir: dir.into(),
            matcher,
            inner: Mutex::new(Inner { sessions: HashMap::new(), stats: Stats::default() }),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn session_path(&self, session: &str) -> PathBuf {
        session_path(&self.dir, session)
    }

    pub fn lookup(&self, session: &str, request: &RecordedRequest) -> anyhow::Result<Lookup> {
        let key = self.matcher.key(request);
        let mut inner = self.lock();
        let Inner { sessions, stats } = &mut *inner;
        let state = self.session(sessions, session)?;

        if let Some(i) = (0..state.keys.len()).find(|&i| !state.used[i] && state.keys[i] == key) {
            state.used[i] = true;
            stats.replayed += 1;
            return Ok(Lookup::Hit { index: i, response: Box::new(state.cassette.interactions[i].response.clone()) });
        }

        // Prefer an unused recording to the same endpoint: that's almost always
        // the step the caller meant, so its diff is the useful one.
        let unused = |i: &usize| !state.used[*i];
        let same_endpoint = (0..state.keys.len()).filter(unused).find(|&i| {
            let r = &state.cassette.interactions[i].request;
            r.method == request.method && r.upstream == request.upstream && r.path == request.path
        });
        let closest = same_endpoint.or_else(|| (0..state.keys.len()).find(unused));
        let actual = self.matcher.describe(request);
        let diff = closest.map(|i| {
            let expected = self.matcher.describe(&state.cassette.interactions[i].request);
            TextDiff::from_lines(&expected, &actual)
                .unified_diff()
                .context_radius(3)
                .header(&format!("recorded (step {})", i + 1), "actual")
                .to_string()
        });
        Ok(Lookup::Miss(Miss {
            session: session.to_string(),
            request: actual,
            closest_step: closest.map(|i| i + 1),
            diff,
        }))
    }

    pub fn note_miss(&self, miss: &Miss) {
        self.lock().stats.misses.push(miss.clone());
    }

    /// Appends an interaction, saves the session's cassette immediately, and
    /// returns the interaction's index.
    pub fn record(&self, session: &str, interaction: Interaction) -> anyhow::Result<usize> {
        let key = self.matcher.key(&interaction.request);
        let mut inner = self.lock();
        let Inner { sessions, stats } = &mut *inner;
        let state = self.session(sessions, session)?;
        state.cassette.interactions.push(interaction);
        state.keys.push(key);
        // Already "used": an identical later request in this run is forwarded and
        // recorded too, so the next replay sees exactly what happened this time.
        state.used.push(true);
        state.cassette.save(&state.path)?;
        stats.recorded += 1;
        Ok(state.cassette.interactions.len() - 1)
    }

    /// Final statistics, including recordings that were never replayed.
    pub fn finish(&self) -> Stats {
        let inner = self.lock();
        let mut stats = inner.stats.clone();
        if self.mode != Mode::Record {
            for (name, state) in &inner.sessions {
                let unused = state.used[..state.loaded].iter().filter(|u| !**u).count();
                if unused > 0 {
                    stats.unused.insert(name.clone(), unused);
                }
            }
        }
        stats
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic while holding the lock can't leave the data half-updated in a
        // way that matters more than losing the whole run, so keep going.
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn session<'a>(&self, sessions: &'a mut HashMap<String, Session>, name: &str) -> anyhow::Result<&'a mut Session> {
        if !sessions.contains_key(name) {
            let path = self.session_path(name);
            let cassette = match self.mode {
                // Recording starts each session from scratch.
                Mode::Record => Cassette::new(name),
                Mode::Replay | Mode::Auto => Cassette::load(&path)?.unwrap_or_else(|| Cassette::new(name)),
            };
            let keys: Vec<String> = cassette.interactions.iter().map(|i| self.matcher.key(&i.request)).collect();
            let loaded = keys.len();
            sessions.insert(name.to_string(), Session { path, cassette, used: vec![false; loaded], keys, loaded });
        }
        Ok(sessions.get_mut(name).expect("inserted above"))
    }
}

pub fn session_path(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("{session}.json"))
}

/// Session names become file paths, so keep them to a safe character set.
pub fn validate_session(name: &str) -> Result<(), String> {
    let valid_chars = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    let valid_parts = name.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    if name.is_empty() || name.len() > 200 || !valid_chars || !valid_parts {
        return Err(format!(
            "invalid session name `{name}`: use letters, digits, `-`, `_`, `.` and `/` (for folders), without empty, `.` or `..` parts"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cassette::Payload;

    fn request(body: &str) -> RecordedRequest {
        RecordedRequest {
            method: "POST".into(),
            upstream: "openai".into(),
            path: "/v1/chat/completions".into(),
            query: None,
            body: Payload::from_bytes(body.as_bytes(), Some("application/json")),
        }
    }

    fn response(status: u16) -> RecordedResponse {
        RecordedResponse { status, content_type: None, duration_ms: None, body: Payload::default(), events: vec![] }
    }

    #[test]
    fn session_names() {
        assert!(validate_session("default").is_ok());
        assert!(validate_session("agents/refund-flow_v2.1").is_ok());
        for bad in ["", "../x", "a//b", "/abs", "a/./b", "sp ace", "a\\b"] {
            assert!(validate_session(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn replays_identical_requests_in_order_then_misses() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = Store::new(Mode::Record, dir.path(), Matcher::default());
        recorder.record("s", Interaction { request: request(r#"{"n":1}"#), response: response(201) }).unwrap();
        recorder.record("s", Interaction { request: request(r#"{"n":1}"#), response: response(202) }).unwrap();

        let replayer = Store::new(Mode::Replay, dir.path(), Matcher::default());
        let status = |l: Lookup| match l {
            Lookup::Hit { response, .. } => response.status,
            Lookup::Miss(_) => 0,
        };
        assert_eq!(status(replayer.lookup("s", &request(r#"{"n":1}"#)).unwrap()), 201);
        assert_eq!(status(replayer.lookup("s", &request(r#"{"n":1}"#)).unwrap()), 202);
        let Lookup::Miss(miss) = replayer.lookup("s", &request(r#"{"n":1}"#)).unwrap() else {
            panic!("expected a miss")
        };
        assert_eq!(miss.closest_step, None);
        assert_eq!(replayer.finish().replayed, 2);
    }

    #[test]
    fn miss_shows_diff_against_closest_step() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = Store::new(Mode::Record, dir.path(), Matcher::default());
        recorder
            .record("s", Interaction { request: request(r#"{"prompt":"hello"}"#), response: response(200) })
            .unwrap();

        let replayer = Store::new(Mode::Replay, dir.path(), Matcher::default());
        let Lookup::Miss(miss) = replayer.lookup("s", &request(r#"{"prompt":"goodbye"}"#)).unwrap() else { panic!() };
        assert_eq!(miss.closest_step, Some(1));
        let diff = miss.diff.unwrap();
        assert!(diff.contains("-  \"prompt\": \"hello\""), "{diff}");
        assert!(diff.contains("+  \"prompt\": \"goodbye\""), "{diff}");
        assert_eq!(replayer.finish().unused.get("s"), Some(&1));
    }
}
