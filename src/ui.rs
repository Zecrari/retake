//! The web dashboard: browse cassettes, and watch the proxy live.
//!
//! The front-end lives in `ui/` and is compiled into the binary. It is served
//! under `/__retake/`, both by `retake ui` and by the proxy itself.

use std::convert::Infallible;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use axum::Router;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;

use crate::cassette::Cassette;
use crate::live::Live;
use crate::store::{self, session_path};

const INDEX: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const APP_CSS: &str = include_str!("../ui/app.css");
const FAVICON: &str = include_str!("../ui/favicon.svg");

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
                   connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// Header the dashboard sends with every change, so a cross-site form or
/// `fetch` can't modify cassettes (it would need a CORS preflight we never allow).
const CHANGE_HEADER: &str = "x-retake-ui";

pub struct UiState {
    dir: PathBuf,
    live: Option<Arc<Live>>,
    mode: Option<&'static str>,
    writable: bool,
}

impl UiState {
    pub fn new(dir: &Path, live: Option<Arc<Live>>, mode: Option<&'static str>, writable: bool) -> Self {
        UiState { dir: dir.to_path_buf(), live, mode, writable }
    }
}

pub fn router<S: Clone + Send + Sync + 'static>(state: Arc<UiState>) -> Router<S> {
    let api = Router::new()
        .route("/info", get(info))
        .route("/sessions", get(sessions))
        .route("/session", get(session).delete(delete_session))
        .route("/interaction", delete(delete_interaction))
        .route("/events", get(events))
        .layer(middleware::from_fn(guard));
    Router::new()
        .route("/__retake", get(|| async { Redirect::permanent("/__retake/") }))
        .route("/__retake/", get(|| async { asset(INDEX, "text/html; charset=utf-8") }))
        .route("/__retake/app.js", get(|| async { asset(APP_JS, "text/javascript; charset=utf-8") }))
        .route("/__retake/app.css", get(|| async { asset(APP_CSS, "text/css; charset=utf-8") }))
        .route("/__retake/favicon.svg", get(|| async { asset(FAVICON, "image/svg+xml") }))
        .nest("/__retake/api", api)
        .with_state(state)
}

