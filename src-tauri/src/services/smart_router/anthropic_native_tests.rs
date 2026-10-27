//! Anthropic-protocol passthrough for `/v1/messages`.
//!
//! Two properties matter and are asserted here:
//!  * a candidate that has its own Anthropic endpoint receives the request
//!    verbatim — including the `anthropic-beta` grants (1M context, prompt
//!    caching) the client negotiated, which the OpenAI conversion drops;
//!  * candidates are still tried in the configured order, and the OpenAI
//!    conversion remains the fallback for everything else.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State as AxumState;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};

use super::*;

#[derive(Default)]
struct Captured {
    beta: Option<String>,
    api_key: Option<String>,
    authorization: Option<String>,
    version: Option<String>,
    user_agent: Option<String>,
    body: Value,
}

async fn capture_handler(
    AxumState(captured): AxumState<Arc<Mutex<Captured>>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let header_value = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    {
        let mut captured = captured.lock().unwrap();
        captured.beta = header_value("anthropic-beta");
        captured.api_key = header_value("x-api-key");
        captured.authorization = header_value("authorization");
        captured.version = header_value("anthropic-version");
        captured.user_agent = header_value("user-agent");
        captured.body = body;
    }
    json_response(json!({
        "id": "msg_native",
        "type": "message",
        "role": "assistant",
        "model": "upstream-model",
        "content": [{"type": "text", "text": "native answer"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 3, "output_tokens": 2}
    }))
}

async fn capture_server(captured: Arc<Mutex<Captured>>) -> SocketAddr {
    let app = Router::new()
        .route("/v1/messages", post(capture_handler))
        .with_state(captured);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

fn json_response(value: Value) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

fn stub_candidate(id: &str) -> Candidate {
    let model_id = format!("{id}-model");
    let base_url = format!("http://{id}.invalid/v1");
    Candidate {
        attempt: None,
        anthropic_base: None,
        fingerprint: candidate_fingerprint(&model_id, &base_url, "key-native"),
        internal_id: id.to_string(),
        model_id,
        base_url,
        api_key: "key-native".into(),
        account_source: None,
    }
}

fn state_with_fixtures(entries: Vec<(&str, Response)>) -> AppState {
    let mut state = AppState::for_tests().unwrap();
    state.fixture_responses = Some(Arc::new(Mutex::new(
        entries
            .into_iter()
            .map(|(id, response)| (id.to_string(), response))
            .collect::<HashMap<String, Response>>(),
    )));
    state
}

fn messages_body() -> Value {
    json!({
        "model": "auto",
        "max_tokens": 16,
        "system": [{"type": "text", "text": "system prompt", "cache_control": {"type": "ephemeral"}}],
        "messages": [{"role": "user", "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}]}],
        "stream": false
    })
}

#[test]
fn anthropic_messages_url_accepts_every_stored_shape() {
    assert_eq!(
        anthropic_messages_url("https://api.anthropic.com"),
        "https://api.anthropic.com/v1/messages"
    );
    assert_eq!(
        anthropic_messages_url("https://relay.example/anthropic/"),
        "https://relay.example/anthropic/v1/messages"
    );
    assert_eq!(
        anthropic_messages_url("https://relay.example/v1"),
        "https://relay.example/v1/messages"
    );
    // Already an endpoint: left untouched.
    assert_eq!(
        anthropic_messages_url("https://relay.example/v1/messages"),
        "https://relay.example/v1/messages"
    );
}

#[test]
fn anthropic_messages_base_reads_the_model_config() {
    let with_url: crate::models::model::ModelConfig = serde_json::from_value(json!({
        "internalId": "m1",
        "name": "Model 1",
        "baseUrl": "https://relay.example/v1",
        "anthropicUrl": "https://relay.example/v1/messages",
        "apiKey": "key"
    }))
    .unwrap();
    assert_eq!(
        anthropic_messages_base(&with_url).as_deref(),
        Some("https://relay.example/v1/messages")
    );

    // A base URL that already points at the Messages endpoint is enough.
    let messages_only: crate::models::model::ModelConfig = serde_json::from_value(json!({
        "internalId": "m2",
        "name": "Model 2",
        "baseUrl": "https://relay.example/v1/messages",
        "apiKey": "key"
    }))
    .unwrap();
    assert_eq!(
        anthropic_messages_base(&messages_only).as_deref(),
        Some("https://relay.example/v1/messages")
    );

    // Pure OpenAI configuration keeps taking the converted path.
    let openai_only: crate::models::model::ModelConfig = serde_json::from_value(json!({
        "internalId": "m3",
        "name": "Model 3",
        "baseUrl": "https://relay.example/v1",
        "apiKey": "key"
    }))
    .unwrap();
    assert_eq!(anthropic_messages_base(&openai_only), None);
}

#[tokio::test]
async fn native_candidate_receives_beta_grants_and_an_unmodified_body() {
    let captured = Arc::new(Mutex::new(Captured::default()));
    let addr = capture_server(captured.clone()).await;
    let state = AppState::for_tests().unwrap();
    let base = format!("http://{addr}");

    let mut headers = HeaderMap::new();
    headers.insert(
        "anthropic-beta",
        HeaderValue::from_static("context-1m-2025-08-07,prompt-caching-2024-07-31"),
    );
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    headers.insert(
        header::USER_AGENT,
        HeaderValue::from_static("claude-cli/2.1"),
    );

    let body = messages_body();
    let response =
        send_anthropic_messages(&state, &headers, &stub_candidate("native"), &base, &body)
            .await
            .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(payload["content"][0]["text"], "native answer");

    let captured = captured.lock().unwrap();
    assert_eq!(
        captured.beta.as_deref(),
        Some("context-1m-2025-08-07,prompt-caching-2024-07-31"),
        "1M context and prompt-caching grants must reach the upstream"
    );
    assert_eq!(captured.version.as_deref(), Some("2023-06-01"));
    assert_eq!(captured.user_agent.as_deref(), Some("claude-cli/2.1"));
    // Third-party gateways get both auth schemes.
    assert_eq!(captured.api_key.as_deref(), Some("key-native"));
    assert_eq!(captured.authorization.as_deref(), Some("Bearer key-native"));
    // The body is byte-for-byte what the client sent: the model id is not
    // rewritten and `cache_control` markers survive.
    assert_eq!(captured.body, body);
}

#[tokio::test]
async fn native_candidate_answers_without_openai_conversion() {
    let native_body = json!({
        "id": "msg_native",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "native answer"}],
        "stop_reason": "end_turn"
    });
    let state = state_with_fixtures(vec![
        ("native", json_response(native_body.clone())),
        (
            "openai",
            json_response(json!({
                "choices": [{"message": {"role": "assistant", "content": "converted answer"}, "finish_reason": "stop"}]
            })),
        ),
    ]);
    let mut native = stub_candidate("native");
    native.anthropic_base = Some("https://anthropic.example/v1".into());

    let response = route_messages_with_candidates(
        &state,
        &HeaderMap::new(),
        messages_body(),
        vec![native, stub_candidate("openai")],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["X-EchoBird-Routed-Via"],
        "native/native-model"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        payload, native_body,
        "a native Anthropic answer must not be re-shaped by the OpenAI converter"
    );
}

#[tokio::test]
async fn failed_native_candidate_falls_through_to_the_next_one() {
    let state = state_with_fixtures(vec![
        (
            "native",
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"type":"error","error":{"type":"api_error","message":"boom"}})
                        .to_string(),
                ))
                .unwrap(),
        ),
        (
            "openai",
            json_response(json!({
                "choices": [{"message": {"role": "assistant", "content": "converted answer"}, "finish_reason": "stop"}]
            })),
        ),
    ]);
    let mut native = stub_candidate("native");
    native.anthropic_base = Some("https://anthropic.example/v1".into());

    let response = route_messages_with_candidates(
        &state,
        &HeaderMap::new(),
        messages_body(),
        vec![native, stub_candidate("openai")],
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["X-EchoBird-Routed-Via"],
        "openai/openai-model"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        payload["type"], "message",
        "the fallback must answer in Anthropic shape"
    );
    assert_eq!(payload["content"][0]["text"], "converted answer");
}

