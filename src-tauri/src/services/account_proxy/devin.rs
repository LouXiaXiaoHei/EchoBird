//! Devin subscription transport. Requests use the selected saved account only.
use super::{json_response, stream_response, ProxyError};
use crate::services::{
    devin_accounts,
    devin_protocol::{self as wire, Field, Message},
};
use axum::{body::Body, http::StatusCode, response::Response};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::OnceLock,
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

const MODELS: &str = "/exa.api_server_pb.ApiServerService/GetCliModelConfigs";
const CHAT: &str = "/exa.api_server_pb.ApiServerService/GetChatMessage";
const LIMIT: usize = 64 * 1024 * 1024;
fn client() -> Result<&'static reqwest::Client, ProxyError> {
    static CLIENT: OnceLock<Result<reqwest::Client, &'static str>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .tcp_keepalive(Duration::from_secs(60))
                .build()
                .map_err(|_| "Devin HTTP client is unavailable")
        })
        .as_ref()
        .map_err(|e| ProxyError::upstream(*e))
}
fn auth(c: &Value) -> Result<(&str, &str), ProxyError> {
    let key = c["accessToken"]
        .as_str()
        .filter(|s| s.starts_with("devin-session-token$"))
        .ok_or_else(|| {
            ProxyError::new(StatusCode::UNAUTHORIZED, "Devin account requires sign-in")
        })?;
    let server = c["baseUrl"]
        .as_str()
        .unwrap_or(devin_accounts::SERVER)
        .trim_end_matches('/');
    let url = url::Url::parse(server).map_err(|_| ProxyError::upstream("Invalid Devin server"))?;
    #[cfg(test)]
    if url.scheme() == "http" && url.host_str() == Some("127.0.0.1") {
        return Ok((key, server));
    }
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProxyError::upstream("Invalid Devin server"));
    }
    Ok((key, server))
}
fn metadata(key: &str) -> Vec<u8> {
    Message::default()
        .bytes(1, "devin-cli")
        .bytes(2, devin_accounts::VERSION)
        .bytes(3, key)
        .bytes(4, "en")
        .bytes(5, std::env::consts::OS)
        .bytes(7, devin_accounts::VERSION)
        .bytes(12, "chisel")
        .bytes(28, "chisel")
        .0
}
fn error(status: StatusCode, payload: &[u8]) -> ProxyError {
    let v: Value = serde_json::from_slice(payload).unwrap_or(Value::Null);
    let v = v.get("error").unwrap_or(&v);
    let code = v["code"].as_str().unwrap_or("");
    let status = match code {
        "unauthenticated" => StatusCode::UNAUTHORIZED,
        "permission_denied" => StatusCode::FORBIDDEN,
        "resource_exhausted" => StatusCode::TOO_MANY_REQUESTS,
        "invalid_argument" | "out_of_range" => StatusCode::BAD_REQUEST,
        _ if status.is_redirection() => StatusCode::BAD_GATEWAY,
        _ if status.is_success() => StatusCode::BAD_GATEWAY,
        _ => status,
    };
    // Upstream payloads can include request details. Keep credentials out of errors.
    let message = match status {
        StatusCode::UNAUTHORIZED => "Devin sign-in has expired",
        StatusCode::FORBIDDEN => "Devin account cannot use this model",
        StatusCode::TOO_MANY_REQUESTS => "Devin usage limit reached",
        StatusCode::BAD_REQUEST => "Devin rejected the model or request parameters",
        StatusCode::PAYLOAD_TOO_LARGE => "Devin request is too large",
        _ => "Devin upstream request failed",
    };
    ProxyError::new(status, message)
}
pub(super) async fn list_models(
    c: Value,
    _client: &reqwest::Client,
) -> Result<Vec<String>, ProxyError> {
    let client = client()?;
    let (key, server) = auth(&c)?;
    let response = client
        .post(format!("{server}{MODELS}"))
        .header("content-type", "application/proto")
        .header("Connect-Protocol-Version", "1")
        .timeout(Duration::from_secs(30))
        .body(Message::default().bytes(1, metadata(key)).0)
        .send()
        .await
        .map_err(|_| ProxyError::upstream("Devin model directory is unavailable"))?;
    if !response.status().is_success() {
        return Err(error(response.status(), &[]));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ProxyError::upstream("Cannot read Devin model directory"))?;
        if bytes.len().saturating_add(chunk.len()) > LIMIT {
            return Err(ProxyError::upstream("Devin model directory is too large"));
        }
        bytes.extend(chunk);
    }
    let mut ids = Vec::new();
    let mut free = Vec::new();
    for f in wire::fields(&bytes).map_err(ProxyError::upstream)? {
        if let Field::Bytes(1, b) = f {
            let fields = wire::fields(b).map_err(ProxyError::upstream)?;
            if let Some(id) = wire::string(&fields, 22).filter(|s| {
                wire::number(&fields, 4) != Some(1)
                    && !s.is_empty()
                    && !matches!(s.to_ascii_lowercase().as_str(), "adaptive" | "fusion")
            }) {
                if wire::number(&fields, 24) == Some(4) {
                    free.push(id.clone());
                }
                ids.push(id);
            }
        }
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(ProxyError::upstream("Devin returned no available models"));
    }
    super::providers::remember_free_model_ids(
        crate::models::model::AccountProvider::Devin,
        &c,
        free,
    );
    Ok(ids)
}
fn tool_id_out(id: &str) -> String {
    if !id.is_empty()
        && !id.starts_with("ebdv_")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        id.into()
    } else {
        format!("ebdv_{}", URL_SAFE_NO_PAD.encode(id))
    }
}
fn tool_id_in(id: &str) -> String {
    id.strip_prefix("ebdv_")
        .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .filter(|raw| tool_id_out(raw) == id)
        .unwrap_or_else(|| id.into())
}
#[derive(Default)]
struct Prompt {
    role: u64,
    text: String,
    calls: Vec<(String, String, String)>,
    result: Option<String>,
    images: Vec<(String, String)>,
}
impl Prompt {
    fn encode(&self) -> Vec<u8> {
        let mut m = Message::default()
            .bytes(1, uuid::Uuid::new_v4().to_string())
            .num(2, self.role)
            .bytes(3, &self.text);
        for (id, name, args) in &self.calls {
            m = m.bytes(
                6,
                Message::default()
                    .bytes(1, id)
                    .bytes(2, name)
                    .bytes(3, args)
                    .0,
            );
        }
        if let Some(id) = &self.result {
            m = m.bytes(7, id);
        }
        for (data, mime) in &self.images {
            m = m.bytes(10, Message::default().bytes(1, data).bytes(2, mime).0);
        }
        m.0
    }
}
fn content(value: &Value) -> Result<(String, Vec<(String, String)>), ProxyError> {
    if let Some(s) = value.as_str() {
        return Ok((s.into(), vec![]));
    }
    let mut text = String::new();
    let mut images = vec![];
    for p in value.as_array().into_iter().flatten() {
        if p["type"] == "text" {
            text.push_str(p["text"].as_str().unwrap_or(""));
        } else if p["type"] == "image_url" {
            let url = p["image_url"]["url"].as_str().unwrap_or("");
            let Some((mime, data)) = url
                .strip_prefix("data:")
                .and_then(|s| s.split_once(";base64,"))
            else {
                return Err(ProxyError::new(
                    StatusCode::BAD_REQUEST,
                    "Devin images require inline data URLs",
                ));
            };
            if !mime.starts_with("image/") || STANDARD.decode(data).is_err() {
                return Err(ProxyError::new(
                    StatusCode::BAD_REQUEST,
                    "Invalid Devin image data",
                ));
            }
            images.push((data.into(), mime.into()));
        } else {
            return Err(ProxyError::new(
                StatusCode::BAD_REQUEST,
                "Unsupported Devin content part",
            ));
        }
    }
    Ok((text, images))
}
fn flush_missing(pending: &mut BTreeMap<String, String>, messages: &mut Vec<Prompt>) {
    for (id, _) in std::mem::take(pending) {
        messages.push(Prompt {
            role: 4,
            text: "The tool was interrupted before returning a result.".into(),
            result: Some(id),
            ..Default::default()
        });
    }
}
fn request(key: &str, model: &str, body: &Value) -> Result<Vec<u8>, ProxyError> {
    super::protocol::reject_structured_output(body)?;
    if let Some(choice) = body.get("tool_choice").filter(|v| !v.is_null()) {
        if !matches!(choice.as_str(), Some("auto" | "none")) {
            return Err(ProxyError::new(
                StatusCode::BAD_REQUEST,
                "Devin does not expose forced tool selection",
            ));
        }
    }
    let mut messages: Vec<Prompt> = vec![];
    let mut system = vec![];
    let mut pending = BTreeMap::new();
    let input = body["messages"]
        .as_array()
        .ok_or_else(|| ProxyError::new(StatusCode::BAD_REQUEST, "Messages are required"))?;
    for row in input {
        let (text, images) = content(&row["content"])?;
        match row["role"].as_str() {
            Some("system" | "developer") => system.push(text),
            Some("assistant") => {
                let merge = messages
                    .last()
                    .is_some_and(|m| m.role == 2 && !m.calls.is_empty());
                if !merge {
                    flush_missing(&mut pending, &mut messages);
                }
                let mut calls = Vec::new();
                for call in row["tool_calls"].as_array().into_iter().flatten() {
                    let id = tool_id_in(call["id"].as_str().unwrap_or(""));
                    let name = call["function"]["name"].as_str().unwrap_or("").to_string();
                    if id.is_empty() || name.is_empty() {
                        return Err(ProxyError::new(
                            StatusCode::BAD_REQUEST,
                            "Incomplete Devin tool call",
                        ));
                    }
                    let args = call["function"]["arguments"]
                        .as_str()
                        .unwrap_or("{}")
                        .to_string();
                    if serde_json::from_str::<Value>(&args).is_err() {
                        return Err(ProxyError::new(
                            StatusCode::BAD_REQUEST,
                            "Invalid Devin tool arguments",
                        ));
                    }
                    if pending.insert(id.clone(), name.clone()).is_some() {
                        return Err(ProxyError::new(
                            StatusCode::BAD_REQUEST,
                            "Duplicate Devin tool call id",
                        ));
                    }
                    calls.push((id, name, args));
                }
                if let Some(last) = messages
                    .last_mut()
                    .filter(|m| m.role == 2 && !m.calls.is_empty())
                {
                    last.text.push_str(&text);
                    last.calls.extend(calls);
                } else if !text.is_empty() || !calls.is_empty() {
                    messages.push(Prompt {
                        role: 2,
                        text,
                        calls,
                        ..Default::default()
                    });
                }
            }
            Some("tool") => {
                let id = tool_id_in(row["tool_call_id"].as_str().unwrap_or(""));
                if pending.remove(&id).is_some() {
                    messages.push(Prompt {
                        role: 4,
                        text,
                        result: Some(id),
                        images,
                        ..Default::default()
                    });
                }
            }
            Some("user") => {
                flush_missing(&mut pending, &mut messages);
                messages.push(Prompt {
                    role: 1,
                    text,
                    images,
                    ..Default::default()
                });
            }
            _ => {
                return Err(ProxyError::new(
                    StatusCode::BAD_REQUEST,
                    "Unsupported Devin message role",
                ))
            }
        }
    }
    flush_missing(&mut pending, &mut messages);
    if messages.is_empty() {
        return Err(ProxyError::new(
            StatusCode::BAD_REQUEST,
            "Messages cannot be empty",
        ));
    }
    let mut tools = BTreeMap::<String, (String, Value)>::new();
    if body["tool_choice"] != "none" {
        for tool in body["tools"].as_array().into_iter().flatten() {
            let f = &tool["function"];
            let name = f["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    ProxyError::new(StatusCode::BAD_REQUEST, "Invalid Devin tool definition")
                })?;
            tools.insert(
                name.into(),
                (
                    f["description"].as_str().unwrap_or("").into(),
                    f.get("parameters")
                        .cloned()
                        .unwrap_or(json!({"type":"object","properties":{}})),
                ),
            );
        }
    }
    for message in &messages {
        for (_, name, _) in &message.calls {
            if body["tool_choice"] == "none" {
                return Err(ProxyError::new(
                    StatusCode::BAD_REQUEST,
                    "Devin cannot disable tools while replaying tool history",
                ));
            }
            tools
                .entry(name.clone())
                .or_insert((String::new(), json!({"type":"object","properties":{}})));
        }
    }
    if !tools.is_empty() {
        system.push(format!(
            "Function tool descriptions:\n{}",
            tools
                .iter()
                .map(
                    |(name, (description, _))| json!({"name":name,"description":description})
                        .to_string()
                )
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let instructions = system.join("\n\n");
    if !instructions.is_empty() {
        if let Some(first) = messages.iter_mut().find(|p| p.role == 1) {
            first.text = format!("{instructions}\n\n{}", first.text);
        } else {
            messages.insert(
                0,
                Prompt {
                    role: 1,
                    text: instructions,
                    ..Default::default()
                },
            );
        }
    }
    let mut out = Message::default().bytes(1, metadata(key));
    for message in messages {
        out = out.bytes(3, message.encode());
    }
    let max = body["max_completion_tokens"]
        .as_u64()
        .or_else(|| body["max_tokens"].as_u64())
        .unwrap_or(128000);
    let temperature = body["temperature"].as_f64().unwrap_or(1.0).max(0.000001);
    let top_p = body["top_p"].as_f64().unwrap_or(0.95).max(0.000001);
    out = out.num(7, 5).bytes(
        8,
        Message::default()
            .num(1, 1)
            .num(2, max)
            .num(3, 400)
            .double(5, temperature)
            .num(7, 40)
            .double(8, top_p)
            .0,
    );
    for (name, (_, schema)) in tools {
        out = out.bytes(
            10,
            Message::default()
                .bytes(1, &name)
                .bytes(
                    2,
                    format!("Function {name}; details are in the instructions."),
                )
                .bytes(3, schema.to_string())
                .0,
        );
    }
    out = out.bytes(21, model);
    if out.0.len() > LIMIT {
        return Err(ProxyError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Devin request is too large",
        ));
    }
    Ok(out.0)
}
fn frame(message: &[u8]) -> Result<(Vec<u8>, bool), ProxyError> {
    let compressed = message.len() > 8192;
    let bytes = if compressed {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(message).map_err(ProxyError::upstream)?;
        e.finish().map_err(ProxyError::upstream)?
    } else {
        message.to_vec()
    };
    let mut frame = vec![u8::from(compressed)];
    frame.extend((bytes.len() as u32).to_be_bytes());
    frame.extend(bytes);
    Ok((frame, compressed))
}
#[derive(Default)]
struct Decoder {
    buffer: Vec<u8>,
    ended: bool,
    current: Option<usize>,
    tools: Vec<(String, String)>,
    input: u64,
    output: u64,
    cache: u64,
    cache_write: u64,
    stop: u64,
}
impl Decoder {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Value>, ProxyError> {
        if self.ended && !bytes.is_empty() {
            return Err(ProxyError::upstream("Data after Devin stream end"));
        }
        if self.buffer.len() + bytes.len() > LIMIT + 5 {
            return Err(ProxyError::upstream("Devin stream frame is too large"));
        }
        self.buffer.extend(bytes);
        let mut events = vec![];
        while self.buffer.len() >= 5 {
            let flag = self.buffer[0];
            let length =
                u32::from_be_bytes(self.buffer[1..5].try_into().expect("checked header")) as usize;
            if length > LIMIT || flag & !3 != 0 {
                return Err(ProxyError::upstream("Invalid Devin stream frame"));
            }
            if self.buffer.len() < length + 5 {
                break;
            }
            let mut payload = self.buffer[5..length + 5].to_vec();
            self.buffer.drain(..length + 5);
            if flag & 1 != 0 {
                let mut output = Vec::new();
                flate2::read::GzDecoder::new(payload.as_slice())
                    .take((LIMIT + 1) as u64)
                    .read_to_end(&mut output)
                    .map_err(|_| ProxyError::upstream("Invalid Devin compressed frame"))?;
                if output.len() > LIMIT {
                    return Err(ProxyError::upstream("Devin decoded frame is too large"));
                }
                payload = output;
            }
            if flag & 2 != 0 {
                let trailer: Value = serde_json::from_slice(&payload)
                    .map_err(|_| ProxyError::upstream("Invalid Devin stream trailer"))?;
                if trailer.get("error").is_some_and(|v| !v.is_null()) {
                    return Err(error(StatusCode::OK, &payload));
                }
                self.ended = true;
                if !self.buffer.is_empty() {
                    return Err(ProxyError::upstream("Data after Devin stream end"));
                }
                let prompt = self
                    .input
                    .saturating_add(self.cache)
                    .saturating_add(self.cache_write);
                events.push(json!({"finish_reason":if !self.tools.is_empty()||self.stop==10{"tool_calls"}else if self.stop==3{"length"}else{"stop"},"usage":{"prompt_tokens":prompt,"completion_tokens":self.output,"total_tokens":prompt.saturating_add(self.output),"prompt_tokens_details":{"cached_tokens":self.cache,"cache_write_tokens":self.cache_write}}}));
                break;
            }
            for f in wire::fields(&payload).map_err(ProxyError::upstream)? {
                match f {
                    Field::Bytes(n, b) if n == 3 || n == 9 => {
                        let text = std::str::from_utf8(b)
                            .map_err(|_| ProxyError::upstream("Invalid Devin UTF-8"))?;
                        events.push(
                            json!({"delta":{if n==3{"content"}else{"reasoning_content"}:text}}),
                        );
                    }
                    Field::Bytes(6, b) => {
                        let f = wire::fields(b).map_err(ProxyError::upstream)?;
                        let name = wire::string(&f, 2).unwrap_or_default();
                        let id = wire::string(&f, 1).unwrap_or_default();
                        if !id.is_empty() {
                            self.current = self.tools.iter().position(|t| t.0 == id);
                        }
                        if !name.is_empty() {
                            let existing = (!id.is_empty())
                                .then(|| self.tools.iter().position(|t| t.0 == id))
                                .flatten();
                            let index = existing.unwrap_or(self.tools.len());
                            self.current = Some(index);
                            if existing.is_none() {
                                self.tools.push((id.clone(), name.clone()));
                                events.push(json!({"delta":{"tool_calls":[{"index":index,"id":if id.is_empty(){format!("call_{}",uuid::Uuid::new_v4().simple())}else{tool_id_out(&id)},"type":"function","function":{"name":name,"arguments":""}}]}}));
                            }
                        }
                        if let Some(args) = wire::string(&f, 3).filter(|s| !s.is_empty()) {
                            let index = self.current.ok_or_else(|| {
                                ProxyError::upstream("Devin tool arguments arrived without a tool")
                            })?;
                            events.push(json!({"delta":{"tool_calls":[{"index":index,"function":{"arguments":args}}]}}));
                        }
                    }
                    Field::Number(5, n) => self.stop = n,
                    Field::Bytes(7, b) => {
                        let f = wire::fields(b).map_err(ProxyError::upstream)?;
                        if let Some(n) = wire::number(&f, 2) {
                            self.input = n;
                        }
                        if let Some(n) = wire::number(&f, 3) {
                            self.output = n;
                        }
                        if let Some(n) = wire::number(&f, 4) {
                            self.cache_write = n;
                        }
                        if let Some(n) = wire::number(&f, 5) {
                            self.cache = n;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(events)
    }
    fn finish(&self) -> Result<(), ProxyError> {
        if !self.ended || !self.buffer.is_empty() {
            Err(ProxyError::upstream("Devin reply ended unexpectedly"))
        } else {
            Ok(())
        }
    }
}
async fn forward(response: reqwest::Response, tx: mpsc::Sender<Result<Value, ProxyError>>) {
    let mut stream = response.bytes_stream();
    let mut decoder = Decoder::default();
    loop {
        let next = tokio::select! {_ = tx.closed()=>return,next=stream.next()=>next};
        match next {
            Some(Ok(bytes)) => match decoder.feed(&bytes) {
                Ok(events) => {
                    for e in events {
                        if tx.send(Ok(e)).await.is_err() {
                            return;
                        }
                    }
                    if decoder.ended {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            },
            Some(Err(_)) => {
                let _ = tx
                    .send(Err(ProxyError::upstream("Devin connection interrupted")))
                    .await;
                return;
            }
            None => {
                if let Err(e) = decoder.finish() {
                    let _ = tx.send(Err(e)).await;
                }
                return;
            }
        }
    }
}
pub(super) async fn route(
    c: Value,
    model: &str,
    body: Value,
    _client: &reqwest::Client,
) -> Result<Response, ProxyError> {
    let client = client()?;
    let (key, server) = auth(&c)?;
    let (raw, gzip) = frame(&request(key, model, &body)?)?;
    let mut req = client
        .post(format!("{server}{CHAT}"))
        .header("Authorization", format!("Basic {key}"))
        .header("content-type", "application/connect+proto")
        .header("Connect-Protocol-Version", "1")
        .header("Connect-Accept-Encoding", "gzip")
        .body(raw);
    if gzip {
        req = req.header("Connect-Content-Encoding", "gzip");
    }
    let response = req
        .send()
        .await
        .map_err(|_| ProxyError::upstream("Devin service is unavailable"))?;
    if !response.status().is_success() {
        return Err(error(response.status(), &[]));
    }
    let (tx, mut rx) = mpsc::channel(16);
    tokio::spawn(forward(response, tx));
    let id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
    let model = model.to_owned();
    let created = chrono::Utc::now().timestamp();
    if body["stream"].as_bool().unwrap_or(false) {
        let (out, input) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {_ = out.closed()=>return,event=rx.recv()=>event};
                let Some(event) = event else {
                    break;
                };
                let value = match event {
                    Ok(e) => {
                        json!({"id":id,"object":"chat.completion.chunk","created":created,"model":model,"choices":[{"index":0,"delta":e.get("delta").cloned().unwrap_or(json!({})),"finish_reason":e.get("finish_reason").cloned().unwrap_or(Value::Null)}],"usage":e.get("usage")})
                    }
                    Err(e) => {
                        json!({"error":{"message":e.message,"type":"devin_error","status":e.status.as_u16()}})
                    }
                };
                if out
                    .send(Ok(Bytes::from(format!("data: {value}\n\n"))))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = out.send(Ok(Bytes::from_static(b"data: [DONE]\n\n"))).await;
        });
        return Ok(stream_response(Body::from_stream(ReceiverStream::new(
            input,
        ))));
    }
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tools = BTreeMap::<u64, Value>::new();
    let mut finish = None;
    let mut usage = Value::Null;
    let mut received = 0_usize;
    while let Some(e) = rx.recv().await {
        let e = e?;
        received = received.saturating_add(e.to_string().len());
        if received > LIMIT {
            return Err(ProxyError::upstream("Devin reply is too large"));
        }
        if let Some(t) = e["delta"]["content"].as_str() {
            text.push_str(t);
        }
        if let Some(t) = e["delta"]["reasoning_content"].as_str() {
            reasoning.push_str(t);
        }
        for t in e["delta"]["tool_calls"].as_array().into_iter().flatten() {
            let index = t["index"]
                .as_u64()
                .ok_or_else(|| ProxyError::upstream("Invalid Devin tool index"))?;
            let entry=tools.entry(index).or_insert_with(||json!({"id":t["id"],"type":"function","function":{"name":t["function"]["name"],"arguments":""}}));
            if let Some(args) = t["function"]["arguments"].as_str() {
                let old = entry["function"]["arguments"].as_str().unwrap_or("");
                entry["function"]["arguments"] = json!(format!("{old}{args}"));
            }
        }
        if e.get("finish_reason").is_some() {
            finish = e["finish_reason"].as_str().map(str::to_owned);
            usage = e["usage"].clone();
        }
    }
    let finish = finish.ok_or_else(|| ProxyError::upstream("Devin reply ended unexpectedly"))?;
    let mut message = json!({"role":"assistant","content":text});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !tools.is_empty() {
        message["tool_calls"] = json!(tools.into_values().collect::<Vec<_>>());
    }
    Ok(json_response(
        &json!({"id":id,"object":"chat.completion","created":created,"model":model,"choices":[{"index":0,"message":message,"finish_reason":finish}],"usage":usage}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_session_is_never_replayed_to_a_redirect_target() {
        use axum::{routing::post, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let redirect = || async {
            (
                StatusCode::TEMPORARY_REDIRECT,
                [("location", "/destination")],
            )
        };
        let app = Router::new()
            .route(MODELS, post(redirect))
            .route(CHAT, post(redirect))
            .route(
                "/destination",
                post(move || {
                    let seen = seen.clone();
                    async move {
                        seen.fetch_add(1, Ordering::SeqCst);
                        "unexpected"
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let c = json!({"accessToken":"devin-session-token$fixture","baseUrl":base});
        let inherited = reqwest::Client::new();
        assert_eq!(
            list_models(c.clone(), &inherited).await.unwrap_err().status,
            502
        );
        assert_eq!(
            route(
                c,
                "fixture",
                json!({"messages":[{"role":"user","content":"hello"}]}),
                &inherited
            )
            .await
            .unwrap_err()
            .status,
            502
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
        task.abort();
    }
    #[tokio::test]
    async fn selected_accounts_have_isolated_models_requests_and_stream_results() {
        use axum::body::to_bytes;
        use axum::{extract::Request, routing::post, Router};
        let model_handler = post(|r: Request| async move {
            let data = to_bytes(r.into_body(), LIMIT).await.unwrap();
            let f = wire::fields(&data).unwrap();
            let meta = f
                .iter()
                .find_map(|f| {
                    if let Field::Bytes(1, b) = f {
                        Some(*b)
                    } else {
                        None
                    }
                })
                .unwrap();
            let meta = wire::fields(meta).unwrap();
            let token = wire::string(&meta, 3).unwrap();
            let id = if token.ends_with("one") {
                "model-one"
            } else {
                "model-two"
            };
            Message::default()
                .bytes(1, Message::default().bytes(22, id).0)
                .bytes(
                    1,
                    Message::default().bytes(22, "disabled-model").num(4, 1).0,
                )
                .0
        });
        let chat_handler = post(|r: Request| async move {
            let token = r.headers()["authorization"].to_str().unwrap().to_owned();
            let gzip = r.headers().get("connect-content-encoding").is_some();
            let data = to_bytes(r.into_body(), LIMIT).await.unwrap();
            let mut raw = data[5..].to_vec();
            if gzip {
                let mut decoded = Vec::new();
                flate2::read::GzDecoder::new(raw.as_slice())
                    .read_to_end(&mut decoded)
                    .unwrap();
                raw = decoded;
            }
            let fields = wire::fields(&raw).unwrap();
            let model = wire::string(&fields, 21).unwrap();
            assert!(token.starts_with("Basic devin-session-token$"));
            assert_eq!(gzip, model == "model-two");
            let meta = fields
                .iter()
                .find_map(|f| {
                    if let Field::Bytes(1, b) = f {
                        Some(*b)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert!(
                token
                    == format!(
                        "Basic {}",
                        wire::string(&wire::fields(meta).unwrap(), 3).unwrap()
                    )
            );
            let text = if token.ends_with("one") {
                "reply-one"
            } else {
                "reply-two"
            };
            let message = Message::default()
                .bytes(3, text)
                .bytes(9, "reason")
                .bytes(7, Message::default().num(2, 10).num(3, 2).num(5, 4).0)
                .num(5, 1)
                .0;
            let (mut framed, _) = frame(&message).unwrap();
            framed.extend([2, 0, 0, 0, 2, b'{', b'}']);
            let chunks: Vec<Result<Bytes, std::io::Error>> = framed
                .chunks(3)
                .map(|b| Ok(Bytes::copy_from_slice(b)))
                .collect();
            Response::builder()
                .header("content-type", "application/connect+proto")
                .body(Body::from_stream(futures_util::stream::iter(chunks)))
                .unwrap()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route(MODELS, model_handler)
            .route(CHAT, chat_handler);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();
        for (suffix, stream) in [("one", false), ("two", true)] {
            let c = json!({"accessToken":format!("devin-session-token${suffix}"),"baseUrl":base});
            let ids = list_models(c.clone(), &client).await.unwrap();
            assert_eq!(ids, vec![format!("model-{suffix}")]);
            let content = if stream {
                "long ".repeat(2000)
            } else {
                "hello".into()
            };
            let response = route(
                c,
                &ids[0],
                json!({"messages":[{"role":"user","content":content}],"stream":stream}),
                &client,
            )
            .await
            .unwrap();
            let bytes = to_bytes(response.into_body(), LIMIT).await.unwrap();
            let reply = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(reply.contains(&format!("reply-{suffix}")));
            assert!(!reply.contains("devin-session-token$"));
            if stream {
                assert!(reply.ends_with("data: [DONE]\n\n"));
                assert!(reply.contains("\"prompt_tokens\":14"));
            } else {
                let value: Value = serde_json::from_str(&reply).unwrap();
                assert_eq!(value["usage"]["total_tokens"], 16);
                assert_eq!(value["choices"][0]["finish_reason"], "stop");
            }
        }
        task.abort();
    }
    #[test]
    fn interleaved_tool_arguments_and_error_trailers() {
        let mut d = Decoder::default();
        let call = |id: &str, name: &str, args: &str| {
            Message::default()
                .bytes(
                    6,
                    Message::default()
                        .bytes(1, id)
                        .bytes(2, name)
                        .bytes(3, args)
                        .0,
                )
                .0
        };
        d.feed(&frame(&call("id:one", "read", "{")).unwrap().0)
            .unwrap();
        d.feed(&frame(&call("id:two", "write", "{}")).unwrap().0)
            .unwrap();
        let events = d.feed(&frame(&call("id:one", "", "}")).unwrap().0).unwrap();
        assert_eq!(events[0]["delta"]["tool_calls"][0]["index"], 0);
        let trailer =
            json!({"error":{"code":"resource_exhausted","message":"secret fixture"}}).to_string();
        let mut f = vec![2];
        f.extend((trailer.len() as u32).to_be_bytes());
        f.extend(trailer.as_bytes());
        let error = d.feed(&f).unwrap_err();
        assert_eq!(error.status, 429);
        assert!(!error.message.contains("secret fixture"));
    }
    #[test]
    fn fragmented_frames_and_tool_id_roundtrip() {
        for id in ["Bash:0#fixture", "call_safe", "ebdv_already", "工具"] {
            assert_eq!(tool_id_in(&tool_id_out(id)), id);
        }
        let message = Message::default()
            .bytes(3, "hello")
            .bytes(
                6,
                Message::default()
                    .bytes(1, "tool:0")
                    .bytes(2, "read_file")
                    .bytes(3, "{}")
                    .0,
            )
            .0;
        let (mut bytes, _) = frame(&message).unwrap();
        let trailer = b"{}";
        bytes.push(2);
        bytes.extend((trailer.len() as u32).to_be_bytes());
        bytes.extend(trailer);
        let mut decoder = Decoder::default();
        let mut all = vec![];
        for chunk in bytes.chunks(3) {
            all.extend(decoder.feed(chunk).unwrap());
        }
        decoder.finish().unwrap();
        assert_eq!(all.last().unwrap()["finish_reason"], "tool_calls");
        assert_eq!(all[0]["delta"]["content"], "hello");
        assert!(Decoder::default().finish().is_err());
    }
    #[test]
    fn compressed_request_and_invalid_stream_fail_closed() {
        let body = vec![b'x'; 9000];
        let (f, gzip) = frame(&body).unwrap();
        assert!(gzip);
        let mut output = Vec::new();
        flate2::read::GzDecoder::new(&f[5..])
            .read_to_end(&mut output)
            .unwrap();
        assert_eq!(output, body);
        assert!(Decoder::default().feed(&[0, 255, 255, 255, 255]).is_err());
        assert!(Decoder::default().feed(&[2, 0, 0, 0, 1, b'x']).is_err());
    }
    #[test]
    fn tool_history_preserves_original_ids_and_repairs_missing_results() {
        let b = json!({"messages":[{"role":"system","content":"fixture"},{"role":"assistant","tool_calls":[{"id":tool_id_out("run:0#id"),"function":{"name":"run","arguments":"{}"}}]},{"role":"user","content":"continue"}],"tools":[]});
        let bytes = request("devin-session-token$fixture", "swe-2-high", &b).unwrap();
        let fields = wire::fields(&bytes).unwrap();
        let messages: Vec<_> = fields
            .iter()
            .filter_map(|f| {
                if let Field::Bytes(3, b) = f {
                    Some(wire::fields(b).unwrap())
                } else {
                    None
                }
            })
            .collect();
        assert!(messages
            .iter()
            .any(|m| wire::string(m, 7).as_deref() == Some("run:0#id")));
        assert_eq!(wire::string(&fields, 21).as_deref(), Some("swe-2-high"));
    }
}
