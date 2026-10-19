//! The HTTP proxy. Requests arrive as `/<upstream>/<path>` and are either
//! served from a cassette or forwarded to the upstream and recorded.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::{StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info, warn};

use crate::cassette::{Interaction, Payload, RecordedRequest, RecordedResponse};
use crate::config::{Config, Mode, Timing};
use crate::matcher::Matcher;
use crate::sse::{self, EventSplitter};
use crate::store::{self, Lookup, Store};

/// Picks the cassette for a request; defaults to `--session`.
pub const SESSION_HEADER: &str = "x-retake-session";
const MAX_BODY_BYTES: usize = 256 * 1024 * 1024;
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

pub struct AppState {
    pub store: Store,
    mode: Mode,
    upstreams: BTreeMap<String, String>,
    default_session: String,
    timing: Timing,
    record_timing: bool,
    record_errors: bool,
    client: reqwest::Client,
    tasks: TaskTracker,
}

impl AppState {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        store::validate_session(&config.default_session).map_err(anyhow::Error::msg)?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("retake/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .context("building HTTP client")?;
        Ok(AppState {
            store: Store::new(config.mode, &config.cassette_dir, Matcher::new(&config.ignore_fields)),
            mode: config.mode,
            upstreams: config.upstreams.clone(),
            default_session: config.default_session.clone(),
            timing: config.timing,
            record_timing: config.record_timing,
            record_errors: config.record_errors,
            client,
            tasks: TaskTracker::new(),
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new().route("/__retake/health", get(|| async { "ok" })).fallback(handle).with_state(state)
}

pub struct Server {
    pub addr: SocketAddr,
    state: Arc<AppState>,
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<io::Result<()>>,
}

pub async fn start(state: Arc<AppState>, bind: SocketAddr) -> anyhow::Result<Server> {
    let listener = TcpListener::bind(bind).await.with_context(|| format!("binding {bind}"))?;
    let addr = listener.local_addr()?;
    let (shutdown, signal) = oneshot::channel::<()>();
    let app = router(state.clone());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = signal.await;
            })
            .await
    });
    Ok(Server { addr, state, shutdown, handle })
}

impl Server {
    /// Stops accepting requests and waits for in-flight recordings to be saved.
    pub async fn stop(self) -> anyhow::Result<()> {
        let _ = self.shutdown.send(());
        let tasks = self.state.tasks.clone();
        tasks.close();
        let drain = async {
            let served = self.handle.await;
            tasks.wait().await;
            served
        };
        match tokio::time::timeout(SHUTDOWN_GRACE, drain).await {
            Ok(served) => served.context("proxy task panicked")?.context("proxy server failed")?,
            Err(_) => warn!("timed out waiting for open connections to close"),
        }
        Ok(())
    }
}

async fn handle(State(state): State<Arc<AppState>>, request: Request) -> Response {
    match proxy(state, request).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

async fn proxy(state: Arc<AppState>, request: Request) -> Result<Response, ProxyError> {
    let (parts, body) = request.into_parts();
    let (upstream, path) = split_upstream(parts.uri.path());
    let Some(base_url) = state.upstreams.get(upstream).cloned() else {
        let known: Vec<&str> = state.upstreams.keys().map(String::as_str).collect();
        return Err(ProxyError::new(
            StatusCode::NOT_FOUND,
            "unknown_upstream",
            format!(
                "`/{upstream}` is not a configured upstream. Known upstreams: {}. Add one with --upstream NAME=URL.",
                known.join(", ")
            ),
        ));
    };
    let session = match parts.headers.get(SESSION_HEADER) {
        Some(value) => {
            value.to_str().map_err(|_| ProxyError::bad_request(format!("{SESSION_HEADER} must be ASCII")))?.to_string()
        }
        None => state.default_session.clone(),
    };
    store::validate_session(&session).map_err(ProxyError::bad_request)?;

    let bytes = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|e| ProxyError::bad_request(format!("reading request body: {e}")))?;
    let mut body_payload = Payload::from_bytes(&bytes, header_str(&parts.headers, header::CONTENT_TYPE));
    body_payload.redact();
    let recorded = RecordedRequest {
        method: parts.method.to_string(),
        upstream: upstream.to_string(),
        path: path.to_string(),
        query: parts.uri.query().map(str::to_string),
        body: body_payload,
    };

    if state.mode != Mode::Record {
        let lookup = state.store.lookup(&session, &recorded).map_err(ProxyError::cassette)?;
        match lookup {
            Lookup::Hit(response) => {
                debug!(session, "replayed {} /{}{}", recorded.method, upstream, path);
                return replay(*response, state.timing);
            }
            Lookup::Miss(miss) if state.mode == Mode::Replay => {
                state.store.note_miss(&miss);
                let message = miss.message();
                error!("{message}");
                return Err(ProxyError::new(StatusCode::NOT_FOUND, "no_match", message));
            }
            Lookup::Miss(_) => {}
        }
    }

    forward(state, parts, bytes, &base_url, session, recorded).await
}

