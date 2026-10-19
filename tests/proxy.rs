//! End-to-end tests: a fake LLM API, the real proxy, and a real HTTP client.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bytes::Bytes;
use serde_json::{Value, json};

use retake::config::{Config, Mode};
use retake::proxy::{self, AppState, SESSION_HEADER};
use retake::store::Stats;

const SSE_CHUNKS: [&str; 4] =
    ["data: {\"delta\":\"Hel\"}\n", "\ndata: {\"delta\":\"lo\"}\n\nda", "ta: [DONE]\n\n", ": trailing comment"];

/// A fake LLM API that counts how often it's called.
async fn fake_upstream() -> (SocketAddr, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        .route("/v1/broken", post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }))
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, hits)
}

async fn chat(State(hits): State<Arc<AtomicUsize>>, body: Bytes) -> Response {
    let n = hits.fetch_add(1, Ordering::SeqCst) + 1;
    let request: Value = serde_json::from_slice(&body).unwrap();
    if request["stream"] == json!(true) {
        let chunks = futures_util::stream::iter(SSE_CHUNKS.map(|c| Ok::<_, std::io::Error>(Bytes::from(c))));
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from_stream(chunks))
            .unwrap();
    }
    axum::Json(json!({ "call": n, "echo": request["messages"], "leak": "sk-proj-abcdefghijklmnopqrstuvwxyz" }))
        .into_response()
}

struct Proxy {
    url: String,
    server: proxy::Server,
    state: Arc<AppState>,
}

async fn start_proxy(mode: Mode, dir: &Path, upstream: SocketAddr, configure: impl FnOnce(&mut Config)) -> Proxy {
    let mut config = Config::new(mode, dir);
    config.upstreams.insert("openai".into(), format!("http://{upstream}"));
    configure(&mut config);
    let state = Arc::new(AppState::new(&config).unwrap());
    let server = proxy::start(state.clone(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
    Proxy { url: format!("http://{}/openai/v1", server.addr), server, state }
}

impl Proxy {
    async fn stop(self) -> Stats {
        self.server.stop().await.unwrap();
        self.state.store.finish()
    }
}

async fn post_json(url: &str, body: Value, session: Option<&str>) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(url)
        .header(header::AUTHORIZATION, "Bearer sk-test-abcdefghijklmnopqrstuvwxyz")
        .json(&body);
    if let Some(session) = session {
        request = request.header(SESSION_HEADER, session);
    }
    request.send().await.unwrap()
}

fn chat_body(text: &str) -> Value {
    json!({ "model": "test-model", "messages": [{ "role": "user", "content": text }] })
}

#[tokio::test]
async fn records_then_replays_json_offline() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, hits) = fake_upstream().await;

    let proxy = start_proxy(Mode::Record, dir.path(), upstream, |_| {}).await;
    let recorded: Value =
        post_json(&format!("{}/chat/completions", proxy.url), chat_body("hi"), None).await.json().await.unwrap();
    assert_eq!(recorded["call"], 1);
    assert_eq!(proxy.stop().await.recorded, 1);
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    let cassette = std::fs::read_to_string(dir.path().join("default.json")).unwrap();
    assert!(!cassette.contains("sk-proj-"), "response secret leaked into cassette");
    assert!(!cassette.contains("sk-test-"), "request header leaked into cassette");
    assert!(cassette.contains("[REDACTED]"));

    let proxy = start_proxy(Mode::Replay, dir.path(), upstream, |_| {}).await;
    // Same request with different key order and whitespace still matches.
    let raw = r#"{ "messages": [{"content":"hi","role":"user"}], "model": "test-model" }"#;
    let response = reqwest::Client::new()
        .post(format!("{}/chat/completions", proxy.url))
        .header(header::CONTENT_TYPE, "application/json")
        .body(raw)
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["x-retake"], "replay");
    let replayed: Value = response.json().await.unwrap();
    assert_eq!(replayed["call"], 1);
    assert_eq!(replayed["echo"], recorded["echo"]);
    let stats = proxy.stop().await;
    assert_eq!((stats.replayed, stats.misses.len()), (1, 0));
    assert_eq!(hits.load(Ordering::SeqCst), 1, "replay must not reach the upstream");
}

