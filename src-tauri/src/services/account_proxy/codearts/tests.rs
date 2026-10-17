use super::*;
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    routing::any,
    Router,
};
use std::{
    future::IntoFuture,
    sync::{Arc, Mutex},
};

fn credentials() -> Value {
    json!({"accessKey":"fixture-ak","secretKey":"fixture-sk","securityToken":"fixture-token"})
}

#[test]
fn signature_matches_the_shipped_desktop_signer() {
    // Expected signature was produced by the extracted desktop sign() at this timestamp.
    let signed = signed_headers_at(
        "POST",
        "https://example.test/api/v2/chat/completions?z=2&a=x%20y",
        &credentials(),
        "{\"model\":\"fixture-model\"}",
        BTreeMap::from([("Content-Type".into(), "application/json".into())]),
        "20261018T010203Z",
    )
    .unwrap();
    assert_eq!(signed["authorization"], "SDK-HMAC-SHA256 Access=fixture-ak, SignedHeaders=content-type;host;x-sdk-date;x-security-token, Signature=a6a0cccd91317f2a35d1a9ae38f7670501464cedc088ae6b413ef5657cc0b4d3");
    let mut changed = credentials();
    changed["secretKey"] = json!("different-key");
    let other = signed_headers_at(
        "POST",
        "https://example.test/api/v2/chat/completions?z=2&a=x%20y",
        &changed,
        "{\"model\":\"fixture-model\"}",
        BTreeMap::from([("Content-Type".into(), "application/json".into())]),
        "20261018T010203Z",
    )
    .unwrap();
    assert_ne!(signed["authorization"], other["authorization"]);
    changed["securityToken"] = Value::Null;
    assert!(signed_headers("GET", "https://example.test", &changed, "", BTreeMap::new()).is_err());
}

#[derive(Clone, Default)]
struct Fixture {
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    mode: Arc<Mutex<String>>,
}

async fn upstream(State(state): State<Fixture>, request: Request) -> Response {
    assert_eq!(request.headers()["x-security-token"], "fixture-token");
    let auth = request.headers()["authorization"].to_str().unwrap();
    assert!(auth.starts_with("SDK-HMAC-SHA256 Access=fixture-ak, "));
    assert!(!auth.contains("fixture-sk"));
    let path = request.uri().to_string();
    let bytes = to_bytes(request.into_body(), 1_000_000).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    state.requests.lock().unwrap().push((path.clone(), body));
    match path.as_str() {
        "/v1/agent-center/agents/useragents?offset=0&limit=100&is_primary_agent=true" => {
            if *state.mode.lock().unwrap() == "missing" {
                return super::super::json_response(&json!({"agents":[]}));
            }
            super::super::json_response(&json!({"agents":[
                {"agent_name":"CodeAgent","agent_id":"hidden","alias":{"alias_zh_cn":"智能体"},"show_in_ide":false},
                {"agent_name":"CodeAgent","agent_id":"fixture-id","alias":{"alias_zh_cn":"智能体"},"show_in_ide":true}
            ]}))
        }
        "/v1/agent-center/agents/detail?agent_id=fixture-id" => {
            super::super::json_response(&json!({"gpts":{"models":[
                {"model_name":"Fixture","model_type":"builtin","model_parameters":{"model_id":"fixture-chat"}},
                {"model_name":"Third party","model_type":"third","model_parameters":{"model_id":"not-account-backed"}},
                {"model_name":"GLM-5.1","model_type":"builtin","model_parameters":{"model_id":"hidden-vendor-model"}},
                {"model_name":"Malformed","model_type":"builtin","model_parameters":{}}
            ]}}))
        }
        "/api/v2/chat/completions" => {
            if *state.mode.lock().unwrap() == "busy" {
                return super::super::stream_response(Body::from(
                    "data: {\"error_code\":\"ModelArts.81101\",\"error_msg\":\"Busy\"}\n\n",
                ));
            }
            super::super::stream_response(Body::from(concat!(
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\",\"tool_calls\":[{\"index\":0,\"id\":\"call_fixture\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{\\\"marker\\\":\\\"ok\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3}}\n\n",
                "data: [DONE]\n\n"
            )))
        }
        _ => panic!("unexpected request: {path}"),
    }
}

