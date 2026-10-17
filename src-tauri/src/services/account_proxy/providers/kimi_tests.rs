use super::*;
use axum::{
    extract::{Request, State},
    routing::any,
    Router,
};
use std::future::IntoFuture;

type Requests = Arc<Mutex<Vec<(String, Value)>>>;

async fn upstream(State(requests): State<Requests>, request: Request) -> Response {
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer fixture-token"
    );
    let path = request.uri().to_string();
    let body = axum::body::to_bytes(request.into_body(), 1_000_000)
        .await
        .unwrap();
    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
    requests.lock().unwrap().push((path.clone(), body));
    if path == "/models" {
        return super::super::json_response(&json!({"data":[
            {"id":"fixture-chat","protocol":"openai"},
            {"id":"fixture-tool","protocol":"anthropic"}
        ]}));
    }
    if path == "/messages?beta=true" {
        return stream_response(Body::from(concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_fixture\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_fixture\",\"name\":\"weather\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Oslo\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        )));
    }
    stream_response(Body::from("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"))
}

#[tokio::test]
async fn kimi_discovers_models_and_their_protocol_without_model_id_rules() {
    let requests: Requests = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .fallback(any(upstream))
                .with_state(requests.clone()),
        )
        .into_future(),
    );
    let secret = json!({"accessToken":"fixture-token","baseUrl":format!("http://{address}"),"accountId":uuid::Uuid::new_v4().to_string()});
    let client = reqwest::Client::new();
    assert!(requests.lock().unwrap().is_empty());
    let models = list_models(AccountProvider::Kimi, secret.clone(), &client)
        .await
        .unwrap();
    assert_eq!(models, ["fixture-chat", "fixture-tool"]);
    let body = json!({"messages":[{"role":"user","content":"Weather?"}],"tools":[{"type":"function","function":{"name":"weather","parameters":{"type":"object","properties":{"city":{"type":"string"}}}}}],"reasoning_effort":"high","stream":false});
    for model in models {
        let response = route(
            AccountProvider::Kimi,
            secret.clone(),
            &model,
            body.clone(),
            &client,
        )
        .await
        .unwrap();
        assert!(response.status().is_success());
        let result: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1_000_000)
                .await
                .unwrap(),
        )
        .unwrap();
        if model == "fixture-chat" {
            assert_eq!(result["choices"][0]["message"]["content"], "ok");
        } else {
            assert_eq!(
                result["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
                "weather"
            );
            let arguments: Value = serde_json::from_str(
                result["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(arguments, json!({"city":"Oslo"}));
        }
    }
    let response = route(
        AccountProvider::Kimi,
        secret.clone(),
        "fixture-tool",
        json!({"messages":[{"role":"user","content":"Weather?"}],"stream":false}),
        &client,
    )
    .await
    .unwrap();
    assert!(response.status().is_success());
    route(
        AccountProvider::Kimi,
        secret,
        "fixture-tool",
        json!({"messages":[{"role":"user","content":"Weather?"}],"reasoning_effort":"none","stream":false}),
        &client,
    ).await.unwrap();
    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.iter().filter(|(p, _)| p == "/models").count(), 1);
    let anthropic = &recorded
        .iter()
        .find(|(p, _)| p == "/messages?beta=true")
        .unwrap()
        .1;
    assert_eq!(anthropic["tools"][0]["name"], "weather");
    assert_eq!(anthropic["tools"][0]["input_schema"]["type"], "object");
    assert_eq!(anthropic["thinking"], json!({"type":"adaptive"}));
    assert_eq!(anthropic["output_config"], json!({"effort":"high"}));
    assert!(anthropic.get("reasoning_effort").is_none());
    let chat = &recorded
        .iter()
        .find(|(p, _)| p == "/chat/completions")
        .unwrap()
        .1;
    assert_eq!(chat["reasoning_effort"], "high");
    let default = &recorded[recorded.len() - 2].1;
    assert!(default.get("thinking").is_none());
    assert!(default.get("output_config").is_none());
    let off = &recorded.last().unwrap().1;
    assert_eq!(off["thinking"], json!({"type":"disabled"}));
    assert!(off.get("output_config").is_none());
    server.abort();
}