#[tokio::test]
async fn replays_streams_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, hits) = fake_upstream().await;
    let mut body = chat_body("stream please");
    body["stream"] = json!(true);

    let proxy = start_proxy(Mode::Record, dir.path(), upstream, |_| {}).await;
    let live = post_json(&format!("{}/chat/completions", proxy.url), body.clone(), None).await;
    assert_eq!(live.headers()[header::CONTENT_TYPE], "text/event-stream");
    let live = live.bytes().await.unwrap();
    assert_eq!(live, SSE_CHUNKS.concat().as_bytes());
    proxy.stop().await;

    let cassette: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("default.json")).unwrap()).unwrap();
    let events = cassette["interactions"][0]["response"]["events"].as_array().unwrap();
    assert_eq!(events.len(), 4, "stored as whole events, not network chunks");
    assert_eq!(events[1]["data"], "data: {\"delta\":\"lo\"}\n\n");

    let proxy = start_proxy(Mode::Replay, dir.path(), upstream, |_| {}).await;
    let replayed = post_json(&format!("{}/chat/completions", proxy.url), body, None).await;
    assert_eq!(replayed.headers()[header::CONTENT_TYPE], "text/event-stream");
    assert_eq!(replayed.bytes().await.unwrap(), live);
    proxy.stop().await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unrecorded_request_fails_with_a_diff() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, hits) = fake_upstream().await;

    let proxy = start_proxy(Mode::Record, dir.path(), upstream, |_| {}).await;
    post_json(&format!("{}/chat/completions", proxy.url), chat_body("refund order 1"), None).await;
    proxy.stop().await;

    let proxy = start_proxy(Mode::Replay, dir.path(), upstream, |_| {}).await;
    let response = post_json(&format!("{}/chat/completions", proxy.url), chat_body("refund order 2"), None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND, "4xx so SDKs don't retry");
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["type"], "retake_no_match");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("closest: step 1"), "{message}");
    assert!(message.contains("-      \"content\": \"refund order 1\""), "{message}");
    assert!(message.contains("+      \"content\": \"refund order 2\""), "{message}");

    let stats = proxy.stop().await;
    assert_eq!(stats.misses.len(), 1);
    assert_eq!(stats.unused.get("default"), Some(&1));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ignored_fields_and_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, _) = fake_upstream().await;
    let ignore = |c: &mut Config| c.ignore_fields = vec!["metadata.request_id".into()];
    let with_id = |id: &str| {
        let mut body = chat_body("same");
        body["metadata"] = json!({ "request_id": id });
        body
    };

    let proxy = start_proxy(Mode::Record, dir.path(), upstream, ignore).await;
    let url = format!("{}/chat/completions", proxy.url);
    post_json(&url, with_id("a"), Some("suite/one")).await;
    post_json(&url, chat_body("other"), Some("suite/two")).await;
    proxy.stop().await;
    assert!(dir.path().join("suite/one.json").exists());
    assert!(dir.path().join("suite/two.json").exists());

    let proxy = start_proxy(Mode::Replay, dir.path(), upstream, ignore).await;
    let url = format!("{}/chat/completions", proxy.url);
    assert_eq!(post_json(&url, with_id("b"), Some("suite/one")).await.status(), StatusCode::OK);
    // Recorded in the other session only.
    assert_eq!(post_json(&url, chat_body("other"), Some("suite/one")).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(post_json(&url, chat_body("x"), Some("../escape")).await.status(), StatusCode::BAD_REQUEST);
    proxy.stop().await;
}

#[tokio::test]
async fn server_errors_are_forwarded_but_not_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, _) = fake_upstream().await;

    let proxy = start_proxy(Mode::Record, dir.path(), upstream, |_| {}).await;
    let response = post_json(&format!("{}/broken", proxy.url), json!({}), None).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.text().await.unwrap(), "boom");
    assert_eq!(proxy.stop().await.recorded, 0);
    assert!(!dir.path().join("default.json").exists());
}

#[tokio::test]
async fn auto_mode_records_only_whats_missing() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, hits) = fake_upstream().await;

    for expected_hits in [1, 1] {
        let proxy = start_proxy(Mode::Auto, dir.path(), upstream, |_| {}).await;
        let response = post_json(&format!("{}/chat/completions", proxy.url), chat_body("hi"), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        proxy.stop().await;
        assert_eq!(hits.load(Ordering::SeqCst), expected_hits);
    }

    let proxy = start_proxy(Mode::Auto, dir.path(), upstream, |_| {}).await;
    post_json(&format!("{}/chat/completions", proxy.url), chat_body("new"), None).await;
    let stats = proxy.stop().await;
    assert_eq!((stats.recorded, hits.load(Ordering::SeqCst)), (1, 2));
}

#[tokio::test]
async fn unknown_upstream_is_explained() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, _) = fake_upstream().await;
    let proxy = start_proxy(Mode::Replay, dir.path(), upstream, |_| {}).await;
    let url = proxy.url.replace("/openai/v1", "/nope/v1/x");
    let response = post_json(&url, json!({}), None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["type"], "retake_unknown_upstream");
    proxy.stop().await;
}