async fn forward(
    state: Arc<AppState>,
    parts: Parts,
    bytes: Bytes,
    base_url: &str,
    session: String,
    recorded: RecordedRequest,
) -> Result<Response, ProxyError> {
    let mut url = format!("{base_url}{}", recorded.path);
    if let Some(query) = &recorded.query {
        url.push('?');
        url.push_str(query);
    }
    let mut headers = parts.headers;
    strip_request_headers(&mut headers);

    let started = Instant::now();
    let upstream = state
        .client
        .request(parts.method, &url)
        .headers(headers)
        .body(bytes)
        .send()
        .await
        .map_err(|e| ProxyError::new(StatusCode::BAD_GATEWAY, "upstream_unreachable", format!("{url}: {e}")))?;

    let status = upstream.status();
    let content_type = header_str(upstream.headers(), header::CONTENT_TYPE).map(str::to_string);
    let should_record = state.record_errors || !(status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error());
    if !should_record {
        warn!(session, "not recording {status} from {url} (use --record-errors to keep error responses)");
    }

    let mut response = Response::builder().status(status);
    for (name, value) in upstream.headers() {
        if !is_hop_by_hop(name) && name != header::CONTENT_LENGTH && name != header::CONTENT_ENCODING {
            response = response.header(name, value);
        }
    }

    let label = format!("{} /{}{}", recorded.method, recorded.upstream, recorded.path);
    let elapsed_ms = move |record: bool| record.then(|| started.elapsed().as_millis() as u64);

    if sse::is_event_stream(content_type.as_deref()) {
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(64);
        let mut stream = upstream.bytes_stream();
        let task_state = state.clone();
        state.tasks.spawn(async move {
            let record_timing = task_state.record_timing;
            let mut splitter = EventSplitter::default();
            let mut complete = true;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        splitter.push(&chunk, elapsed_ms(record_timing));
                        // Keep reading even if the client went away, so the recording is complete.
                        let _ = tx.send(Ok(chunk)).await;
                    }
                    Err(e) => {
                        complete = false;
                        warn!(session, "{label}: upstream stream failed, not recording: {e}");
                        let _ = tx.send(Err(io::Error::other(e))).await;
                        break;
                    }
                }
            }
            drop(tx);
            if !(complete && should_record) {
                return;
            }
            let (body, events) = match splitter.finish(elapsed_ms(record_timing)) {
                Ok(events) => (Payload::default(), events),
                Err(raw) => (Payload::from_bytes(&raw, None), Vec::new()),
            };
            let response = RecordedResponse {
                status: status.as_u16(),
                content_type,
                duration_ms: elapsed_ms(record_timing),
                body,
                events,
            };
            save(&task_state, &session, recorded, response, &label);
        });
        let body = Body::from_stream(ReceiverStream::new(rx));
        return response.body(body).map_err(ProxyError::internal);
    }

    let body = upstream
        .bytes()
        .await
        .map_err(|e| ProxyError::new(StatusCode::BAD_GATEWAY, "upstream_failed", format!("{url}: {e}")))?;
    if should_record {
        let mut payload = Payload::from_bytes(&body, content_type.as_deref());
        payload.redact();
        let recorded_response = RecordedResponse {
            status: status.as_u16(),
            content_type,
            duration_ms: elapsed_ms(state.record_timing),
            body: payload,
            events: Vec::new(),
        };
        save(&state, &session, recorded, recorded_response, &label);
    }
    response.body(Body::from(body)).map_err(ProxyError::internal)
}