/// `retake ui`: serves the dashboard on its own, with editing enabled.
pub async fn serve(dir: PathBuf, bind: SocketAddr, open: bool) -> anyhow::Result<()> {
    let state = Arc::new(UiState::new(&dir, None, None, true));
    let listener = tokio::net::TcpListener::bind(bind).await.with_context(|| format!("binding {bind}"))?;
    let url = format!("http://{}/__retake/", listener.local_addr()?);
    eprintln!("retake dashboard: {url}");
    eprintln!("Browsing cassettes in {}. Press Ctrl+C to stop.", dir.display());
    if open {
        open_browser(&url);
    }
    axum::serve(listener, router::<()>(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("dashboard server failed")
}

/// Best effort: if no browser opens, the URL is printed anyway.
pub fn open_browser(url: &str) {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = result {
        tracing::debug!("could not open a browser: {e}");
    }
}

fn asset(body: &'static str, content_type: &'static str) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// Only answers requests addressed to localhost (blocking DNS-rebinding
/// attacks), and requires the dashboard's header on anything that changes data.
async fn guard(request: Request, next: Next) -> Response {
    let local = request.headers().get(header::HOST).and_then(|h| h.to_str().ok()).is_some_and(is_local_host);
    if !local {
        return error(StatusCode::FORBIDDEN, "the dashboard API only answers requests to localhost");
    }
    if request.method() != Method::GET && !request.headers().contains_key(CHANGE_HEADER) {
        return error(StatusCode::FORBIDDEN, "missing x-retake-ui header");
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn is_local_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    matches!(name.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1")
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, axum::Json(serde_json::json!({ "error": message.into() }))).into_response()
}

#[derive(Serialize)]
struct Info {
    version: &'static str,
    /// The proxy's mode, or none when browsing with `retake ui`.
    mode: Option<&'static str>,
    live: bool,
    writable: bool,
    /// The cassette directory as given on the command line.
    cassettes: String,
    /// The same directory as an absolute path.
    cassettes_path: String,
}

async fn info(State(state): State<Arc<UiState>>) -> Response {
    let absolute = fs::canonicalize(&state.dir).unwrap_or_else(|_| state.dir.clone());
    let absolute = absolute.display().to_string();
    // Windows canonical paths start with \\?\, which is noise in the UI.
    let absolute = absolute.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(absolute);
    axum::Json(Info {
        version: env!("CARGO_PKG_VERSION"),
        mode: state.mode,
        live: state.live.is_some(),
        writable: state.writable,
        cassettes: state.dir.display().to_string(),
        cassettes_path: absolute,
    })
    .into_response()
}

#[derive(Serialize)]
struct SessionInfo {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    interactions: Option<usize>,
    bytes: u64,
    /// Unix time in milliseconds.
    modified: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn sessions(State(state): State<Arc<UiState>>) -> Response {
    let dir = state.dir.clone();
    match tokio::task::spawn_blocking(move || list_sessions(&dir)).await {
        Ok(Ok(list)) => axum::Json(list).into_response(),
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn list_sessions(dir: &Path) -> anyhow::Result<Vec<SessionInfo>> {
    #[derive(Deserialize)]
    struct Count {
        interactions: Vec<serde::de::IgnoredAny>,
    }

    let mut files = Vec::new();
    if dir.is_dir() {
        collect(dir, dir, &mut files)?;
    }
    let mut list: Vec<SessionInfo> = files
        .into_iter()
        .map(|(name, path)| {
            let meta = fs::metadata(&path).ok();
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            let parsed = fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| Ok(serde_json::from_slice::<Count>(&bytes)?.interactions.len()));
            let (interactions, error) = match parsed {
                Ok(count) => (Some(count), None),
                Err(e) => (None, Some(format!("not a valid cassette: {e}"))),
            };
            SessionInfo { name, interactions, bytes: meta.map_or(0, |m| m.len()), modified, error }
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let Ok(relative) = path.strip_prefix(root) else { continue };
            let name = relative.with_extension("").to_string_lossy().replace('\\', "/");
            if store::validate_session(&name).is_ok() {
                out.push((name, path));
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct SessionQuery {
    name: String,
}

async fn session(State(state): State<Arc<UiState>>, Query(query): Query<SessionQuery>) -> Response {
    if let Err(e) = store::validate_session(&query.name) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let path = session_path(&state.dir, &query.name);
    match tokio::task::spawn_blocking(move || Cassette::load(&path)).await {
        Ok(Ok(Some(cassette))) => axum::Json(cassette).into_response(),
        Ok(Ok(None)) => error(StatusCode::NOT_FOUND, format!("no cassette named `{}`", query.name)),
        Ok(Err(e)) => error(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn delete_session(State(state): State<Arc<UiState>>, Query(query): Query<SessionQuery>) -> Response {
    if !state.writable {
        return read_only();
    }
    if let Err(e) = store::validate_session(&query.name) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let path = session_path(&state.dir, &query.name);
    match tokio::task::spawn_blocking(move || fs::remove_file(path)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            error(StatusCode::NOT_FOUND, format!("no cassette named `{}`", query.name))
        }
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct InteractionQuery {
    session: String,
    index: usize,
}

async fn delete_interaction(State(state): State<Arc<UiState>>, Query(query): Query<InteractionQuery>) -> Response {
    if !state.writable {
        return read_only();
    }
    if let Err(e) = store::validate_session(&query.session) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    let path = session_path(&state.dir, &query.session);
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<Cassette>> {
        let Some(mut cassette) = Cassette::load(&path)? else { return Ok(None) };
        if query.index >= cassette.interactions.len() {
            anyhow::bail!("interaction {} does not exist", query.index);
        }
        cassette.interactions.remove(query.index);
        cassette.save(&path)?;
        Ok(Some(cassette))
    })
    .await;
    match result {
        Ok(Ok(Some(cassette))) => axum::Json(cassette).into_response(),
        Ok(Ok(None)) => error(StatusCode::NOT_FOUND, "no such cassette"),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn read_only() -> Response {
    error(StatusCode::CONFLICT, "cassettes are read-only while the proxy is running; use `retake ui` to edit them")
}

async fn events(State(state): State<Arc<UiState>>) -> Response {
    let Some(live) = state.live.clone() else {
        return error(StatusCode::NOT_FOUND, "no live feed: the dashboard isn't attached to a running proxy");
    };
    let (history, receiver) = live.subscribe();
    let new = BroadcastStream::new(receiver).filter_map(|event| async move { event.ok() });
    let feed = stream::iter(history)
        .chain(new)
        .map(|event| {
            let sse = Event::default().id(event.id.to_string()).json_data(&event);
            Ok::<_, Infallible>(sse.unwrap_or_default())
        })
        .take_until(live.closed());
    Sse::new(feed).keep_alive(KeepAlive::default()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_local_hosts() {
        for host in ["127.0.0.1:7357", "localhost:9000", "LOCALHOST", "[::1]:80", "127.0.0.1"] {
            assert!(is_local_host(host), "{host}");
        }
        for host in ["evil.example:7357", "127.0.0.1.evil.example", "[::2]:80", "192.168.1.5:80"] {
            assert!(!is_local_host(host), "{host}");
        }
    }

    #[test]
    fn lists_sessions_recursively() {
        let dir = tempfile::tempdir().unwrap();
        Cassette::new("a").save(&dir.path().join("a.json")).unwrap();
        Cassette::new("suite/b").save(&dir.path().join("suite/b.json")).unwrap();
        fs::write(dir.path().join("broken.json"), "{").unwrap();
        fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let list = list_sessions(dir.path()).unwrap();
        let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "broken", "suite/b"]);
        assert_eq!(list[0].interactions, Some(0));
        assert!(list[1].error.is_some());
        assert!(list_sessions(&dir.path().join("missing")).unwrap().is_empty());
    }
}