#[tokio::test]
async fn native_stream_is_forwarded_frame_for_frame() {
    // Exactly what an Anthropic upstream emits. The OpenAI adapter would
    // re-shape this (its own message_start, rewritten model, re-chunked
    // deltas); a byte-identical result proves the stream was not converted.
    let sse = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"upstream-model\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    let state = state_with_fixtures(vec![(
        "native",
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from(sse))
            .unwrap(),
    )]);
    let mut native = stub_candidate("native");
    native.anthropic_base = Some("https://anthropic.example/v1".into());

    let mut body = messages_body();
    body["stream"] = json!(true);
    let response =
        route_messages_with_candidates(&state, &HeaderMap::new(), body, vec![native]).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        sse,
        "the Anthropic event stream must reach the client unconverted"
    );
}

#[tokio::test]
async fn no_candidates_answers_in_anthropic_shape() {
    let state = AppState::for_tests().unwrap();
    let response =
        route_messages_with_candidates(&state, &HeaderMap::new(), messages_body(), Vec::new())
            .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let payload: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(payload["type"], "error");
}

#[test]
fn published_budget_is_the_smallest_window_among_candidates() {
    let mut large = stub_candidate("large");
    large.model_id = "glm-5.2".into();
    large.base_url = "https://maas.qianwenaiapi.com/v1".into();
    let mut small = stub_candidate("small");
    small.model_id = "MiniMax-M2.7".into();

    let (context, output) = published_budgets(vec![large.clone(), small.clone()]);
    assert_eq!(context, 204_800);
    assert!(output <= context / 2);

    // A single candidate reports its own window.
    assert_eq!(published_budgets(vec![large]).0, 1_000_000);
    // No candidates falls back to the documented default.
    assert_eq!(
        published_budgets(Vec::new()).0,
        crate::services::model_context::DEFAULT_CONTEXT_WINDOW
    );
}