fn save(state: &AppState, session: &str, request: RecordedRequest, response: RecordedResponse, label: &str) {
    let status = response.status;
    match state.store.record(session, Interaction { request, response }) {
        Ok(()) => info!(session, "recorded {label} -> {status}"),
        Err(e) => error!(session, "failed to save recording of {label}: {e:#}"),
    }
}

fn replay(recorded: RecordedResponse, timing: Timing) -> Result<Response, ProxyError> {
    let status = StatusCode::from_u16(recorded.status)
        .map_err(|_| ProxyError::cassette(anyhow::anyhow!("invalid status {} in cassette", recorded.status)))?;
    let mut response = Response::builder().status(status).header("x-retake", "replay");
    if let Some(content_type) = &recorded.content_type {
        response = response.header(header::CONTENT_TYPE, content_type);
    }
    if recorded.events.is_empty() {
        let body = recorded.body.to_bytes().map_err(ProxyError::cassette)?;
        return response.body(Body::from(body)).map_err(ProxyError::internal);
    }
    let realtime = timing == Timing::Realtime;
    let events = futures_util::stream::unfold(
        (recorded.events.into_iter(), None::<tokio::time::Instant>),
        move |(mut events, start)| async move {
            let event = events.next()?;
            let start = start.unwrap_or_else(tokio::time::Instant::now);
            if realtime && let Some(t_ms) = event.t_ms {
                tokio::time::sleep_until(start + Duration::from_millis(t_ms)).await;
            }
            Some((Ok::<Bytes, Infallible>(Bytes::from(event.data)), (events, Some(start))))
        },
    );
    response.body(Body::from_stream(events)).map_err(ProxyError::internal)
}

/// `/openai/v1/chat/completions` -> (`openai`, `/v1/chat/completions`).
fn split_upstream(path: &str) -> (&str, &str) {
    let trimmed = path.trim_start_matches('/');
    match trimmed.find('/') {
        Some(i) => (&trimmed[..i], &trimmed[i..]),
        None => (trimmed, "/"),
    }
}

fn header_str(headers: &HeaderMap, name: HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn strip_request_headers(headers: &mut HeaderMap) {
    let drop: Vec<HeaderName> = headers
        .keys()
        .filter(|name| {
            is_hop_by_hop(name)
                || *name == header::HOST
                || *name == header::CONTENT_LENGTH
                // The client decompresses for us; asking for a specific encoding would bypass that.
                || *name == header::ACCEPT_ENCODING
                || name.as_str() == SESSION_HEADER
        })
        .cloned()
        .collect();
    for name in drop {
        headers.remove(name);
    }
}

/// Errors use 4xx statuses where possible: SDKs retry 5xx responses, which
/// would turn a clear failure into a slow, confusing one.
#[derive(Debug)]
pub struct ProxyError {
    status: StatusCode,
    kind: &'static str,
    message: String,
}

impl ProxyError {
    fn new(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Self {
        ProxyError { status, kind, message: message.into() }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    fn cassette(e: anyhow::Error) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "cassette_error", format!("{e:#}"))
    }

    fn internal(e: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        // Shaped so both the OpenAI and Anthropic SDKs surface the message.
        let body = serde_json::json!({
            "type": "error",
            "error": { "type": format!("retake_{}", self.kind), "message": self.message },
        });
        let mut response = (self.status, axum::Json(body)).into_response();
        response.headers_mut().insert("x-retake-error", HeaderValue::from_static(self.kind));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_upstream_from_path() {
        assert_eq!(split_upstream("/openai/v1/chat/completions"), ("openai", "/v1/chat/completions"));
        assert_eq!(split_upstream("/anthropic"), ("anthropic", "/"));
        assert_eq!(split_upstream("/"), ("", "/"));
    }
}
