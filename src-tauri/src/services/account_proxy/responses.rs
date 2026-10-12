use std::collections::BTreeMap;

use axum::{
    body::{to_bytes, Body},
    http::StatusCode,
    response::Response,
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::protocol::SseParser;
use super::tools::{self, RESPONSES_REQUEST};
use super::{json_response, stream_response, ProxyError};

/// Responses clients enter the same Chat-based router, including tool-result turns.
pub fn responses_to_chat(input: &Value) -> Result<Value, ProxyError> {
    if input
        .get("previous_response_id")
        .is_some_and(|value| !value.is_null())
    {
        return Err(ProxyError::new(
            StatusCode::BAD_REQUEST,
            "The router requires explicit conversation input instead of previous_response_id",
        ));
    }
    let catalog = tools::catalog(input)?;
    let mut messages = Vec::new();
    if let Some(instructions) = input.get("instructions").filter(|v| !v.is_null()) {
        messages.push(json!({"role":"system","content":instructions}));
    }
    if let Some(text) = input["input"].as_str() {
        messages.push(json!({"role":"user","content":text}));
    } else if let Some(items) = input["input"].as_array() {
        for item in items {
            match item["type"].as_str().unwrap_or("message") {
                "message" => {
                    let content = if item["content"].is_string() {
                        item["content"].clone()
                    } else {
                        let mut parts = Vec::new();
                        for part in item["content"].as_array().into_iter().flatten() {
                            parts.push(match part["type"].as_str().unwrap_or_default() {
                                "input_text"|"output_text"|"text"=>json!({"type":"text","text":part["text"]}),
                                "input_image"=>json!({"type":"image_url","image_url":{"url":part["image_url"],"detail":part.get("detail").cloned().unwrap_or(json!("auto"))}}),
                                "input_file"=>part.clone(),
                                _=>return Err(ProxyError::new(StatusCode::BAD_REQUEST,"Unsupported Responses content type")),
                            });
                        }
                        json!(parts)
                    };
                    messages.push(json!({"role":item["role"],"content":content}));
                }
                "function_call" | "custom_tool_call" => {
                    let name = tools::alias(
                        item["namespace"].as_str().unwrap_or_default(),
                        item["name"].as_str().unwrap_or_default(),
                    );
                    let arguments = if item["type"] == "custom_tool_call" {
                        json!({"input":item["input"]}).to_string()
                    } else {
                        item["arguments"].as_str().unwrap_or_default().to_string()
                    };
                    messages.push(json!({"role":"assistant","content":null,"tool_calls":[{"id":item["call_id"],"type":"function","function":{"name":name,"arguments":arguments}}]}));
                }
                "function_call_output" | "custom_tool_call_output" => messages.push(
                    json!({"role":"tool","tool_call_id":item["call_id"],"content":item["output"]}),
                ),
                "reasoning" | "additional_tools" => {}
                _ => {
                    return Err(ProxyError::new(
                        StatusCode::BAD_REQUEST,
                        "Unsupported Responses input item",
                    ))
                }
            }
        }
    } else {
        return Err(ProxyError::new(
            StatusCode::BAD_REQUEST,
            "Responses input must be text or an array",
        ));
    }
    let tools = tools::chat_tools(&catalog);
    let mut output = json!({"model":input["model"],"messages":messages,"tools":tools,"stream":input["stream"].as_bool().unwrap_or(false)});
    output[RESPONSES_REQUEST] = input.clone();
    for key in [
        "temperature",
        "top_p",
        "parallel_tool_calls",
        "reasoning",
        "text",
    ] {
        if let Some(value) = input.get(key) {
            output[key] = value.clone();
        }
    }
    if let Some(value) = input.get("max_output_tokens") {
        output["max_tokens"] = value.clone();
    }
    if let Some(choice) = input.get("tool_choice") {
        output["tool_choice"] = if choice.is_object() {
            json!({"type":"function","function":{"name":tools::alias(choice["namespace"].as_str().unwrap_or_default(),choice["name"].as_str().unwrap_or_default())}})
        } else {
            choice.clone()
        };
    }
    Ok(output)
}

#[derive(Default)]
struct Encoder {
    id: String,
    model: String,
    sequence: u64,
    output: Vec<Value>,
    text_index: Option<usize>,
    reasoning_index: Option<usize>,
    tool_indexes: BTreeMap<u64, usize>,
    tools: BTreeMap<String, Value>,
    custom_arguments: BTreeMap<usize, String>,
    pending_tools: BTreeMap<u64, Vec<Value>>,
    response_item_indexes: BTreeMap<String, usize>,
    usage: Value,
    finish_reason: String,
    finished: bool,
}

impl Encoder {
    fn new() -> Self {
        Self {
            id: format!("resp_{}", uuid::Uuid::new_v4().simple()),
            ..Self::default()
        }
    }

    fn with_tools(request: &Value) -> Self {
        Self {
            tools: tools::catalog(request).unwrap_or_default(),
            ..Self::new()
        }
    }

    fn event(&mut self, kind: &str, mut value: Value) -> Bytes {
        value["type"] = json!(kind);
        value["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        Bytes::from(format!("event: {kind}\ndata: {value}\n\n"))
    }

    fn response(&self, status: &str) -> Value {
        let mut output = self.output.clone();
        for item in &mut output {
            item["status"] = json!(if status == "in_progress" {
                "in_progress"
            } else {
                "completed"
            });
        }
        let mut response = json!({"id":self.id,"object":"response","created_at":chrono::Utc::now().timestamp(),"model":self.model,
            "status":status,"output":output,"error":null,"incomplete_details":null,"usage":self.usage});
        if status == "incomplete" {
            response["incomplete_details"] = json!({"reason":"max_output_tokens"});
        }
        response
    }

    fn created(&mut self) -> Bytes {
        let response = self.response("in_progress");
        self.event("response.created", json!({"response":response}))
    }

    fn response_item(&mut self, item: &Value, chunks: &mut Vec<Bytes>) {
        let id = item["id"].as_str().unwrap_or_default();
        if let Some(index) = self.response_item_indexes.get(id) {
            self.output[*index] = item.clone();
            return;
        }
        let index = self.output.len();
        self.response_item_indexes.insert(id.into(), index);
        self.reasoning_index = Some(index);
        self.output.push(item.clone());
        chunks.push(self.event(
            "response.output_item.added",
            json!({"output_index":index,"item":item}),
        ));
    }

    fn decode(&mut self, data: &str) -> Result<Vec<Bytes>, String> {
        if self.finished {
            return Ok(Vec::new());
        }
        if data == "[DONE]" {
            if !self.pending_tools.is_empty() {
                return Err("Router completed a tool call without its name".into());
            }
            return Ok(self.complete());
        }
        let value: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
        if value.get("error").is_some_and(|error| !error.is_null()) {
            return Err(value["error"]["message"]
                .as_str()
                .unwrap_or("Inference failed")
                .into());
        }
        if let Some(model) = value["model"].as_str() {
            self.model = model.into();
        }
        if let Some(usage) = value.get("usage") {
            self.usage = json!({"input_tokens":usage["prompt_tokens"],"output_tokens":usage["completion_tokens"],"total_tokens":usage["total_tokens"],
                "input_tokens_details":{"cached_tokens":usage["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0)},
                "output_tokens_details":{"reasoning_tokens":usage["completion_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or(0)}});
        }
        let mut chunks = Vec::new();
        for choice in value["choices"].as_array().into_iter().flatten().take(1) {
            if let Some(reason) = choice["finish_reason"].as_str() {
                self.finish_reason = reason.into();
            }
            let delta = choice
                .get("delta")
                .or_else(|| choice.get("message"))
                .unwrap_or(&Value::Null);
            if let Some(item) = delta.get("_echobird_response_item") {
                self.response_item(item, &mut chunks);
            }
            let native_items = delta["_echobird_response_items"].as_array();
            for item in native_items.into_iter().flatten() {
                self.response_item(item, &mut chunks);
            }
            if let Some(text) = delta["reasoning_content"]
                .as_str()
                .filter(|text| !text.is_empty() && native_items.is_none())
            {
                let index = if let Some(index) = self.reasoning_index {
                    index
                } else {
                    let index = self.output.len();
                    self.reasoning_index = Some(index);
                    let item = json!({"type":"reasoning","id":format!("rs_{}",uuid::Uuid::new_v4().simple()),"status":"in_progress","summary":[]});
                    self.output.push(item.clone());
                    chunks.push(self.event(
                        "response.output_item.added",
                        json!({"output_index":index,"item":item}),
                    ));
                    let part = json!({"type":"summary_text","text":""});
                    self.output[index]["summary"] = json!([part]);
                    chunks.push(self.event("response.reasoning_summary_part.added",json!({"output_index":index,"item_id":self.output[index]["id"],"summary_index":0,"part":part})));
                    index
                };
                if self.output[index]["summary"]
                    .as_array()
                    .map_or(true, |summary| summary.is_empty())
                {
                    let part = json!({"type":"summary_text","text":""});
                    self.output[index]["summary"] = json!([part]);
                    chunks.push(self.event("response.reasoning_summary_part.added",json!({"output_index":index,"item_id":self.output[index]["id"],"summary_index":0,"part":part})));
                }
                let previous = self.output[index]["summary"][0]["text"]
                    .as_str()
                    .unwrap_or_default();
                self.output[index]["summary"][0]["text"] = json!(format!("{previous}{text}"));
                chunks.push(self.event("response.reasoning_summary_text.delta",json!({"output_index":index,"item_id":self.output[index]["id"],"summary_index":0,"delta":text})));
            }
            if let Some(text) = delta["content"].as_str().filter(|text| !text.is_empty()) {
                let index = if let Some(index) = self.text_index {
                    index
                } else {
                    let index = self.output.len();
                    self.text_index = Some(index);
                    let item = json!({"type":"message","id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"role":"assistant","status":"in_progress","content":[]});
                    self.output.push(item.clone());
                    chunks.push(self.event(
                        "response.output_item.added",
                        json!({"output_index":index,"item":item}),
                    ));
                    let part = json!({"type":"output_text","text":"","annotations":[]});
                    self.output[index]["content"] = json!([part]);
                    chunks.push(self.event("response.content_part.added",json!({"output_index":index,"item_id":self.output[index]["id"],"content_index":0,"part":part})));
                    index
                };
                let previous = self.output[index]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default();
                self.output[index]["content"][0]["text"] = json!(format!("{previous}{text}"));
                chunks.push(self.event("response.output_text.delta",json!({"output_index":index,"item_id":self.output[index]["id"],"content_index":0,"delta":text})));
            }
            for (position, call) in delta["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let key = call["index"].as_u64().unwrap_or(position as u64);
                let mut call = call.clone();
                if !self.tool_indexes.contains_key(&key)
                    && call["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .is_empty()
                {
                    self.pending_tools.entry(key).or_default().push(call);
                    continue;
                }
                if let Some(previous) = self.pending_tools.remove(&key) {
                    let arguments = previous
                        .iter()
                        .filter_map(|call| call["function"]["arguments"].as_str())
                        .collect::<String>();
                    if call["id"].is_null() {
                        if let Some(id) = previous.iter().find_map(|call| call.get("id")) {
                            call["id"] = id.clone();
                        }
                    }
                    call["function"]["arguments"] = json!(format!(
                        "{arguments}{}",
                        call["function"]["arguments"].as_str().unwrap_or_default()
                    ));
                }
                let index = if let Some(index) = self.tool_indexes.get(&key) {
                    *index
                } else {
                    let index = self.output.len();
                    self.tool_indexes.insert(key, index);
                    let alias = call["function"]["name"].as_str().unwrap_or_default();
                    let declaration = self.tools.get(alias);
                    let custom = declaration.is_some_and(|tool| tool["type"] == "custom");
                    let name = declaration
                        .map(|tool| tool["name"].clone())
                        .unwrap_or_else(|| json!(alias));
                    let mut item = json!({"type":if custom {"custom_tool_call"} else {"function_call"},"id":format!("{}_{}",if custom {"ctc"} else {"fc"},uuid::Uuid::new_v4().simple()),"call_id":call["id"],
                        "name":name,"status":"in_progress"});
                    if let Some(id) = call.get("_echobird_response_item_id") {
                        item["id"] = id.clone();
                    }
                    item[if custom { "input" } else { "arguments" }] = json!("");
                    if let Some(namespace) = declaration.and_then(|tool| tool.get("namespace")) {
                        item["namespace"] = namespace.clone();
                    }
                    self.output.push(item.clone());
                    chunks.push(self.event(
                        "response.output_item.added",
                        json!({"output_index":index,"item":item}),
                    ));
                    index
                };
                if let Some(id) = call.get("id") {
                    self.output[index]["call_id"] = id.clone();
                }
                if let Some(name) = call["function"].get("name") {
                    self.output[index]["name"] = self
                        .tools
                        .get(name.as_str().unwrap_or_default())
                        .map(|tool| tool["name"].clone())
                        .unwrap_or_else(|| name.clone());
                }
                if let Some(arguments) = call["function"]["arguments"]
                    .as_str()
                    .filter(|a| !a.is_empty())
                {
                    if self.output[index]["type"] == "custom_tool_call" {
                        self.custom_arguments
                            .entry(index)
                            .or_default()
                            .push_str(arguments);
                    } else {
                        let old = self.output[index]["arguments"].as_str().unwrap_or_default();
                        self.output[index]["arguments"] = json!(format!("{old}{arguments}"));
                        chunks.push(self.event("response.function_call_arguments.delta",json!({"output_index":index,"item_id":self.output[index]["id"],"delta":arguments})));
                    }
                }
            }
        }
        Ok(chunks)
    }

    fn complete(&mut self) -> Vec<Bytes> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let mut chunks = Vec::new();
        for index in 0..self.output.len() {
            if self.output[index]["type"] == "custom_tool_call" {
                let input = tools::custom_input(
                    self.custom_arguments
                        .get(&index)
                        .map(String::as_str)
                        .unwrap_or_default(),
                );
                self.output[index]["input"] = json!(input);
                chunks.push(self.event(
                    "response.custom_tool_call_input.delta",
                    json!({"output_index":index,"item_id":self.output[index]["id"],"delta":input}),
                ));
                chunks.push(self.event(
                    "response.custom_tool_call_input.done",
                    json!({"output_index":index,"item_id":self.output[index]["id"],"input":input}),
                ));
            }
            self.output[index]["status"] = json!("completed");
            let item = self.output[index].clone();
            if item["type"] == "function_call" {
                chunks.push(self.event("response.function_call_arguments.done",json!({"output_index":index,"item_id":item["id"],"arguments":item["arguments"]})));
            } else if item["type"] == "reasoning" {
                for (summary_index, part) in
                    item["summary"].as_array().into_iter().flatten().enumerate()
                {
                    chunks.push(self.event("response.reasoning_summary_text.done",json!({"output_index":index,"item_id":item["id"],"summary_index":summary_index,"text":part["text"]})));
                    chunks.push(self.event("response.reasoning_summary_part.done",json!({"output_index":index,"item_id":item["id"],"summary_index":summary_index,"part":part})));
                }
            } else if item["type"] == "message" {
                chunks.push(self.event("response.output_text.done",json!({"output_index":index,"item_id":item["id"],"content_index":0,"text":item["content"][0]["text"]})));
                chunks.push(self.event("response.content_part.done",json!({"output_index":index,"item_id":item["id"],"content_index":0,"part":item["content"][0]})));
            }
            chunks.push(self.event(
                "response.output_item.done",
                json!({"output_index":index,"item":item}),
            ));
        }
        let status = if self.finish_reason == "length" {
            "incomplete"
        } else {
            "completed"
        };
        let response = self.response(status);
        chunks.push(self.event(
            if status == "incomplete" {
                "response.incomplete"
            } else {
                "response.completed"
            },
            json!({"response":response}),
        ));
        chunks
    }

    fn failed(&mut self, error: &ProxyError) -> Bytes {
        self.finished = true;
        let mut response = self.response("failed");
        let mut detail = json!({"code":error.error_code.as_deref().unwrap_or("upstream_error"),
            "message":error.message,"status":error.status.as_u16()});
        if let Some(kind) = &error.error_type {
            detail["type"] = json!(kind);
        }
        if let Some(retry_after) = error
            .retry_after
            .as_ref()
            .and_then(|value| value.to_str().ok())
        {
            detail["retry_after"] = json!(retry_after);
        }
        if error.model_scope {
            detail["scope"] = json!("model");
        }
        response["error"] = detail;
        self.event("response.failed", json!({"response":response}))
    }
}

fn stream_failure(error: &(dyn std::error::Error + 'static)) -> ProxyError {
    let mut cause = Some(error);
    while let Some(error) = cause {
        if let Some(error) = error.downcast_ref::<ProxyError>() {
            return error.clone();
        }
        cause = error
            .downcast_ref::<std::io::Error>()
            .and_then(|error| {
                error
                    .get_ref()
                    .map(|inner| inner as &(dyn std::error::Error + 'static))
            })
            .or_else(|| error.source());
    }
    ProxyError::new(StatusCode::BAD_GATEWAY, error.to_string())
}

/// Convert router output for native Responses clients without losing function calls.
pub async fn chat_to_responses(response: Response, stream: bool) -> Response {
    chat_to_responses_with_tools(response, stream, &Value::Null).await
}

/// The original declarations restore custom calls and namespaces after any account hop.
pub async fn chat_to_responses_with_tools(
    response: Response,
    stream: bool,
    request: &Value,
) -> Response {
    if !response.status().is_success() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let mut encoder = Encoder::with_tools(request);
    if !stream {
        let bytes = match to_bytes(body, 32 * 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(error) => {
                return routed_headers(conversion_error(&error.to_string()), &parts.headers)
            }
        };
        let data = match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(error) => {
                return routed_headers(conversion_error(&error.to_string()), &parts.headers)
            }
        };
        if let Err(error) = encoder.decode(&data.to_string()) {
            return routed_headers(conversion_error(&error), &parts.headers);
        }
        if !encoder.pending_tools.is_empty() {
            return routed_headers(
                conversion_error("Router completed a tool call without its name"),
                &parts.headers,
            );
        }
        encoder.complete();
        return routed_headers(
            json_response(&encoder.response(if encoder.finish_reason == "length" {
                "incomplete"
            } else {
                "completed"
            })),
            &parts.headers,
        );
    }
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);
    let response_headers = parts.headers.clone();
    tokio::spawn(async move {
        let mut upstream = body.into_data_stream();
        let mut parser = SseParser::default();
        if tx.send(Ok(encoder.created())).await.is_err() {
            return;
        }
        loop {
            let next = tokio::select! { _=tx.closed()=>return,next=upstream.next()=>next };
            let eof = next.is_none();
            let frames = match next {
                Some(Ok(bytes)) => parser.push(&bytes),
                Some(Err(error)) => {
                    let _ = tx.send(Ok(encoder.failed(&stream_failure(&error)))).await;
                    return;
                }
                None => parser.finish(),
            };
            for (event, data) in frames {
                if let Some(error) = super::stream_error(&event, &data, &response_headers) {
                    let _ = tx.send(Ok(encoder.failed(&error))).await;
                    return;
                }
                match encoder.decode(&data) {
                    Ok(chunks) => {
                        for chunk in chunks {
                            if tx.send(Ok(chunk)).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        let error = ProxyError::new(StatusCode::BAD_GATEWAY, error);
                        let _ = tx.send(Ok(encoder.failed(&error))).await;
                        return;
                    }
                }
            }
            if encoder.finished {
                return;
            }
            if eof {
                let _ = tx
                    .send(Ok(encoder.failed(&ProxyError::new(
                        StatusCode::BAD_GATEWAY,
                        "Router stream ended before completion",
                    ))))
                    .await;
                return;
            }
        }
    });
    routed_headers(
        stream_response(Body::from_stream(ReceiverStream::new(rx))),
        &parts.headers,
    )
}

fn routed_headers(mut response: Response, original: &axum::http::HeaderMap) -> Response {
    for (name, value) in original {
        if !matches!(
            name.as_str(),
            "content-type" | "content-length" | "content-encoding" | "transfer-encoding"
        ) {
            response.headers_mut().insert(name.clone(), value.clone());
        }
    }
    response
}

fn conversion_error(message: &str) -> Response {
    let mut response = json_response(&json!({"error":{"message":message,"type":"upstream_error"}}));
    *response.status_mut() = StatusCode::BAD_GATEWAY;
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom_request() -> Value {
        json!({"model":"auto","stream":true,"tools":[{"type":"custom","name":"apply_patch","description":"Patch files","format":{"type":"grammar","syntax":"lark","definition":"start: patch\npatch: PATCH\nPATCH: /[\\s\\S]+/"}},
            {"type":"namespace","name":"mcp__pad","tools":[{"type":"custom","name":"write","format":{"type":"text"}}]}],
            "input":[{"type":"custom_tool_call","call_id":"old","name":"apply_patch","input":"*** Begin Patch\n*** End Patch"},{"type":"custom_tool_call_output","call_id":"old","output":"ok"}]})
    }

    #[test]
    fn custom_request_keeps_raw_history_and_native_grammar() {
        let request = custom_request();
        let chat = responses_to_chat(&request).unwrap();
        assert_eq!(
            tools::custom_input(
                chat["messages"][0]["tool_calls"][0]["function"]["arguments"]
                    .as_str()
                    .unwrap()
            ),
            request["input"][0]["input"]
        );
        assert_eq!(chat["messages"][1]["tool_call_id"], "old");
        assert_eq!(chat["messages"][1]["content"], "ok");
        let native = super::super::protocol::chat_to_responses(&chat, "gpt-test");
        assert_eq!(native["tools"], request["tools"]);
        assert_eq!(native["input"], request["input"]);
        assert_eq!(native["model"], "gpt-test");
        let gemini = super::super::protocol::chat_to_code_assist(&chat, "test", "project");
        assert_eq!(
            gemini["request"]["contents"][0]["parts"][0]["functionCall"]["args"]["input"],
            request["input"][0]["input"]
        );
    }

    #[test]
    fn custom_stream_restores_raw_input_and_namespace_without_function_events() {
        let request = custom_request();
        let raw = "*** Begin Patch\n*** Add File: 模型.txt\n+\"hello\"\n*** End Patch";
        let arguments = json!({"input":raw}).to_string();
        let split = arguments.find("Add File").unwrap();
        let mut encoder = Encoder::with_tools(&request);
        let mut chunks=encoder.decode(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"arguments":&arguments[..split]}}]}}]}).to_string()).unwrap();
        chunks.extend(encoder.decode(&json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"apply_patch","arguments":&arguments[split..]}},{"index":1,"id":"call_b","function":{"name":"mcp__pad__write","arguments":"{\"input\":\"raw\"}"}}]}}]}).to_string()).unwrap());
        chunks.extend(encoder.decode("[DONE]").unwrap());
        let text = chunks
            .iter()
            .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
            .collect::<String>();
        assert!(!text.contains("response.function_call_arguments"));
        assert!(text.contains("response.custom_tool_call_input.delta"));
        assert!(text.contains("response.custom_tool_call_input.done"));
        assert_eq!(encoder.output[0]["type"], "custom_tool_call");
        assert_eq!(encoder.output[0]["call_id"], "call_a");
        assert_eq!(encoder.output[0]["input"], raw);
        assert!(encoder.output[0].get("arguments").is_none());
        assert_eq!(encoder.output[1]["name"], "write");
        assert_eq!(encoder.output[1]["namespace"], "mcp__pad");
    }

    #[tokio::test]
    async fn native_custom_sse_returns_original_type_in_stream_and_json() {
        use super::super::protocol::{ChatDecoder, WireProtocol};
        let request = custom_request();
        let raw = "*** Begin Patch\n*** End Patch";
        let mut decoder = ChatDecoder::new(WireProtocol::Responses, "gpt-test");
        let mut frames = Vec::new();
        for frame in [
            json!({"type":"response.output_item.added","item":{"type":"custom_tool_call","id":"ctc_a","call_id":"call_a","name":"apply_patch","input":""}}),
            json!({"type":"response.custom_tool_call_input.delta","item_id":"ctc_a","delta":"*** Begin Patch\n"}),
            json!({"type":"response.custom_tool_call_input.delta","item_id":"ctc_a","delta":"*** End Patch"}),
            json!({"type":"response.custom_tool_call_input.done","item_id":"ctc_a","input":raw}),
            json!({"type":"response.output_item.done","item":{"type":"custom_tool_call","id":"ctc_a","call_id":"call_a","name":"apply_patch","input":raw}}),
            json!({"type":"response.completed","response":{"object":"response","id":"resp_custom","status":"completed","error":null,"output":[{"type":"custom_tool_call","id":"ctc_a","call_id":"call_a","name":"apply_patch","input":raw}]}}),
        ] {
            frames.extend(decoder.decode("", &frame.to_string()).unwrap());
        }
        assert_eq!(
            tools::custom_input(
                decoder.collected()["choices"][0]["message"]["tool_calls"][0]["function"]
                    ["arguments"]
                    .as_str()
                    .unwrap()
            ),
            raw
        );
        for stream in [false, true] {
            let body = if stream {
                Body::from(
                    frames
                        .iter()
                        .flat_map(|frame| frame.to_vec())
                        .collect::<Vec<_>>(),
                )
            } else {
                Body::from(decoder.collected().to_string())
            };
            let response =
                chat_to_responses_with_tools(Response::new(body), stream, &request).await;
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let response = if stream {
                let mut parser = SseParser::default();
                let frames = parser.push(&bytes);
                frames
                    .iter()
                    .find_map(|(_, data)| {
                        let value: Value = serde_json::from_str(data).ok()?;
                        (value["type"] == "response.completed").then(|| value["response"].clone())
                    })
                    .unwrap()
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            assert_eq!(response["output"][0]["type"], "custom_tool_call");
            assert_eq!(response["output"][0]["input"], raw);
            assert_eq!(response["output"][0]["call_id"], "call_a");
        }
    }

    #[tokio::test]
    async fn stateless_tool_followup_keeps_completed_reasoning_and_native_item_ids() {
        use super::super::protocol::{ChatDecoder, WireProtocol};
        let request = custom_request();
        let reasoning = json!({"type":"reasoning","id":"rs_original","status":"completed","summary":[{"type":"summary_text","text":"thinking"}],"encrypted_content":"opaque-fixture-content"});
        let tool = json!({"type":"custom_tool_call","id":"ctc_original","call_id":"call_a","name":"apply_patch","input":"*** Begin Patch\n*** End Patch","status":"completed"});
        let mut decoder = ChatDecoder::new(WireProtocol::Responses, "gpt-test");
        let mut frames = Vec::new();
        for frame in [
            json!({"type":"response.output_item.added","item":{"type":"reasoning","id":"rs_original","summary":[],"status":"in_progress"}}),
            json!({"type":"response.reasoning_summary_text.delta","delta":"thinking"}),
            json!({"type":"response.output_item.done","item":reasoning}),
            json!({"type":"response.output_item.added","item":{"type":"custom_tool_call","id":"ctc_original","call_id":"call_a","name":"apply_patch","input":""}}),
            json!({"type":"response.output_item.done","item":tool}),
            json!({"type":"response.completed","response":{"object":"response","id":"resp_followup","status":"completed","error":null,"output":[reasoning,tool]}}),
        ] {
            frames.extend(decoder.decode("", &frame.to_string()).unwrap());
        }
        for stream in [false, true] {
            let body = if stream {
                Body::from(
                    frames
                        .iter()
                        .flat_map(|frame| frame.to_vec())
                        .collect::<Vec<_>>(),
                )
            } else {
                Body::from(decoder.collected().to_string())
            };
            let response =
                chat_to_responses_with_tools(Response::new(body), stream, &request).await;
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let result = if stream {
                let mut parser = SseParser::default();
                parser
                    .push(&bytes)
                    .iter()
                    .find_map(|(_, data)| {
                        let value: Value = serde_json::from_str(data).ok()?;
                        (value["type"] == "response.completed").then(|| value["response"].clone())
                    })
                    .unwrap()
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            assert_eq!(result["output"][0], reasoning);
            assert_eq!(result["output"][1]["id"], "ctc_original");
            let mut followup = request.clone();
            let mut input = result["output"].as_array().unwrap().clone();
            input.push(json!({"type":"custom_tool_call_output","call_id":"call_a","output":"ok"}));
            followup["input"] = json!(input);
            let chat = responses_to_chat(&followup).unwrap();
            let native = super::super::protocol::chat_to_responses(&chat, "gpt-test");
            assert_eq!(native["input"], followup["input"]);
            assert_eq!(
                native["input"][0]["encrypted_content"],
                "opaque-fixture-content"
            );
        }
    }

    #[test]
    fn code_mode_additional_tools_are_declared_and_hosted_search_is_explicitly_rejected() {
        let request = json!({"model":"auto","input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"functions","tools":[{"type":"custom","name":"exec","format":{"type":"text"}},{"type":"function","name":"wait","parameters":{"type":"object"}}]}]},
            {"type":"custom_tool_call","call_id":"a","name":"exec","namespace":"functions","input":"text('ok')"},{"type":"custom_tool_call_output","call_id":"a","output":"ok"}]});
        let chat = responses_to_chat(&request).unwrap();
        assert_eq!(chat["tools"].as_array().unwrap().len(), 2);
        assert_eq!(
            chat["messages"][0]["tool_calls"][0]["function"]["name"],
            "exec"
        );
        assert_eq!(
            super::super::protocol::chat_to_responses(&chat, "native")["input"],
            request["input"]
        );
        let unsupported = json!({"input":"hello","tools":[{"type":"web_search"}]});
        assert_eq!(
            responses_to_chat(&unsupported).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn conversion_preserves_routing_trace_and_replaces_body_headers() {
        for stream in [false, true] {
            let body = if stream {
                Body::from("data: {\"model\":\"actual\",\"error\":null,\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n")
            } else {
                Body::from(json!({"model":"actual","error":null,"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}).to_string())
            };
            let response = Response::builder()
                .header("X-EchoBird-Routed-Via", "own-model")
                .header("content-length", "1")
                .header("content-type", "application/x-original")
                .body(body)
                .unwrap();
            let converted = chat_to_responses(response, stream).await;
            assert_eq!(converted.headers()["X-EchoBird-Routed-Via"], "own-model");
            assert!(!converted.headers().contains_key("content-length"));
            let bytes = to_bytes(converted.into_body(), 1024 * 1024).await.unwrap();
            let content = String::from_utf8(bytes.to_vec()).unwrap();
            if stream {
                assert!(content.contains("response.completed"));
                assert!(content.contains("response.output_text.delta"));
                assert!(!content.contains("response.failed"));
            } else {
                assert_eq!(
                    serde_json::from_str::<Value>(&content).unwrap()["status"],
                    "completed"
                );
                assert_eq!(
                    serde_json::from_str::<Value>(&content).unwrap()["output"][0]["content"][0]
                        ["text"],
                    "ok"
                );
            }
        }
    }

    #[tokio::test]
    async fn conversion_rejects_upstream_errors_in_stream_and_json() {
        let failure = json!({"error":{"code":"rate_limit_exceeded","message":"Quota exhausted"}});
        for stream in [false, true] {
            let body = if stream {
                Body::from(format!("data: {failure}\n\ndata: [DONE]\n\n"))
            } else {
                Body::from(failure.to_string())
            };
            let response = Response::builder()
                .header("X-EchoBird-Routed-Via", "own-model")
                .body(body)
                .unwrap();
            let converted = chat_to_responses(response, stream).await;
            assert_eq!(converted.headers()["X-EchoBird-Routed-Via"], "own-model");
            assert_eq!(
                converted.status(),
                if stream {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_GATEWAY
                }
            );
            let bytes = to_bytes(converted.into_body(), 1024 * 1024).await.unwrap();
            if stream {
                let mut parser = SseParser::default();
                let events = parser.push(&bytes);
                assert!(!events
                    .iter()
                    .any(|(event, _)| event == "response.completed"));
                let failed = events
                    .iter()
                    .find(|(event, _)| event == "response.failed")
                    .unwrap();
                let value: Value = serde_json::from_str(&failed.1).unwrap();
                assert_eq!(value["response"]["error"]["message"], "Quota exhausted");
            } else {
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["error"]["message"], "Quota exhausted");
            }
        }
    }

    #[tokio::test]
    async fn conversion_does_not_complete_a_truncated_stream_with_null_error() {
        let response = Response::new(Body::from(
            "data: {\"error\":null,\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
        ));
        let converted = chat_to_responses(response, true).await;
        let bytes = to_bytes(converted.into_body(), 1024 * 1024).await.unwrap();
        let mut parser = SseParser::default();
        let events = parser.push(&bytes);
        assert!(events
            .iter()
            .any(|(event, _)| event == "response.output_text.delta"));
        assert!(!events
            .iter()
            .any(|(event, _)| event == "response.completed"));
        let failed = events
            .iter()
            .find(|(event, _)| event == "response.failed")
            .unwrap();
        let value: Value = serde_json::from_str(&failed.1).unwrap();
        assert_eq!(
            value["response"]["error"]["message"],
            "Router stream ended before completion"
        );
    }

    #[test]
    fn responses_round_trip_preserves_parallel_calls_and_results() {
        let input = json!({"model":"auto","input":[{"type":"function_call","call_id":"a","name":"read","arguments":"{}"},
            {"type":"function_call_output","call_id":"a","output":"ok"}],"tools":[{"type":"function","name":"read","parameters":{"type":"object"}}]});
        let chat = responses_to_chat(&input).unwrap();
        assert_eq!(chat["messages"][0]["tool_calls"][0]["id"], "a");
        assert_eq!(chat["messages"][1]["tool_call_id"], "a");
        let mut encoder = Encoder::new();
        encoder.decode(&json!({"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"write","arguments":"{"}},
            {"index":0,"id":"a","function":{"name":"read","arguments":"{}"}}]}}]}).to_string()).unwrap();
        encoder.decode(&json!({"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"p\":1}"}}]},"finish_reason":"tool_calls"}]}).to_string()).unwrap();
        let chunks = encoder.decode("[DONE]").unwrap();
        assert!(String::from_utf8(chunks.last().unwrap().to_vec())
            .unwrap()
            .contains("response.completed"));
        assert_eq!(encoder.output[0]["arguments"], "{\"p\":1}");
        assert_eq!(encoder.output[1]["call_id"], "a");
    }
}
