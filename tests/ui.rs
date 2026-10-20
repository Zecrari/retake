//! End-to-end tests for the dashboard API, both inside the proxy and standalone.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::routing::post;
use futures_util::StreamExt;
use serde_json::{Value, json};

use retake::cassette::Cassette;
use retake::config::{Config, Mode};
use retake::proxy::{self, AppState};
use retake::ui::{self, UiState};

async fn fake_upstream() -> SocketAddr {
    let app = Router::new().route("/v1/chat/completions", post(|| async { axum::Json(json!({ "ok": true })) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn proxy_serves_dashboard_and_live_feed() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(Mode::Record, dir.path());
    config.upstreams.insert("openai".into(), format!("http://{}", fake_upstream().await));
    let state = Arc::new(AppState::new(&config).unwrap());
    let server = proxy::start(state.clone(), "127.0.0.1:0".parse().unwrap()).await.unwrap();
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();

    let page = client.get(format!("{base}/__retake/")).send().await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert!(page.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap().contains("script-src 'self'"));
    assert!(page.text().await.unwrap().contains("retake dashboard"));

    let info: Value = client.get(format!("{base}/__retake/api/info")).send().await.unwrap().json().await.unwrap();
    assert_eq!(info["live"], true);
    assert_eq!(info["mode"], "record");
    assert_eq!(info["writable"], false);

    // Subscribe, then make a request through the proxy and expect it on the feed.
    let feed = client.get(format!("{base}/__retake/api/events")).send().await.unwrap();
    assert_eq!(feed.headers()[header::CONTENT_TYPE], "text/event-stream");
    let mut feed = feed.bytes_stream();
    client.post(format!("{base}/openai/v1/chat/completions")).json(&json!({ "model": "m" })).send().await.unwrap();
    let mut seen = String::new();
    while !seen.contains("\"kind\":\"recorded\"") {
        let chunk = tokio::time::timeout(Duration::from_secs(5), feed.next()).await.unwrap().unwrap().unwrap();
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(seen.contains("\"index\":0"), "{seen}");
    assert!(seen.contains("\"path\":\"/openai/v1/chat/completions\""), "{seen}");

    let sessions: Value =
        client.get(format!("{base}/__retake/api/sessions")).send().await.unwrap().json().await.unwrap();
    assert_eq!(sessions[0]["name"], "default");
    assert_eq!(sessions[0]["interactions"], 1);

    // The proxy owns the cassettes while it runs.
    let delete = client
        .delete(format!("{base}/__retake/api/session?name=default"))
        .header("x-retake-ui", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::CONFLICT);

    // Shutdown must not wait for the open dashboard feed.
    tokio::time::timeout(Duration::from_secs(5), server.stop()).await.expect("stop hung").unwrap();
}

#[tokio::test]
async fn standalone_dashboard_edits_and_is_protected() {
    let dir = tempfile::tempdir().unwrap();
    let mut cassette = Cassette::new("suite/one");
    let interaction: retake::cassette::Interaction = serde_json::from_value(json!({
        "request": { "method": "POST", "upstream": "openai", "path": "/v1/chat/completions", "json": { "n": 1 } },
        "response": { "status": 200, "json": { "ok": true } }
    }))
    .unwrap();
    cassette.interactions = vec![interaction.clone(), interaction];
    cassette.save(&dir.path().join("suite/one.json")).unwrap();

    let state = Arc::new(UiState::new(dir.path(), None, None, true));
    let base = serve(ui::router::<()>(state)).await;
    let client = reqwest::Client::new();

    let info: Value = client.get(format!("{base}/__retake/api/info")).send().await.unwrap().json().await.unwrap();
    assert_eq!((info["live"].clone(), info["writable"].clone()), (json!(false), json!(true)));
    let events = client.get(format!("{base}/__retake/api/events")).send().await.unwrap();
    assert_eq!(events.status(), StatusCode::NOT_FOUND);

    let session: Value =
        client.get(format!("{base}/__retake/api/session?name=suite/one")).send().await.unwrap().json().await.unwrap();
    assert_eq!(session["interactions"].as_array().unwrap().len(), 2);

    // DNS rebinding: a request whose Host isn't localhost is refused.
    let rebound =
        client.get(format!("{base}/__retake/api/sessions")).header(header::HOST, "evil.example").send().await.unwrap();
    assert_eq!(rebound.status(), StatusCode::FORBIDDEN);

    // Changes need the dashboard's header, which cross-site requests can't send.
    let url = format!("{base}/__retake/api/interaction?session=suite/one&index=0");
    assert_eq!(client.delete(&url).send().await.unwrap().status(), StatusCode::FORBIDDEN);
    let updated: Value = client.delete(&url).header("x-retake-ui", "1").send().await.unwrap().json().await.unwrap();
    assert_eq!(updated["interactions"].as_array().unwrap().len(), 1);
    assert_eq!(Cassette::load(&dir.path().join("suite/one.json")).unwrap().unwrap().interactions.len(), 1);

    let out_of_range = client
        .delete(format!("{base}/__retake/api/interaction?session=suite/one&index=5"))
        .header("x-retake-ui", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(out_of_range.status(), StatusCode::BAD_REQUEST);

    let escape = client.get(format!("{base}/__retake/api/session?name=../secrets")).send().await.unwrap();
    assert_eq!(escape.status(), StatusCode::BAD_REQUEST);

    let removed = client
        .delete(format!("{base}/__retake/api/session?name=suite/one"))
        .header("x-retake-ui", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert!(!dir.path().join("suite/one.json").exists());
}
