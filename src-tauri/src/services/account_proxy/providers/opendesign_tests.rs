use super::*;
use axum::{
    extract::{Request, State},
    Router,
};

type Calls = Arc<tokio::sync::Mutex<Vec<(String, HeaderMap, Value)>>>;

async fn upstream(State(calls): State<Calls>, request: Request) -> Response {
    let path = request.uri().path().to_string();
    let headers = request.headers().clone();
    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    calls
        .lock()
        .await
        .push((path.clone(), headers.clone(), body.clone()));
    if path == "/models" {
        let id = if headers[header::AUTHORIZATION] == "Bearer runtime-one" {
            "brand-new-model"
        } else {
            "other-account-model"
        };
        return super::super::json_response(&json!({"data":[{"id":id}]}));
    }
    if body["model"] == "expired" {
        let mut result = super::super::json_response(
            &json!({"error":{"message":"Account expired","code":"invalid_api_key"}}),
        );
        *result.status_mut() = StatusCode::UNAUTHORIZED;
        return result;
    }
    if path == "/responses" {
        return stream_response(Body::from("event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_fixture\",\"object\":\"response\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\"}]}],\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"));
    }
    if path == "/messages" {
        return stream_response(Body::from("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_fixture\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"));
    }
    stream_response(Body::from("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n"))
}

async fn fixture() -> (String, Calls, tokio::task::JoinHandle<()>) {
    let calls = Calls::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(upstream).with_state(calls.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, calls, server)
}

#[tokio::test]
async fn model_catalog_uses_each_accounts_runtime_key_and_accepts_new_model_ids() {
    let (base, calls, server) = fixture().await;
    let client = reqwest::Client::new();
    for (key, expected) in [
        ("runtime-one", "brand-new-model"),
        ("runtime-two", "other-account-model"),
    ] {
        let models = super::super::list_models(
            AccountProvider::OpenDesign,
            json!({"accessToken":key,"baseUrl":base}),
            &client,
        )
        .await
        .unwrap();
        assert_eq!(models, [expected]);
    }
    assert_eq!(calls.lock().await.len(), 2);
    for (path, headers, _) in calls.lock().await.iter() {
        assert_eq!(path, "/models");
        assert!(!headers.contains_key("x-api-key"));
    }
    server.abort();
}

#[tokio::test]
async fn chat_requests_reuse_all_three_wire_converters_and_preserve_auth_errors() {
    let (base, calls, server) = fixture().await;
    let client = reqwest::Client::new();
    let credentials = json!({"accessToken":"runtime-one","baseUrl":base});
    for (model, path) in [
        ("new-chat-model", "/chat/completions"),
        ("deepseek-v4-pro", "/responses"),
        ("claude-haiku-5.5", "/responses"),
        ("glm-5.3-flash", "/messages"),
    ] {
        for streaming in [false, true] {
            let response = super::super::route(
                AccountProvider::OpenDesign,
                credentials.clone(),
                model,
                json!({"messages":[{"role":"user","content":"hello"}],"stream":streaming}),
                &client,
            )
            .await
            .unwrap();
            assert!(response.status().is_success());
            let payload = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let text = String::from_utf8(payload.to_vec()).unwrap();
            assert!(text.contains("ok"), "{model}: {text}");
            let recorded = calls.lock().await;
            let (recorded_path, headers, body) = recorded.last().unwrap();
            assert_eq!(recorded_path, path);
            assert_eq!(headers[header::AUTHORIZATION], "Bearer runtime-one");
            if path == "/messages" {
                assert_eq!(headers["x-api-key"], "runtime-one");
                assert_eq!(headers["anthropic-version"], "2023-06-01");
            } else {
                assert!(!headers.contains_key("x-api-key"));
            }
            assert_eq!(body["model"], model);
            assert!(!serde_json::to_string(body).unwrap().contains("runtime-one"));
        }
    }
    let error = super::super::route(
        AccountProvider::OpenDesign,
        credentials,
        "expired",
        json!({"messages":[]}),
        &client,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    assert_eq!(calls.lock().await.len(), 9);
    server.abort();
}

#[tokio::test]
async fn native_responses_are_forwarded_only_for_responses_models() {
    let (base, calls, server) = fixture().await;
    let client = reqwest::Client::new();
    let credentials = json!({"accessToken":"runtime-one","baseUrl":base});
    let original = json!({"input":"hello","stream":true,"metadata":{"preserved":true},"tools":[{"type":"function","name":"read","parameters":{"type":"object"}}]});
    let response = super::super::route_responses(
        AccountProvider::OpenDesign,
        credentials.clone(),
        "deepseek-v4-pro",
        original.clone(),
        &client,
    )
    .await
    .unwrap()
    .unwrap();
    let payload = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(String::from_utf8(payload.to_vec())
        .unwrap()
        .contains("response.completed"));
    assert_eq!(calls.lock().await[0].2["metadata"], original["metadata"]);
    assert_eq!(calls.lock().await[0].2["tools"], original["tools"]);
    for model in ["new-chat-model", "glm-5.3-flash"] {
        assert!(super::super::route_responses(
            AccountProvider::OpenDesign,
            credentials.clone(),
            model,
            original.clone(),
            &client
        )
        .await
        .unwrap()
        .is_none());
    }
    assert_eq!(calls.lock().await.len(), 1);
    server.abort();
}