#[tokio::test]
async fn discovery_and_inference_use_signed_official_paths_and_shared_chat_conversion() {
    let state = Fixture::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .fallback(any(upstream))
                .with_state(state.clone()),
        )
        .into_future(),
    );
    let mut secret = credentials();
    secret["baseUrl"] = json!(format!("http://{address}"));
    let client = reqwest::Client::new();
    assert!(state.requests.lock().unwrap().is_empty());
    assert_eq!(
        super::super::list_models(
            crate::models::model::AccountProvider::CodeArts,
            secret.clone(),
            &client
        )
        .await
        .unwrap(),
        ["fixture-chat"]
    );
    for streaming in [false, true] {
        let body = json!({"messages":[{"role":"user","content":"fixture"}],"stream":streaming,
            "tools":[{"type":"function","function":{"name":"echo","parameters":{"type":"object"}}}],"reasoning_effort":"high"});
        let response = super::super::route(
            crate::models::model::AccountProvider::CodeArts,
            secret.clone(),
            "fixture-chat",
            body,
            &client,
        )
        .await
        .unwrap();
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        if streaming {
            let frames = std::str::from_utf8(&bytes).unwrap();
            assert!(frames.contains("data: [DONE]"));
            assert!(frames.contains("call_fixture"));
        } else {
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(result["choices"][0]["message"]["content"], "ok");
            assert_eq!(
                result["choices"][0]["message"]["reasoning_content"],
                "thinking"
            );
            assert_eq!(
                result["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
                "{\"marker\":\"ok\"}"
            );
            assert_eq!(result["usage"]["completion_tokens"], 3);
        }
        let requests = state.requests.lock().unwrap();
        let sent = &requests.last().unwrap().1;
        assert_eq!(sent["model"], "fixture-chat");
        assert_eq!(sent["reasoning_effort"], "high");
        assert_eq!(sent["stream"], true);
        assert_eq!(sent["tool_stream"], true);
        assert_eq!(sent["tools"][0]["function"]["name"], "echo");
        assert!(!sent.to_string().contains("fixture-token"));
    }
    let continuation = json!({"messages":[
        {"role":"assistant","tool_calls":[{"id":"call_previous","type":"function","function":{"name":"echo","arguments":"{}"}}]},
        {"role":"tool","tool_call_id":"call_previous","content":"fixture-result"}
    ],"stream":false,"tool_choice":"none"});
    let response = route(secret.clone(), "fixture-chat", continuation, &client)
        .await
        .unwrap();
    to_bytes(response.into_body(), 1_000_000).await.unwrap();
    {
        let requests = state.requests.lock().unwrap();
        let sent = &requests.last().unwrap().1;
        assert_eq!(sent["messages"][1]["tool_call_id"], "call_previous");
        assert_eq!(sent["messages"][1]["content"], "fixture-result");
        assert_eq!(sent["tool_choice"], "none");
        assert!(sent.get("reasoning_effort").is_none());
    }
    *state.mode.lock().unwrap() = "busy".into();
    let result = route(
        secret.clone(),
        "fixture-chat",
        json!({"messages":[],"stream":false}),
        &client,
    )
    .await;
    // The shared converter exposes stream errors on its response body.
    let bytes = to_bytes(result.unwrap().into_body(), 1_000_000).await;
    assert!(bytes.is_err());
    let error = providers::stream_error(
        "",
        "{\"error_code\":\"ModelArts.81101\",\"error_msg\":\"Busy\"}",
        &Default::default(),
    )
    .unwrap();
    assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error.error_code.as_deref(), Some("ModelArts.81101"));
    *state.mode.lock().unwrap() = "missing".into();
    let before = state.requests.lock().unwrap().len();
    assert!(list_models(secret, &client).await.is_err());
    assert_eq!(state.requests.lock().unwrap().len(), before + 1);
    server.abort();
}

#[test]
fn malformed_or_empty_catalogs_do_not_invent_models() {
    assert!(catalog_ids(&json!({})).is_err());
    assert!(catalog_ids(&json!({"gpts":{"models":[]}})).is_err());
}
