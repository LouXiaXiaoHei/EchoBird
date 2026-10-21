use super::*;
use axum::{routing::post, Json, Router};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tokio_stream::wrappers::ReceiverStream;

async fn fixture() -> (
    Value,
    Arc<Mutex<Vec<Value>>>,
    Arc<Notify>,
    tokio::task::JoinHandle<()>,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let cancelled = Arc::new(Notify::new());
    let notify = cancelled.clone();
    let app=Router::new().route("/chat/completions",post(move |headers:HeaderMap,Json(body):Json<Value>| {
        let seen=seen.clone();let notify=notify.clone();async move {
            assert_eq!(headers["authorization"],"Bearer fixture-token");
            assert_eq!(headers["x-dumate-client-type"],"web");assert_eq!(headers["x-dumate-account-id"],"global");
            assert_eq!(body["model"],"model-text");assert_eq!(body["stream"],true);
            assert!(body.get("_echobird_responses").is_none());
            seen.lock().await.push(body.clone());
            match body["messages"][0]["content"].as_str() {
                Some("quota")=>return Response::builder().status(429).header("retry-after","17")
                    .header("content-type","application/json").body(Body::from(json!({"error":{"code":"usage_limit_reached","message":"fixture-token must not leak"}}).to_string())).unwrap(),
                Some("expired")=>return Response::builder().status(401).header("content-type","application/json")
                    .body(Body::from(json!({"error":{"code":"invalid_api_key"}}).to_string())).unwrap(),
                Some("business-error")=>return Response::builder().header("content-type","application/json")
                    .body(Body::from(json!({"error_code":"insufficient_quota","error_message":"fixture-token must not leak"}).to_string())).unwrap(),
                Some("cancel")=> {
                    let (tx,rx)=tokio::sync::mpsc::channel::<Result<bytes::Bytes,std::io::Error>>(1);
                    tx.send(Ok(bytes::Bytes::from_static(b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"started\"}}]}\n\n"))).await.unwrap();
                    tokio::spawn(async move {tx.closed().await;notify.notify_one();});
                    return super::super::stream_response(Body::from_stream(ReceiverStream::new(rx)));
                }
                _=>{}
            }
            let is_result=body["messages"].as_array().unwrap().iter().any(|m| m["role"]=="tool");
            let events=if is_result {
                assert_eq!(body["messages"][2]["tool_call_id"],"call_dumate");
                assert_eq!(body["messages"][2]["content"],"EB42");
                vec![json!({"choices":[{"index":0,"delta":{"content":"EB42"},"finish_reason":"stop"}]})]
            } else {
                assert_eq!(body["tools"][0]["function"]["name"],"echo_probe");
                assert_eq!(body["tool_choice"]["function"]["name"],"echo_probe");
                vec![json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_dumate","type":"function","function":{"name":"echo_probe","arguments":"{\"value\":"}}]}}]}),
                    json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"EB42\"}"}}]},"finish_reason":"tool_calls"}]})]
            };
            let mut sse=events.into_iter().map(|e|format!("data: {e}\n\n")).collect::<String>();sse.push_str("data: [DONE]\n\n");
            super::super::stream_response(Body::from(sse))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let credentials = json!({"baseUrl":format!("http://{}",listener.local_addr().unwrap()),"accessToken":"fixture-token"});
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (credentials, requests, cancelled, server)
}
fn tool_request(stream: bool) -> Value {
    json!({"stream":stream,"messages":[{"role":"user","content":"use tool"}],
        "tools":[{"type":"function","function":{"name":"echo_probe","parameters":{"type":"object","properties":{"value":{"type":"string"}}}}}],
        "tool_choice":{"type":"function","function":{"name":"echo_probe"}},"_echobird_responses":{"input":"internal"}})
}
#[tokio::test]
async fn dumate_preserves_structured_tools_and_continuation_in_streaming_and_collected_chat() {
    let (c, requests, _, server) = fixture().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        list_models(AccountProvider::Dumate, c.clone(), &client)
            .await
            .unwrap(),
        ["model-text"]
    );
    assert!(requests.lock().await.is_empty());
    for stream in [false, true] {
        let response = super::super::route(
            AccountProvider::Dumate,
            c.clone(),
            "model-text",
            tool_request(stream),
            &client,
        )
        .await
        .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let message = if stream {
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(text.contains("tool_calls"));
            assert!(text.contains("call_dumate"));
            assert!(text.contains("[DONE]"));
            json!({"role":"assistant","content":null,"tool_calls":[{"id":"call_dumate","type":"function","function":{"name":"echo_probe","arguments":"{\"value\":\"EB42\"}"}}]})
        } else {
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
            assert_eq!(
                json["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
                "{\"value\":\"EB42\"}"
            );
            json["choices"][0]["message"].clone()
        };
        let follow = json!({"stream":false,"messages":[{"role":"user","content":"use tool"},message,{"role":"tool","tool_call_id":"call_dumate","content":"EB42"}]});
        let response = super::super::route(
            AccountProvider::Dumate,
            c.clone(),
            "model-text",
            follow,
            &client,
        )
        .await
        .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["choices"][0]["message"]["content"], "EB42");
    }
    server.abort();
}
#[tokio::test]
async fn dumate_quota_auth_and_business_errors_keep_recovery_status_without_exposing_tokens() {
    let (c, _, _, server) = fixture().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (prompt, status) in [
        ("quota", StatusCode::TOO_MANY_REQUESTS),
        ("expired", StatusCode::UNAUTHORIZED),
        ("business-error", StatusCode::PAYMENT_REQUIRED),
    ] {
        let e = super::super::route(
            AccountProvider::Dumate,
            c.clone(),
            "model-text",
            json!({"messages":[{"role":"user","content":prompt}]}),
            &client,
        )
        .await
        .unwrap_err();
        assert_eq!(e.status, status);
        assert!(!e.message.contains("fixture-token"));
        if prompt == "quota" {
            assert_eq!(e.retry_after.unwrap(), "17");
        }
    }
    server.abort();
}
#[tokio::test]
async fn dumate_cancelling_a_downstream_stream_releases_the_upstream_connection() {
    let (c, _, cancelled, server) = fixture().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = super::super::route(
        AccountProvider::Dumate,
        c,
        "model-text",
        json!({"stream":true,"messages":[{"role":"user","content":"cancel"}]}),
        &client,
    )
    .await
    .unwrap();
    drop(response);
    tokio::time::timeout(std::time::Duration::from_secs(3), cancelled.notified())
        .await
        .unwrap();
    server.abort();
}
