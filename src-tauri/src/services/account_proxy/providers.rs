use std::collections::BTreeSet;

use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::models::model::AccountProvider;

#[cfg(test)]
use super::json_response;
use super::protocol::{self, ChatDecoder, SseParser, WireProtocol};
use super::{stream_response, ProxyError};

const CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex";
const CODE_ASSIST_BASE: &str = "https://daily-cloudcode-pa.googleapis.com";

fn access_token(credentials: &Value) -> Result<&str, ProxyError> {
    credentials["accessToken"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProxyError::new(StatusCode::UNAUTHORIZED, "Account has no access token"))
}

fn base(provider: AccountProvider, credentials: &Value) -> &str {
    credentials["baseUrl"]
        .as_str()
        .unwrap_or(match provider {
            AccountProvider::Codex => CODEX_BASE,
            AccountProvider::WorkBuddy => "https://copilot.tencent.com",
            AccountProvider::WorkBuddyAi => "https://www.workbuddy.ai",
            AccountProvider::Antigravity => CODE_ASSIST_BASE,
            AccountProvider::Copilot => "https://api.githubcopilot.com",
            AccountProvider::Kilo => "https://api.kilo.ai/api/openrouter",
            AccountProvider::ClaudeCode => unreachable!("Claude uses its CLI"),
        })
        .trim_end_matches('/')
}

fn signed(
    request: reqwest::RequestBuilder,
    provider: AccountProvider,
    credentials: &Value,
) -> Result<reqwest::RequestBuilder, ProxyError> {
    let mut request = request.bearer_auth(access_token(credentials)?);
    match provider {
        AccountProvider::Codex => {
            request = request
                .header("originator", "codex_cli_rs")
                .header("version", "0.159.0")
                .header("user-agent", "codex_cli_rs/0.159.0 (EchoBird)")
                .header("OpenAI-Beta", "responses=experimental");
            if let Some(account) = credentials["accountId"].as_str() {
                request = request.header("chatgpt-account-id", account);
            }
        }
        AccountProvider::WorkBuddy | AccountProvider::WorkBuddyAi => {
            let uid = credentials["uid"].as_str().ok_or_else(|| {
                ProxyError::new(StatusCode::UNAUTHORIZED, "WorkBuddy account has no user id")
            })?;
            request = request
                .header("X-User-Id", uid)
                .header("X-Product", "SaaS")
                .header("X-IDE-Type", "WorkBuddy")
                .header("user-agent", "WorkBuddy/5.5.6")
                .header("X-Requested-With", "XMLHttpRequest");
            if let Some(domain) = credentials["domain"].as_str() {
                request = request.header("X-Domain", domain);
            }
            if let Some(enterprise) = credentials["enterpriseId"].as_str() {
                request = request.header("X-Enterprise-Id", enterprise);
            }
            if provider == AccountProvider::WorkBuddyAi {
                request = request
                    .header("X-Requested-With", "XMLHttpRequest")
                    .header("X-Agent-Intent", "craft")
                    .header("X-Agent-Type", "main")
                    .header("X-IDE-Name", "WorkBuddy")
                    .header("X-IDE-Version", "5.5.6");
                let conversation = uuid::Uuid::new_v4().to_string();
                request = request
                    .header("X-Conversation-ID", &conversation)
                    .header("X-Conversation-Request-ID", &conversation);
                let message = uuid::Uuid::new_v4().to_string();
                request = request
                    .header("X-Conversation-Message-ID", &message)
                    .header("X-Request-ID", &message);
            }
        }
        AccountProvider::Antigravity => {
            request = request
                .header(
                    "user-agent",
                    format!(
                        "antigravity/hub/3.1.4 {} /{}",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ),
                )
                .header("X-Goog-Api-Client", "gl-node/22.21.1");
        }
        AccountProvider::Copilot => {
            request = request
                .header("Copilot-Integration-Id", "copilot-developer-cli")
                .header("X-GitHub-Api-Version", "2026-07-01")
                .header("Editor-Version", "copilot/0.0.1")
                .header("X-Initiator", "user")
                .header("X-Request-Id", uuid::Uuid::new_v4().to_string());
        }
        AccountProvider::Kilo => {
            request = request
                .header("HTTP-Referer", "https://kilo.ai")
                .header("X-Title", "EchoBird");
        }
        AccountProvider::ClaudeCode => unreachable!("Claude uses its CLI"),
    }
    if let Some(headers) = credentials["headers"].as_object() {
        let mut overrides = reqwest::header::HeaderMap::new();
        for (name, value) in headers {
            if let Some(value) = value.as_str() {
                let name =
                    reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                        ProxyError::new(StatusCode::BAD_REQUEST, "Invalid account request header")
                    })?;
                let value = reqwest::header::HeaderValue::from_str(value).map_err(|_| {
                    ProxyError::new(StatusCode::BAD_REQUEST, "Invalid account request header")
                })?;
                overrides.insert(name, value);
            }
        }
        request = request.headers(overrides);
    }
    Ok(request)
}

async fn json_upstream(request: reqwest::RequestBuilder) -> Result<Value, ProxyError> {
    let response = request.send().await.map_err(ProxyError::upstream)?;
    let status = response.status();
    let headers = response.headers().clone();
    let value = response.json::<Value>().await;
    if !status.is_success() {
        return Err(upstream_error(
            status,
            &headers,
            &value.unwrap_or(Value::Null),
        ));
    }
    value.map_err(ProxyError::upstream)
}

fn upstream_error(status: StatusCode, headers: &HeaderMap, value: &Value) -> ProxyError {
    let mut error = ProxyError::new(status, safe_error(value));
    let detail = value
        .get("error")
        .filter(|error| error.is_object())
        .unwrap_or(value);
    error.error_code = detail["code"]
        .as_str()
        .or_else(|| detail["status"].as_str())
        .map(str::to_string);
    error.error_type = detail["type"].as_str().map(str::to_string);
    error.model_scope = detail["scope"] == "model"
        || matches!(
            error.error_code.as_deref(),
            Some("model_not_found" | "model_not_supported" | "tools_not_supported")
        );
    error.retry_after = headers.get(header::RETRY_AFTER).cloned();
    if error.retry_after.is_none() && status == StatusCode::TOO_MANY_REQUESTS {
        if detail["type"] == "usage_limit_reached" {
            let seconds = detail["resets_in_seconds"].as_u64().or_else(|| {
                let reset = detail["resets_at"].as_u64()?;
                let now = chrono::Utc::now().timestamp().max(0) as u64;
                Some(reset.saturating_sub(now))
            });
            error.retry_after =
                seconds.and_then(|seconds| HeaderValue::from_str(&seconds.to_string()).ok());
        }
        if error.retry_after.is_none() {
            let seconds = detail["details"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|hint| {
                    if hint["@type"] != "type.googleapis.com/google.rpc.RetryInfo" {
                        return None;
                    }
                    let delay = hint["retryDelay"]
                        .as_str()?
                        .strip_suffix('s')?
                        .parse::<f64>()
                        .ok()?;
                    (delay.is_finite() && delay >= 0.0).then_some(delay.ceil() as u64)
                });
            error.retry_after =
                seconds.and_then(|seconds| HeaderValue::from_str(&seconds.to_string()).ok());
        }
    }
    error
}

pub(crate) fn stream_error(event: &str, data: &str, headers: &HeaderMap) -> Option<ProxyError> {
    let value: Value = serde_json::from_str(data).ok()?;
    let detail = if event == "response.failed" || value["type"] == "response.failed" {
        value
            .pointer("/response/error")
            .filter(|error| !error.is_null())
            .or_else(|| value.get("error").filter(|error| !error.is_null()))
            .unwrap_or(&value)
    } else if (matches!(event, "response.completed" | "response.incomplete")
        || matches!(
            value["type"].as_str(),
            Some("response.completed" | "response.incomplete")
        ))
        && value
            .pointer("/response/error")
            .is_some_and(|error| !error.is_null())
    {
        &value["response"]["error"]
    } else if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        error
    } else if event == "error" || value["type"] == "error" {
        &value
    } else {
        return None;
    };
    let code = detail["code"]
        .as_str()
        .or_else(|| detail["status"].as_str())
        .or_else(|| detail["type"].as_str());
    let status = detail["status"]
        .as_u64()
        .and_then(|status| u16::try_from(status).ok())
        .and_then(|status| StatusCode::from_u16(status).ok())
        .filter(|status| status.is_client_error() || status.is_server_error())
        .unwrap_or(match code {
            Some(
                "rate_limit_exceeded"
                | "rate_limit_error"
                | "usage_limit_reached"
                | "RESOURCE_EXHAUSTED",
            ) => StatusCode::TOO_MANY_REQUESTS,
            Some("insufficient_quota" | "billing_error" | "billing_hard_limit_reached") => {
                StatusCode::PAYMENT_REQUIRED
            }
            Some("invalid_api_key" | "authentication_error" | "unauthorized" | "invalid_grant") => {
                StatusCode::UNAUTHORIZED
            }
            Some(
                "model_not_found"
                | "model_not_supported"
                | "tools_not_supported"
                | "response_not_found",
            ) => StatusCode::NOT_FOUND,
            Some("invalid_previous_response_id") => StatusCode::BAD_REQUEST,
            Some("server_error" | "overloaded_error" | "service_unavailable_error") => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            _ => StatusCode::BAD_GATEWAY,
        });
    let mut error = upstream_error(status, headers, &json!({"error":detail}));
    if error.retry_after.is_none() {
        error.retry_after = detail["retry_after"]
            .as_str()
            .and_then(|value| HeaderValue::from_str(value).ok());
    }
    Some(error)
}

fn safe_error(value: &Value) -> String {
    value["error"]["message"]
        .as_str()
        .or_else(|| value["message"].as_str())
        .or_else(|| value["msg"].as_str())
        .unwrap_or("Account upstream refused the request")
        .chars()
        .take(800)
        .collect()
}

async fn project(
    credentials: &Value,
    client: &reqwest::Client,
) -> Result<(String, Option<String>), ProxyError> {
    let load_base = credentials["loadBaseUrl"].as_str().unwrap_or_else(|| {
        if credentials["baseUrl"].is_string() {
            base(AccountProvider::Antigravity, credentials)
        } else {
            "https://cloudcode-pa.googleapis.com"
        }
    });
    let request = client
        .post(format!(
            "{}/v1internal:loadCodeAssist",
            load_base.trim_end_matches('/')
        ))
        .json(&json!({"metadata":{"ideType":"ANTIGRAVITY"}}));
    let value = json_upstream(signed(request, AccountProvider::Antigravity, credentials)?).await?;
    let id = credentials["projectId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .or_else(|| value["cloudaicompanionProject"].as_str())
        .or_else(|| value["cloudaicompanionProject"]["id"].as_str());
    let Some(id) = id else {
        let reason = value["ineligibleTiers"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|v| {
                v["reasonMessage"]
                    .as_str()
                    .or_else(|| v["validationErrorMessage"].as_str())
            })
            .unwrap_or("Antigravity has not set up this account; sign in to its app first");
        return Err(ProxyError::new(StatusCode::FORBIDDEN, reason));
    };
    let tier = value["paidTier"]["id"]
        .as_str()
        .filter(|tier| !matches!(*tier, "" | "free-tier" | "legacy-tier"))
        .or_else(|| value["currentTier"]["id"].as_str())
        .filter(|tier| !matches!(*tier, "" | "free-tier" | "legacy-tier"))
        .map(str::to_string);
    Ok((id.into(), tier))
}

async fn copilot_catalog(
    credentials: &Value,
    client: &reqwest::Client,
) -> Result<Value, ProxyError> {
    json_upstream(signed(
        client.get(format!(
            "{}/models",
            base(AccountProvider::Copilot, credentials)
        )),
        AccountProvider::Copilot,
        credentials,
    )?)
    .await
}

pub(super) async fn list_models(
    provider: AccountProvider,
    credentials: Value,
    client: &reqwest::Client,
) -> Result<Vec<String>, ProxyError> {
    let api = base(provider, &credentials);
    let value = match provider {
        AccountProvider::Antigravity => {
            let (id, tier) = project(&credentials, client).await?;
            let mut body = json!({"project":id});
            if let Some(tier) = tier {
                body["entitlement"] = json!({"userTier":tier});
            }
            json_upstream(signed(
                client
                    .post(format!("{api}/v1internal:fetchAvailableModels"))
                    .json(&body),
                provider,
                &credentials,
            )?)
            .await?
        }
        AccountProvider::WorkBuddy | AccountProvider::WorkBuddyAi => {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::USER_AGENT,
                reqwest::header::HeaderValue::from_static("CLI/2.0.0 WorkBuddy/5.5.6"),
            );
            headers.insert(
                "x-requested-with",
                reqwest::header::HeaderValue::from_static("XMLHttpRequest"),
            );
            headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
            let value = json_upstream(
                signed(
                    client.get(format!("{api}/v3/config")),
                    provider,
                    &credentials,
                )?
                .headers(headers),
            )
            .await?;
            if value["code"].as_i64().is_some_and(|c| c != 0) {
                return Err(ProxyError::new(StatusCode::BAD_GATEWAY, safe_error(&value)));
            }
            let models: Vec<String> = value["data"]["agents"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|a| a["name"] == "cli")
                .flat_map(|a| a["models"].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            return nonempty(models);
        }
        AccountProvider::Copilot => copilot_catalog(&credentials, client).await?,
        AccountProvider::Codex => {
            json_upstream(signed(
                client.get(format!("{api}/models?client_version=0.159.0")),
                provider,
                &credentials,
            )?)
            .await?
        }
        AccountProvider::Kilo => {
            json_upstream(signed(
                client.get(format!("{api}/models")),
                provider,
                &credentials,
            )?)
            .await?
        }
        AccountProvider::ClaudeCode => unreachable!("Claude uses its CLI"),
    };
    let ids = if provider == AccountProvider::Antigravity {
        value["models"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, m)| {
                !m["disabled"].as_bool().unwrap_or(false)
                    && !id.starts_with("chat_")
                    && !id.starts_with("tab_")
                    && !id.contains("image")
            })
            .map(|(id, _)| id.clone())
            .collect()
    } else {
        let rows = value["data"]
            .as_array()
            .or_else(|| value["models"].as_array());
        rows.into_iter()
            .flatten()
            .filter(|row| {
                row["policy"]["state"] != "disabled"
                    && (provider == AccountProvider::Copilot
                        || row["model_picker_enabled"] != false)
                    && row["visibility"] != "hide"
                    && row["supported_in_api"] != false
                    && (provider != AccountProvider::Copilot || copilot_endpoint(row).is_some())
            })
            .filter_map(|row| row["id"].as_str().or_else(|| row["slug"].as_str()))
            .map(str::to_string)
            .collect()
    };
    nonempty(ids)
}

fn nonempty(ids: Vec<String>) -> Result<Vec<String>, ProxyError> {
    let ids: Vec<_> = ids
        .into_iter()
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if ids.is_empty() {
        Err(ProxyError::new(
            StatusCode::BAD_GATEWAY,
            "The account API returned no available chat models",
        ))
    } else {
        Ok(ids)
    }
}

fn copilot_endpoint(row: &Value) -> Option<(WireProtocol, &'static str)> {
    if row.get("supported_endpoints").is_none() && row["capabilities"]["type"] == "chat" {
        return Some((WireProtocol::Chat, "/chat/completions"));
    }
    let endpoints = row["supported_endpoints"].as_array()?;
    for (path, protocol) in [
        ("/chat/completions", WireProtocol::Chat),
        ("/responses", WireProtocol::Responses),
        ("/v1/messages", WireProtocol::Anthropic),
        ("/messages", WireProtocol::Anthropic),
    ] {
        if endpoints.iter().any(|endpoint| endpoint == path) {
            return Some((protocol, path));
        }
    }
    None
}

async fn wire(
    provider: AccountProvider,
    credentials: &Value,
    model: &str,
    client: &reqwest::Client,
) -> Result<(WireProtocol, &'static str), ProxyError> {
    if provider == AccountProvider::Codex {
        return Ok((WireProtocol::Responses, "/responses"));
    }
    if provider == AccountProvider::Antigravity {
        return Ok((
            WireProtocol::CodeAssist,
            "/v1internal:streamGenerateContent?alt=sse",
        ));
    }
    if provider != AccountProvider::Copilot {
        return Ok((
            WireProtocol::Chat,
            if matches!(
                provider,
                AccountProvider::WorkBuddy | AccountProvider::WorkBuddyAi
            ) {
                "/v2/chat/completions"
            } else {
                "/chat/completions"
            },
        ));
    }
    let configured = credentials["modelProtocols"][model]
        .as_str()
        .or_else(|| credentials["protocol"].as_str());
    if let Some(protocol) = configured {
        return Ok(match protocol {
            "responses" => (WireProtocol::Responses, "/responses"),
            "messages" => (WireProtocol::Anthropic, "/v1/messages"),
            _ => (WireProtocol::Chat, "/chat/completions"),
        });
    }
    let catalog = copilot_catalog(credentials, client).await?;
    let row = catalog["data"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["id"] == model)
        .filter(|row| {
            row["policy"]["state"] != "disabled"
                && row["visibility"] != "hide"
                && row["supported_in_api"] != false
        })
        .ok_or_else(|| {
            ProxyError::new(
                StatusCode::NOT_FOUND,
                "Copilot does not offer this model to the account",
            )
        })?;
    copilot_endpoint(row).ok_or_else(|| {
        ProxyError::new(
            StatusCode::BAD_REQUEST,
            "Copilot model has no supported inference endpoint",
        )
    })
}

fn anthropic_request(body: &Value, model: &str) -> Value {
    let mut messages = Vec::new();
    let mut system = Vec::new();
    for message in body["messages"].as_array().into_iter().flatten() {
        let role = message["role"].as_str().unwrap_or("user");
        if matches!(role, "system" | "developer") {
            system.push(protocol::text(&message["content"]));
            continue;
        }
        let content = if role == "tool" {
            json!([{"type":"tool_result","tool_use_id":message["tool_call_id"],"content":message["content"]}])
        } else {
            let value = crate::services::local_llm::proxy::openai_to_anthropic(
                &json!({"choices":[{"message":message}]}),
            );
            let mut blocks = value["content"].as_array().cloned().unwrap_or_default();
            if role != "assistant" && message["content"].is_array() {
                blocks=message["content"].as_array().into_iter().flatten().filter_map(|part| {
                    if part["type"]=="text" { Some(json!({"type":"text","text":part["text"]})) }
                    else if part["type"]=="image_url" { let url=part["image_url"]["url"].as_str()?;
                        Some(if let Some(data)=url.strip_prefix("data:") { let (mime,bytes)=data.split_once(";base64,")?;
                            json!({"type":"image","source":{"type":"base64","media_type":mime,"data":bytes}})
                        } else { json!({"type":"image","source":{"type":"url","url":url}}) })
                    } else { None }
                }).collect();
            }
            json!(blocks)
        };
        messages.push(
            json!({"role":if role=="assistant" { "assistant" } else { "user" },"content":content}),
        );
    }
    let tools:Vec<_>=body["tools"].as_array().into_iter().flatten().map(|tool| {
        let function=&tool["function"];
        let mut tool=json!({"name":function["name"],"input_schema":function.get("parameters").cloned().unwrap_or(json!({"type":"object","properties":{}}))});
        if let Some(description)=function.get("description").filter(|value| value.is_string()) { tool["description"]=description.clone(); }
        tool
    }).collect();
    let mut request = json!({"model":model,"system":system.join("\n\n"),"messages":messages,"tools":tools,
        "max_tokens":body["max_tokens"].as_u64().unwrap_or(8192),"stream":true});
    let mut choice = body.get("tool_choice").map(|choice| match choice.as_str() {
        Some("required") => json!({"type":"any"}),
        Some("none") => json!({"type":"none"}),
        _ if choice.is_object() => json!({"type":"tool","name":choice["function"]["name"]}),
        _ => json!({"type":"auto"}),
    });
    if body["parallel_tool_calls"] == false {
        let choice = choice.get_or_insert(json!({"type":"auto"}));
        if choice["type"] != "none" {
            choice["disable_parallel_tool_use"] = json!(true);
        }
    }
    if let Some(choice) = choice {
        request["tool_choice"] = choice;
    }
    request
}

fn workbuddy_tool_choice(body: &mut Value) -> Result<(), ProxyError> {
    let Some(choice) = body.get("tool_choice").filter(|choice| choice.is_object()) else {
        return Ok(());
    };
    let name = choice["function"]["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .filter(|_| choice["type"] == "function")
        .ok_or_else(|| ProxyError::new(StatusCode::BAD_REQUEST, "Invalid named tool choice"))?;
    let tools: Vec<_> = body["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tool| tool["type"] == "function" && tool["function"]["name"] == name)
        .cloned()
        .collect();
    if tools.is_empty() {
        return Err(ProxyError::new(
            StatusCode::BAD_REQUEST,
            "Named tool is not declared",
        ));
    }
    // This gateway decodes tool_choice as a string. A name string acts like auto;
    // required with only that declaration preserves the named-choice contract.
    body["tools"] = json!(tools);
    body["tool_choice"] = json!("required");
    Ok(())
}

pub(super) async fn route(
    provider: AccountProvider,
    credentials: Value,
    model: &str,
    mut body: Value,
    client: &reqwest::Client,
) -> Result<Response, ProxyError> {
    let streaming = body["stream"].as_bool().unwrap_or(false);
    let (protocol, path) = wire(provider, &credentials, model, client).await?;
    let api = base(provider, &credentials);
    let request = match protocol {
        WireProtocol::Responses => protocol::chat_to_responses(&body, model),
        WireProtocol::CodeAssist => {
            let (project, _) = project(&credentials, client).await?;
            protocol::chat_to_code_assist(&body, model, &project)
        }
        WireProtocol::Anthropic => anthropic_request(&body, model),
        WireProtocol::Chat => {
            super::strip_response_metadata(&mut body);
            body["model"] = json!(model);
            body["stream"] = json!(true);
            if matches!(
                provider,
                AccountProvider::WorkBuddy | AccountProvider::WorkBuddyAi
            ) {
                workbuddy_tool_choice(&mut body)?;
                if let Some(messages) = body["messages"].as_array_mut() {
                    if messages
                        .first()
                        .is_some_and(|message| message["role"] != "system")
                    {
                        messages.insert(
                            0,
                            json!({"role":"system","content":"You are a helpful assistant."}),
                        );
                    }
                }
            }
            body
        }
    };
    let request = signed(
        client
            .post(format!("{api}{path}"))
            .json(&request)
            .header("accept", "text/event-stream"),
        provider,
        &credentials,
    )?;
    let request = if protocol == WireProtocol::Anthropic {
        request.header("anthropic-version", "2023-06-01")
    } else {
        request
    };
    let response = request.send().await.map_err(ProxyError::upstream)?;
    let status = response.status();
    if !status.is_success() {
        let headers = response.headers().clone();
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        return Err(upstream_error(status, &headers, &value));
    }
    converted(response, protocol, model, streaming).await
}

pub(super) async fn route_responses(
    provider: AccountProvider,
    credentials: Value,
    model: &str,
    mut body: Value,
    client: &reqwest::Client,
) -> Result<Option<Response>, ProxyError> {
    if provider == AccountProvider::Copilot {
        let configured = credentials["modelProtocols"][model]
            .as_str()
            .or_else(|| credentials["protocol"].as_str());
        let supported = if configured == Some("responses") {
            true
        } else {
            let catalog = copilot_catalog(&credentials, client).await?;
            catalog["data"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|row| row["id"] == model)
                .is_some_and(|row| {
                    row["policy"]["state"] != "disabled"
                        && row["visibility"] != "hide"
                        && row["supported_in_api"] != false
                        && row["supported_endpoints"]
                            .as_array()
                            .is_some_and(|endpoints| {
                                endpoints.iter().any(|path| path == "/responses")
                            })
                })
        };
        if !supported {
            return Ok(None);
        }
    } else if provider != AccountProvider::Codex {
        return Ok(None);
    }
    let streaming = body["stream"].as_bool().unwrap_or(false);
    let request = body.as_object_mut().ok_or_else(|| {
        ProxyError::new(
            StatusCode::BAD_REQUEST,
            "Responses request must be an object",
        )
    })?;
    request.insert("model".into(), json!(model));
    if provider == AccountProvider::Codex {
        // This account backend requires streaming and does not store responses.
        request.insert("stream".into(), json!(true));
        request.insert("store".into(), json!(false));
        if !request.get("instructions").is_some_and(Value::is_string) {
            request.insert("instructions".into(), json!(""));
        }
        request
            .entry("include")
            .or_insert_with(|| json!(["reasoning.encrypted_content"]));
    }
    let response = signed(
        client
            .post(format!("{}/responses", base(provider, &credentials)))
            .json(&body)
            .header(
                "accept",
                if streaming || provider == AccountProvider::Codex {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            ),
        provider,
        &credentials,
    )?
    .send()
    .await
    .map_err(ProxyError::upstream)?;
    let status = response.status();
    let mut headers = response.headers().clone();
    if !status.is_success() {
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        return Err(upstream_error(status, &headers, &value));
    }
    if provider == AccountProvider::Codex && !headers.contains_key(header::CONTENT_TYPE) {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    if !content_type.is_some_and(|value| {
        value.contains("text/event-stream") || value.contains("application/json")
    }) {
        return Err(ProxyError::new(
            StatusCode::BAD_GATEWAY,
            "Account API did not return a Responses payload",
        ));
    }
    let mut result = Response::new(Body::from_stream(response.bytes_stream()));
    *result.status_mut() = status;
    *result.headers_mut() = headers;
    responses_response(result, streaming).map(Some)
}

pub fn responses_response(mut response: Response, streaming: bool) -> Result<Response, ProxyError> {
    let connection_headers: Vec<_> = response
        .headers()
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_string())
        .collect();
    for name in connection_headers {
        response.headers_mut().remove(name.as_str());
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "authorization",
        "set-cookie",
        "cookie",
        "content-length",
        "content-encoding",
    ] {
        response.headers_mut().remove(name);
    }
    if !response.status().is_success() {
        return Ok(response);
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let is_sse = content_type.is_some_and(|value| value.contains("text/event-stream"));
    if streaming {
        if !is_sse {
            return Err(ProxyError::new(
                StatusCode::BAD_GATEWAY,
                "Responses API did not return the requested event stream",
            ));
        }
        return Ok(response);
    }
    if content_type.is_some_and(|value| value.contains("application/json")) {
        return Ok(response);
    }
    if !is_sse {
        return Err(ProxyError::new(
            StatusCode::BAD_GATEWAY,
            "Responses API did not return an event stream",
        ));
    }
    let (mut parts, body) = response.into_parts();
    let headers = parts.headers.clone();
    let mut upstream = body.into_data_stream();
    let (tx, rx) = mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(1);
    tokio::spawn(async move {
        let mut parser = SseParser::default();
        loop {
            let next = tokio::select! { _=tx.closed()=>return, next=upstream.next()=>next };
            let eof = next.is_none();
            let frames = match next {
                Some(Ok(bytes)) => parser.push(&bytes),
                Some(Err(error)) => {
                    let _ = tx.send(Err(std::io::Error::other(error))).await;
                    return;
                }
                None => parser.finish(),
            };
            for (event, data) in frames {
                if let Some(error) = stream_error(&event, &data, &headers) {
                    let _ = tx.send(Err(std::io::Error::other(error))).await;
                    return;
                }
                let value = serde_json::from_str::<Value>(&data);
                let value = match value {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = tx.send(Err(std::io::Error::other(error))).await;
                        return;
                    }
                };
                let kind = value["type"].as_str().unwrap_or(&event);
                if matches!(kind, "response.completed" | "response.incomplete") {
                    let terminal = &value["response"];
                    if !terminal["error"].is_null() {
                        let error = stream_error("response.failed", &data, &headers)
                            .expect("A non-null terminal error is an explicit failure");
                        let _ = tx.send(Err(std::io::Error::other(error))).await;
                        return;
                    }
                    let expected_status = if kind == "response.completed" {
                        "completed"
                    } else {
                        "incomplete"
                    };
                    if terminal["object"] == "response"
                        && terminal["status"] == expected_status
                        && terminal["id"].as_str().is_some_and(|id| !id.is_empty())
                        && (event.is_empty() || event == kind)
                    {
                        let _ = tx.send(Ok(bytes::Bytes::from(terminal.to_string()))).await;
                    } else {
                        let _ = tx
                            .send(Err(std::io::Error::other(
                                "Responses terminal event has an invalid response object",
                            )))
                            .await;
                    }
                    return;
                }
            }
            if eof {
                let _ = tx
                    .send(Err(std::io::Error::other(
                        "Responses stream ended before completion",
                    )))
                    .await;
                return;
            }
        }
    });
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    parts.headers.remove(header::CONTENT_LENGTH);
    Ok(Response::from_parts(
        parts,
        Body::from_stream(ReceiverStream::new(rx)),
    ))
}

async fn converted(
    response: reqwest::Response,
    protocol: WireProtocol,
    model: &str,
    streaming: bool,
) -> Result<Response, ProxyError> {
    let response_headers = response.headers().clone();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok());
    // Codex's authenticated backend can omit Content-Type on a valid event stream.
    // The decoder still requires valid SSE events and a terminal response event.
    let is_sse = content_type.is_some_and(|v| v.contains("text/event-stream"));
    if !(is_sse || protocol == WireProtocol::Responses && content_type.is_none()) {
        return Err(ProxyError::new(
            StatusCode::BAD_GATEWAY,
            "Account API did not return the requested event stream",
        ));
    }
    let mut upstream = response.bytes_stream();
    let mut parser = SseParser::default();
    let mut decoder = ChatDecoder::new(protocol, model);
    // The bounded queue applies downstream backpressure. Closing the response cancels
    // the upstream request, including while it is waiting for its next frame.
    let (tx, rx) = mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(16);
    tokio::spawn(async move {
        loop {
            let next = tokio::select! { _=tx.closed()=>return, next=upstream.next()=>next };
            let eof = next.is_none();
            let frames = match next {
                Some(Ok(bytes)) => parser.push(&bytes),
                Some(Err(error)) => {
                    let _ = tx.send(Err(std::io::Error::other(error))).await;
                    return;
                }
                None => parser.finish(),
            };
            for (event, data) in frames {
                match decoder.decode(&event, &data) {
                    Ok(chunks) => {
                        if streaming {
                            for chunk in chunks {
                                if tx.send(Ok(chunk)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let error = stream_error(&event, &data, &response_headers)
                            .map(std::io::Error::other)
                            .unwrap_or_else(|| std::io::Error::other(error));
                        let _ = tx.send(Err(error)).await;
                        return;
                    }
                }
            }
            if decoder.finished {
                if !streaming {
                    let _ = tx
                        .send(Ok(bytes::Bytes::from(decoder.collected().to_string())))
                        .await;
                }
                return;
            }
            if eof {
                let _ = tx
                    .send(Err(std::io::Error::other(
                        "Account stream ended before completion",
                    )))
                    .await;
                return;
            }
        }
    });
    let body = Body::from_stream(ReceiverStream::new(rx));
    Ok(if streaming {
        stream_response(body)
    } else {
        Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .expect("fixed response headers")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Request, State},
        Router,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    type Requests = Arc<Mutex<Vec<(String, reqwest::header::HeaderMap, Value)>>>;
    async fn upstream(State(requests): State<Requests>, request: Request) -> Response {
        let path = request.uri().to_string();
        let headers = request.headers().clone();
        let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        requests
            .lock()
            .await
            .push((path.clone(), headers, body.clone()));
        if path == "/v1internal:loadCodeAssist" {
            return json_response(
                &json!({"cloudaicompanionProject":"test-project","currentTier":{"id":"standard-tier"}}),
            );
        }
        if path == "/free/v1internal:loadCodeAssist" {
            return json_response(
                &json!({"cloudaicompanionProject":"test-project","currentTier":{"id":"free-tier"},"paidTier":{"id":"free-tier"}}),
            );
        }
        if path == "/v1internal:fetchAvailableModels" {
            if body["entitlement"]["userTier"] == "free-tier" {
                let mut response = json_response(
                    &json!({"error":{"status":"FAILED_PRECONDITION","message":"Precondition check failed."}}),
                );
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return response;
            }
            return json_response(
                &json!({"models":{"gemini-test":{},"image-gen":{},"disabled":{"disabled":true}}}),
            );
        }
        if path == "/v3/config" {
            return json_response(
                &json!({"code":0,"data":{"agents":[{"name":"cli","models":["workbuddy-test"]}]}}),
            );
        }
        if path == "/responses" {
            if body["model"] == "quota-stream" {
                return stream_response(Body::from("event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"type\":\"usage_limit_reached\",\"code\":\"rate_limit_exceeded\",\"message\":\"Account rate limit\",\"scope\":\"model\",\"resets_in_seconds\":90}}}\n\n"));
            }
            if body["model"] == "headerless-invalid" {
                return Response::new(Body::from("{\"not\":\"an event stream\"}"));
            }
            if body["model"] == "done-only" {
                return stream_response(Body::from("event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\ndata: [DONE]\n\n"));
            }
            let incomplete =
                body["model"] == "terminal-incomplete" || body["model"] == "terminal-auth";
            let kind = if incomplete {
                "response.incomplete"
            } else {
                "response.completed"
            };
            let mut terminal = json!({"type":kind,"response":{"object":"response","id":"resp_fixture","status":if incomplete { "incomplete" } else { "completed" },"error":null,"usage":{"input_tokens":1,"output_tokens":1}}});
            if body["model"] == "fake-completed" {
                terminal["response"]["status"] = json!("in_progress");
            } else if body["model"] == "terminal-quota" {
                terminal["response"]["status"] = json!("failed");
                terminal["response"]["error"] = json!({"code":"rate_limit_exceeded","type":"usage_limit_reached","message":"Account limit","scope":"model","resets_in_seconds":90});
            } else if body["model"] == "terminal-auth" {
                terminal["response"]["status"] = json!("failed");
                terminal["response"]["error"] = json!({"code":"authentication_error","message":"Account authorization expired"});
            }
            let payload = Body::from(format!("event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}}\n\nevent: {kind}\ndata: {terminal}\n\n"));
            if body["model"] == "headerless" {
                return Response::new(payload);
            }
            return stream_response(payload);
        }
        if path.starts_with("/v1internal:streamGenerateContent") {
            return stream_response(Body::from("data: {\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}]},\"finishReason\":\"STOP\"}]}}\n\n"));
        }
        if path == "/v2/chat/completions" {
            return stream_response(Body::from("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"));
        }
        let mut response = json_response(
            &json!({"error":{"message":"account quota","code":"rate_limit_exceeded","type":"rate_limit_error","scope":"model"}}),
        );
        *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("120"));
        response
    }

    async fn fixture() -> (String, Requests, tokio::task::JoinHandle<()>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .fallback(upstream)
            .with_state(requests.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (base, requests, server)
    }

    #[tokio::test]
    async fn codex_and_workbuddy_send_identity_and_their_actual_endpoints() {
        let (base, requests, server) = fixture().await;
        let client = reqwest::Client::new();
        let body = json!({"stream":false,"messages":[{"role":"user","content":"hello"}]});
        let response = route(
            AccountProvider::Codex,
            json!({"accessToken":"fixture","accountId":"own-account","baseUrl":base}),
            "gpt-test",
            body.clone(),
            &client,
        )
        .await
        .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let data: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(data["choices"][0]["message"]["content"], "ok");
        route(AccountProvider::WorkBuddy,json!({"accessToken":"fixture","uid":"own-uid","domain":"fixture-domain","baseUrl":base}),"workbuddy-test",body,&client).await.unwrap();
        let recorded = requests.lock().await;
        assert_eq!(recorded[0].0, "/responses");
        assert_eq!(recorded[0].1["chatgpt-account-id"], "own-account");
        assert_eq!(recorded[0].2["store"], false);
        assert_eq!(recorded[1].0, "/v2/chat/completions");
        assert_eq!(recorded[1].1["x-user-id"], "own-uid");
        assert_eq!(recorded[1].1["x-domain"], "fixture-domain");
        assert_eq!(recorded[1].2["messages"][0]["role"], "system");
        assert_eq!(recorded[1].2["messages"][1]["content"], "hello");
        server.abort();
    }

    #[tokio::test]
    async fn workbuddy_named_tool_choice_keeps_the_forced_tool_contract() {
        let (base, requests, server) = fixture().await;
        let client = reqwest::Client::new();
        let selected = json!({"type":"function","function":{"name":"echo_probe","parameters":{"type":"object"},"strict":true}});
        let original = json!({"messages":[{"role":"user","content":"Call another tool."}],"tools":[selected, {"type":"function","function":{"name":"another_probe"}}],"tool_choice":{"type":"function","function":{"name":"echo_probe"}}});
        for provider in [AccountProvider::WorkBuddy, AccountProvider::WorkBuddyAi] {
            route(
                provider,
                json!({"accessToken":"fixture","uid":"fixture","baseUrl":base}),
                "test",
                original.clone(),
                &client,
            )
            .await
            .unwrap();
        }
        assert!(route(
            AccountProvider::Kilo,
            json!({"accessToken":"fixture","baseUrl":base}),
            "test",
            original.clone(),
            &client
        )
        .await
        .is_err());
        let recorded = requests.lock().await;
        for request in &recorded[..2] {
            assert_eq!(request.2["tool_choice"], "required");
            assert_eq!(request.2["tools"], json!([selected]));
            assert_eq!(request.2["messages"][1], original["messages"][0]);
        }
        assert_eq!(recorded[2].2["tool_choice"], original["tool_choice"]);
        assert_eq!(recorded[2].2["tools"], original["tools"]);
        server.abort();
    }

    #[tokio::test]
    async fn workbuddy_string_choices_keep_tools_and_continuation_history() {
        let (base, requests, server) = fixture().await;
        let client = reqwest::Client::new();
        let history = json!([
            {"role":"system","content":"Original system"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call_a","type":"function","function":{"name":"echo_probe","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call_a","content":"probe-ok"}
        ]);
        let tools = json!([{"type":"function","function":{"name":"echo_probe"}},{"type":"function","function":{"name":"another_probe"}}]);
        for choice in ["auto", "none", "required"] {
            route(
                AccountProvider::WorkBuddy,
                json!({"accessToken":"fixture","uid":"fixture","baseUrl":base}),
                "test",
                json!({"messages":history,"tools":tools,"tool_choice":choice}),
                &client,
            )
            .await
            .unwrap();
        }
        for (request, choice) in requests
            .lock()
            .await
            .iter()
            .zip(["auto", "none", "required"])
        {
            assert_eq!(request.2["tool_choice"], choice);
            assert_eq!(request.2["tools"], tools);
            assert_eq!(request.2["messages"], history);
        }
        server.abort();
    }

    #[tokio::test]
    async fn workbuddy_invalid_named_choices_do_not_contact_upstream() {
        let (base, requests, server) = fixture().await;
        let client = reqwest::Client::new();
        for choice in [
            json!({"type":"function","function":{"name":"missing"}}),
            json!({"type":"function"}),
            json!({"type":"custom","name":"echo_probe"}),
        ] {
            let error = route(AccountProvider::WorkBuddy, json!({"accessToken":"fixture","uid":"fixture","baseUrl":base}), "test", json!({"messages":[],"tools":[{"type":"function","function":{"name":"echo_probe"}}],"tool_choice":choice}), &client).await.unwrap_err();
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
        }
        assert!(requests.lock().await.is_empty());
        server.abort();
    }

    #[tokio::test]
    async fn workbuddy_ai_catalog_and_chat_keep_required_headers_and_original_prompt() {
        let (base, requests, server) = fixture().await;
        let client = reqwest::Client::new();
        let credentials = json!({"accessToken":"fixture","uid":"own-uid","baseUrl":base});
        list_models(AccountProvider::WorkBuddyAi, credentials.clone(), &client)
            .await
            .unwrap();
        route(AccountProvider::WorkBuddyAi,credentials,"test",json!({"stream":false,"messages":[{"role":"system","content":"Original client identity"},{"role":"user","content":"hello"}],"_echobird_responses":{"input":"hello"}}),&client).await.unwrap();
        let recorded = requests.lock().await;
        assert_eq!(recorded[0].0, "/v3/config");
        assert_eq!(recorded[0].1["user-agent"], "CLI/2.0.0 WorkBuddy/5.5.6");
        assert_eq!(recorded[0].1["x-requested-with"], "XMLHttpRequest");
        assert_eq!(recorded[0].1["accept"], "application/json");
        assert_eq!(recorded[1].1["x-requested-with"], "XMLHttpRequest");
        assert_eq!(recorded[1].1["x-agent-intent"], "craft");
        assert_eq!(
            recorded[1].2["messages"][0]["content"],
            "Original client identity"
        );
        assert!(recorded[1]
            .2
            .get(super::super::tools::RESPONSES_REQUEST)
            .is_none());
        server.abort();
    }

    #[tokio::test]
    async fn codex_forwards_custom_schema_and_history_without_router_metadata() {
        let (base, requests, server) = fixture().await;
        let original = json!({"model":"auto","input":[{"type":"custom_tool_call","call_id":"call_a","name":"apply_patch","input":"*** Begin Patch\n*** End Patch"},{"type":"custom_tool_call_output","call_id":"call_a","output":"ok"}],"tools":[{"type":"custom","name":"apply_patch","format":{"type":"grammar","syntax":"lark","definition":"start: SOURCE"}}]});
        let chat = super::super::responses_to_chat(&original).unwrap();
        route(
            AccountProvider::Codex,
            json!({"accessToken":"fixture","baseUrl":base}),
            "gpt-test",
            chat,
            &reqwest::Client::new(),
        )
        .await
        .unwrap();
        let recorded = requests.lock().await;
        assert_eq!(recorded[0].0, "/responses");
        assert_eq!(recorded[0].2["input"], original["input"]);
        assert_eq!(recorded[0].2["tools"], original["tools"]);
        assert!(recorded[0]
            .2
            .get(super::super::tools::RESPONSES_REQUEST)
            .is_none());
        server.abort();
    }

    #[tokio::test]
    async fn responses_without_content_type_still_require_real_events_and_completion() {
        let (base, _, server) = fixture().await;
        let client = reqwest::Client::new();
        let credentials = json!({"accessToken":"fixture","baseUrl":base});
        for streaming in [false, true] {
            let response = route(
                AccountProvider::Codex,
                credentials.clone(),
                "headerless",
                json!({"stream":streaming,"messages":[{"role":"user","content":"hello"}]}),
                &client,
            )
            .await
            .unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            if streaming {
                let text = String::from_utf8(bytes.to_vec()).unwrap();
                assert!(text.contains("data: [DONE]"));
                assert!(text.contains("\"content\":\"ok\""));
            } else {
                let response: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(response["choices"][0]["message"]["content"], "ok");
            }
        }
        let response = route(
            AccountProvider::Codex,
            credentials,
            "headerless-invalid",
            json!({"messages":[{"role":"user","content":"hello"}]}),
            &client,
        )
        .await
        .unwrap();
        assert!(axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .is_err());
        server.abort();
    }

    #[tokio::test]
    async fn antigravity_resolves_project_and_catalog_without_quota_or_onboarding() {
        let (base, requests, server) = fixture().await;
        let client = reqwest::Client::new();
        let credentials = json!({"accessToken":"fixture","baseUrl":base});
        assert_eq!(
            list_models(AccountProvider::Antigravity, credentials.clone(), &client)
                .await
                .unwrap(),
            vec!["gemini-test"]
        );
        route(
            AccountProvider::Antigravity,
            credentials,
            "gemini-test",
            json!({"stream":false,"messages":[{"role":"user","content":"hello"}]}),
            &client,
        )
        .await
        .unwrap();
        let recorded = requests.lock().await;
        assert_eq!(
            recorded.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(),
            vec![
                "/v1internal:loadCodeAssist",
                "/v1internal:fetchAvailableModels",
                "/v1internal:loadCodeAssist",
                "/v1internal:streamGenerateContent?alt=sse"
            ]
        );
        assert_eq!(recorded[3].2["project"], "test-project");
        server.abort();
    }

    #[tokio::test]
    async fn antigravity_free_account_omits_the_inapplicable_entitlement() {
        let (base, requests, server) = fixture().await;
        let credentials =
            json!({"accessToken":"fixture","baseUrl":base,"loadBaseUrl":format!("{base}/free")});
        let models = list_models(
            AccountProvider::Antigravity,
            credentials,
            &reqwest::Client::new(),
        )
        .await
        .unwrap();
        assert_eq!(models, vec!["gemini-test"]);
        let recorded = requests.lock().await;
        assert_eq!(
            recorded
                .iter()
                .map(|request| request.0.as_str())
                .collect::<Vec<_>>(),
            vec![
                "/free/v1internal:loadCodeAssist",
                "/v1internal:fetchAvailableModels"
            ]
        );
        assert!(recorded[1].2.get("entitlement").is_none());
        assert_eq!(recorded[1].2["project"], "test-project");
        server.abort();
    }

    #[tokio::test]
    async fn quota_status_is_preserved_for_router_fallback() {
        let (base, _, server) = fixture().await;
        let error = route(
            AccountProvider::Kilo,
            json!({"accessToken":"fixture","baseUrl":base}),
            "test",
            json!({"messages":[]}),
            &reqwest::Client::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.message, "account quota");
        assert_eq!(error.error_code.as_deref(), Some("rate_limit_exceeded"));
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "120");
        assert!(
            response
                .extensions()
                .get::<super::super::ProxyFailureMetadata>()
                .unwrap()
                .model_scope
        );
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["code"], "rate_limit_exceeded");
        assert_eq!(value["error"]["type"], "rate_limit_error");
        server.abort();
    }

    #[test]
    fn quota_reset_hint_is_preserved_without_inventing_an_error_code() {
        let value = json!({"error":{"type":"usage_limit_reached","message":"Wait for reset","resets_in_seconds":90}});
        let error = upstream_error(StatusCode::TOO_MANY_REQUESTS, &HeaderMap::new(), &value);
        assert_eq!(error.retry_after.unwrap(), "90");
        assert!(error.error_code.is_none());
        assert_eq!(error.error_type.as_deref(), Some("usage_limit_reached"));
        assert!(!error.model_scope);
        let error = upstream_error(
            StatusCode::TOO_MANY_REQUESTS,
            &HeaderMap::new(),
            &json!({"error":{"status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"3.25s"}]}}),
        );
        assert_eq!(error.retry_after.unwrap(), "4");
        assert_eq!(error.error_code.as_deref(), Some("RESOURCE_EXHAUSTED"));
    }

    #[tokio::test]
    async fn structured_stream_failure_remains_in_the_body_error_chain() {
        use std::error::Error;
        let (base, _, server) = fixture().await;
        for streaming in [false, true] {
            let response = route(
                AccountProvider::Codex,
                json!({"accessToken":"fixture","baseUrl":base}),
                "quota-stream",
                json!({"stream":streaming,"messages":[]}),
                &reqwest::Client::new(),
            )
            .await
            .unwrap();
            let error = response
                .into_body()
                .into_data_stream()
                .next()
                .await
                .unwrap()
                .unwrap_err();
            let mut cause: &(dyn Error + 'static) = &error;
            let metadata = loop {
                if let Some(metadata) = cause.downcast_ref::<ProxyError>() {
                    break metadata;
                }
                cause = cause
                    .downcast_ref::<std::io::Error>()
                    .and_then(|error| error.get_ref())
                    .map(|error| error as &(dyn Error + 'static))
                    .or_else(|| cause.source())
                    .expect("structured error remains attached to the body");
            };
            assert_eq!(metadata.status, StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(metadata.retry_after.as_ref().unwrap(), "90");
            assert_eq!(metadata.error_code.as_deref(), Some("rate_limit_exceeded"));
            assert!(metadata.model_scope);
        }
        server.abort();
    }

    #[tokio::test]
    async fn chat_entry_rejects_responses_sentinel_and_invalid_terminal() {
        let (base, requests, server) = fixture().await;
        for provider in [AccountProvider::Codex, AccountProvider::Copilot] {
            for streaming in [false, true] {
                for model in ["done-only", "fake-completed"] {
                    let response = route(
                        provider,
                        json!({"accessToken":"fixture","baseUrl":base,"protocol":"responses"}),
                        model,
                        json!({"stream":streaming,"messages":[]}),
                        &reqwest::Client::new(),
                    )
                    .await
                    .unwrap();
                    let mut body = response.into_body().into_data_stream();
                    let mut returned = Vec::new();
                    let mut failure = None;
                    while let Some(chunk) = body.next().await {
                        match chunk {
                            Ok(bytes) => returned.extend_from_slice(&bytes),
                            Err(error) => {
                                failure = Some(error);
                                break;
                            }
                        }
                    }
                    assert!(
                        failure.is_some(),
                        "{provider:?} {model} incorrectly completed"
                    );
                    let text = String::from_utf8(returned).unwrap();
                    assert!(!text.contains("[DONE]"));
                    if streaming {
                        assert!(
                            text.contains(if model == "done-only" {
                                "partial"
                            } else {
                                "ok"
                            }),
                            "positive control: upstream text must have reached the Chat stream"
                        );
                    }
                }
            }
        }
        assert!(requests
            .lock()
            .await
            .iter()
            .all(|request| request.0 == "/responses"));
        server.abort();
    }

    #[tokio::test]
    async fn chat_entry_responses_completed_and_incomplete_keep_success_and_length() {
        let (base, requests, server) = fixture().await;
        for provider in [AccountProvider::Codex, AccountProvider::Copilot] {
            for streaming in [false, true] {
                for (model, reason) in [
                    ("terminal-completed", "stop"),
                    ("terminal-incomplete", "length"),
                ] {
                    let response = route(
                        provider,
                        json!({"accessToken":"fixture","baseUrl":base,"protocol":"responses"}),
                        model,
                        json!({"stream":streaming,"messages":[]}),
                        &reqwest::Client::new(),
                    )
                    .await
                    .unwrap();
                    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                        .await
                        .unwrap();
                    if streaming {
                        let mut parser = SseParser::default();
                        let frames = parser.push(&bytes);
                        assert!(frames.iter().any(|(_, data)| data == "[DONE]"));
                        assert!(frames
                            .iter()
                            .filter_map(|(_, data)| serde_json::from_str::<Value>(data).ok())
                            .any(|value| value["choices"][0]["finish_reason"] == reason));
                    } else {
                        let value: Value = serde_json::from_slice(&bytes).unwrap();
                        assert_eq!(value["choices"][0]["message"]["content"], "ok");
                        assert_eq!(value["choices"][0]["finish_reason"], reason);
                    }
                }
            }
        }
        assert!(requests
            .lock()
            .await
            .iter()
            .all(|request| request.0 == "/responses"));
        server.abort();
    }

    #[tokio::test]
    async fn chat_entry_responses_nested_terminal_errors_keep_typed_account_failures() {
        use std::error::Error;
        let (base, _, server) = fixture().await;
        for provider in [AccountProvider::Codex, AccountProvider::Copilot] {
            for streaming in [false, true] {
                for (model, status, code, retry_after, model_scope) in [
                    (
                        "terminal-quota",
                        StatusCode::TOO_MANY_REQUESTS,
                        "rate_limit_exceeded",
                        Some("90"),
                        true,
                    ),
                    (
                        "terminal-auth",
                        StatusCode::UNAUTHORIZED,
                        "authentication_error",
                        None,
                        false,
                    ),
                ] {
                    let response = route(
                        provider,
                        json!({"accessToken":"fixture","baseUrl":base,"protocol":"responses"}),
                        model,
                        json!({"stream":streaming,"messages":[]}),
                        &reqwest::Client::new(),
                    )
                    .await
                    .unwrap();
                    let mut body = response.into_body().into_data_stream();
                    let mut returned = Vec::new();
                    let error = loop {
                        match body
                            .next()
                            .await
                            .expect("the invalid terminal must fail the body")
                        {
                            Ok(bytes) => returned.extend_from_slice(&bytes),
                            Err(error) => break error,
                        }
                    };
                    assert!(!String::from_utf8(returned).unwrap().contains("[DONE]"));
                    let mut cause: &(dyn Error + 'static) = &error;
                    let metadata = loop {
                        if let Some(metadata) = cause.downcast_ref::<ProxyError>() {
                            break metadata;
                        }
                        cause = cause
                            .downcast_ref::<std::io::Error>()
                            .and_then(|error| error.get_ref())
                            .map(|error| error as &(dyn Error + 'static))
                            .or_else(|| cause.source())
                            .expect("terminal error retains its typed metadata");
                    };
                    assert_eq!(metadata.status, status);
                    assert_eq!(metadata.error_code.as_deref(), Some(code));
                    assert_eq!(
                        metadata
                            .retry_after
                            .as_ref()
                            .and_then(|value| value.to_str().ok()),
                        retry_after
                    );
                    assert_eq!(metadata.model_scope, model_scope);
                }
            }
        }
        server.abort();
    }

    #[tokio::test]
    async fn nonstream_headers_return_before_generation_and_dropping_body_cancels_upstream() {
        use std::time::Duration;
        let canceled = Arc::new(tokio::sync::Notify::new());
        let observed = canceled.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(move || {
            let observed = observed.clone();
            async move {
                let (tx, rx) = mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(1);
                tokio::spawn(async move {
                    tx.closed().await;
                    observed.notify_one();
                });
                stream_response(Body::from_stream(ReceiverStream::new(rx)))
            }
        });
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            route(
                AccountProvider::Codex,
                json!({"accessToken":"fixture","baseUrl":base}),
                "test",
                json!({"stream":false,"messages":[]}),
                &reqwest::Client::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        drop(response);
        tokio::time::timeout(Duration::from_secs(5), canceled.notified())
            .await
            .unwrap();
        server.abort();
    }

    mod native_responses {
        use super::*;

        fn final_response(status: &str) -> Value {
            json!({"id":"resp_upstream","object":"response","created_at":42,"status":status,
                "output":[{"id":"reasoning_upstream","type":"reasoning","encrypted_content":"opaque"},
                    {"id":"message_upstream","type":"message","role":"assistant","content":[{"type":"output_text","text":"原始结果","annotations":[{"type":"url_citation","url":"https://example.invalid/","title":"source"}]}]}],
                "usage":{"input_tokens":3,"output_tokens":7,"output_tokens_details":{"reasoning_tokens":2}},
                "incomplete_details":if status == "incomplete" {json!({"reason":"content_filter"})} else {Value::Null},
                "future_metadata":{"kept":true}})
        }

        fn events(status: &str) -> String {
            format!(": upstream comment\r\nid: upstream-sequence\r\nevent: response.created\r\ndata: {{\"type\":\"response.created\",\"sequence_number\":0,\"response\":{{\"id\":\"resp_upstream\"}}}}\r\n\r\nevent: future.event\r\ndata: {{\"type\":\"future.event\",\"extension\":true}}\r\n\r\nevent: response.{status}\r\ndata: {{\"type\":\"response.{status}\",\"sequence_number\":9,\"response\":{}}}\r\n\r\n", final_response(status))
        }

        async fn upstream(State(requests): State<Requests>, request: Request) -> Response {
            let path = request.uri().to_string();
            let headers = request.headers().clone();
            let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            requests
                .lock()
                .await
                .push((path.clone(), headers, body.clone()));
            if path == "/models" {
                return json_response(&json!({"data":[
                    {"id":"both","supported_endpoints":["/chat/completions","/responses"]},
                    {"id":"chat-only","supported_endpoints":["/chat/completions"]},
                    {"id":"blocked","supported_endpoints":["/responses"],"policy":{"state":"disabled"}}
                ]}));
            }
            assert_eq!(path, "/responses");
            let mut response = match body["model"].as_str().unwrap_or_default() {
                "bad-content" => Response::builder().header(header::CONTENT_TYPE, "text/plain").body(Body::from("not a Responses body")).unwrap(),
                "headerless" => Response::new(Body::from(events("completed"))),
                "incomplete" => stream_response(Body::from(events("incomplete"))),
                "truncated" => stream_response(Body::from("event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"unfinished\"}\n\n")),
                "failed" => stream_response(Body::from("event: response.failed\ndata: {\"response\":{\"error\":{\"code\":\"rate_limit_exceeded\",\"type\":\"usage_limit_reached\",\"message\":\"wait\",\"scope\":\"model\",\"resets_in_seconds\":90}}}\n\n")),
                "json-unexpected" => json_response(&final_response("completed")),
                "both" if body["stream"] == false => Response::builder().header(header::CONTENT_TYPE, "application/json").body(Body::from(format!("  {}\n", final_response("completed")))).unwrap(),
                _ => stream_response(Body::from(events("completed"))),
            };
            response
                .headers_mut()
                .insert("x-request-id", HeaderValue::from_static("req_upstream"));
            response
        }

        async fn fixture() -> (String, Requests, tokio::task::JoinHandle<()>) {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let app = Router::new()
                .fallback(upstream)
                .with_state(requests.clone());
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            (base, requests, server)
        }

        #[tokio::test]
        async fn codex_keeps_native_payload_events_and_terminal_response() {
            let (base, requests, server) = fixture().await;
            let client = reqwest::Client::new();
            for streaming in [true, false] {
                let original = json!({"model":"auto","stream":streaming,"store":true,"input":[{"type":"reasoning","encrypted_content":"earlier"}],
                    "tools":[{"type":"web_search"}],"previous_response_id":"resp_previous","include":["custom.include"],"metadata":{"original":true}});
                let response = route_responses(
                    AccountProvider::Codex,
                    json!({"accessToken":"fixture","accountId":"own-account","baseUrl":base}),
                    "native",
                    original.clone(),
                    &client,
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(response.headers()["x-request-id"], "req_upstream");
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                if streaming {
                    assert_eq!(bytes.as_ref(), events("completed").as_bytes());
                } else {
                    assert_eq!(
                        serde_json::from_slice::<Value>(&bytes).unwrap(),
                        final_response("completed")
                    );
                }
                let recorded = requests.lock().await;
                let sent = recorded.last().unwrap();
                assert_eq!(sent.0, "/responses");
                assert_eq!(sent.1["chatgpt-account-id"], "own-account");
                let mut expected = original;
                expected["model"] = json!("native");
                expected["stream"] = json!(true);
                expected["store"] = json!(false);
                expected["instructions"] = json!("");
                assert_eq!(sent.2, expected);
            }
            server.abort();
        }

        #[tokio::test]
        async fn copilot_prefers_catalog_responses_and_preserves_json_and_request_fields() {
            let (base, requests, server) = fixture().await;
            let client = reqwest::Client::new();
            let credentials =
                json!({"accessToken":"fixture","baseUrl":base,"modelProtocols":{"both":"chat"}});
            let original = json!({"model":"auto","stream":false,"store":true,"previous_response_id":"resp_previous","input":"continue","tools":[{"type":"web_search"}],"metadata":{"kept":true}});
            let response = route_responses(
                AccountProvider::Copilot,
                credentials.clone(),
                "both",
                original.clone(),
                &client,
            )
            .await
            .unwrap()
            .unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            assert_eq!(
                bytes.as_ref(),
                format!("  {}\n", final_response("completed")).as_bytes()
            );
            for model in ["chat-only", "blocked"] {
                assert!(route_responses(
                    AccountProvider::Copilot,
                    credentials.clone(),
                    model,
                    original.clone(),
                    &client
                )
                .await
                .unwrap()
                .is_none());
            }
            let recorded = requests.lock().await;
            assert_eq!(
                recorded
                    .iter()
                    .filter(|call| call.0 == "/responses")
                    .count(),
                1
            );
            let mut expected = original;
            expected["model"] = json!("both");
            assert_eq!(recorded[1].2, expected);
            server.abort();
        }

        #[tokio::test]
        async fn streaming_json_is_rejected_and_response_credentials_are_removed() {
            let (base, _, server) = fixture().await;
            let error = route_responses(
                AccountProvider::Codex,
                json!({"accessToken":"fixture","baseUrl":base}),
                "json-unexpected",
                json!({"stream":true,"input":"hello"}),
                &reqwest::Client::new(),
            )
            .await
            .unwrap_err();
            assert_eq!(error.status, StatusCode::BAD_GATEWAY);
            assert_eq!(
                responses_response(json_response(&final_response("completed")), true)
                    .unwrap_err()
                    .status,
                StatusCode::BAD_GATEWAY
            );
            for streaming in [false, true] {
                let mut response = stream_response(Body::from(events("completed")));
                response.headers_mut().append(
                    header::SET_COOKIE,
                    HeaderValue::from_static("session=fixture"),
                );
                response.headers_mut().append(
                    header::SET_COOKIE,
                    HeaderValue::from_static("other=fixture"),
                );
                for name in [
                    "authorization",
                    "proxy-authorization",
                    "cookie",
                    "keep-alive",
                    "upgrade",
                    "x-hop-secret",
                ] {
                    response
                        .headers_mut()
                        .insert(name, HeaderValue::from_static("fixture"));
                }
                response.headers_mut().insert(
                    header::CONNECTION,
                    HeaderValue::from_static("keep-alive, x-hop-secret"),
                );
                response
                    .headers_mut()
                    .insert("x-request-id", HeaderValue::from_static("req_original"));
                response
                    .headers_mut()
                    .insert("x-ratelimit-remaining", HeaderValue::from_static("7"));
                let response = responses_response(response, streaming).unwrap();
                for name in [
                    "set-cookie",
                    "authorization",
                    "proxy-authorization",
                    "cookie",
                    "connection",
                    "keep-alive",
                    "upgrade",
                    "x-hop-secret",
                ] {
                    assert!(
                        !response.headers().contains_key(name),
                        "{name} must not be relayed"
                    );
                }
                assert_eq!(response.headers()["x-request-id"], "req_original");
                assert_eq!(response.headers()["x-ratelimit-remaining"], "7");
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                if streaming {
                    assert_eq!(bytes.as_ref(), events("completed").as_bytes());
                } else {
                    assert_eq!(
                        serde_json::from_slice::<Value>(&bytes).unwrap(),
                        final_response("completed")
                    );
                }
            }
            server.abort();
        }

        #[tokio::test]
        async fn failed_http_response_keeps_status_body_and_retry_metadata() {
            for streaming in [false, true] {
                let original = json!({"error":{"code":"rate_limit_exceeded","message":"Wait"}});
                let mut response = json_response(&original);
                *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from_static("120"));
                response.headers_mut().append(
                    header::SET_COOKIE,
                    HeaderValue::from_static("session=fixture"),
                );
                let response = responses_response(response, streaming).unwrap();
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(response.headers()[header::RETRY_AFTER], "120");
                assert!(!response.headers().contains_key(header::SET_COOKIE));
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), original);
            }
        }

        #[tokio::test]
        async fn unsupported_account_protocol_does_not_contact_upstream() {
            let (base, requests, server) = fixture().await;
            for provider in [
                AccountProvider::ClaudeCode,
                AccountProvider::WorkBuddy,
                AccountProvider::WorkBuddyAi,
                AccountProvider::Antigravity,
                AccountProvider::Kilo,
            ] {
                assert!(route_responses(
                    provider,
                    json!({"accessToken":"fixture","baseUrl":base}),
                    "responses-named-model",
                    json!({"input":"hello"}),
                    &reqwest::Client::new()
                )
                .await
                .unwrap()
                .is_none());
            }
            assert!(requests.lock().await.is_empty());
            server.abort();
        }

        #[tokio::test]
        async fn native_stream_keeps_failure_events_while_nonstream_retains_typed_failure() {
            use std::error::Error;
            let (base, _, server) = fixture().await;
            let credentials = json!({"accessToken":"fixture","baseUrl":base});
            let response = route_responses(
                AccountProvider::Codex,
                credentials.clone(),
                "failed",
                json!({"stream":true,"input":"hello"}),
                &reqwest::Client::new(),
            )
            .await
            .unwrap()
            .unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            assert!(String::from_utf8(bytes.to_vec())
                .unwrap()
                .starts_with("event: response.failed\n"));
            let response = route_responses(
                AccountProvider::Codex,
                credentials,
                "failed",
                json!({"stream":false,"input":"hello"}),
                &reqwest::Client::new(),
            )
            .await
            .unwrap()
            .unwrap();
            let error = response
                .into_body()
                .into_data_stream()
                .next()
                .await
                .unwrap()
                .unwrap_err();
            let mut cause: &(dyn Error + 'static) = &error;
            let metadata = loop {
                if let Some(metadata) = cause.downcast_ref::<ProxyError>() {
                    break metadata;
                }
                cause = cause
                    .downcast_ref::<std::io::Error>()
                    .and_then(|error| error.get_ref())
                    .map(|error| error as &(dyn Error + 'static))
                    .or_else(|| cause.source())
                    .unwrap();
            };
            assert_eq!(metadata.status, StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(metadata.retry_after.as_ref().unwrap(), "90");
            assert!(metadata.model_scope);
            server.abort();
        }

        #[tokio::test]
        async fn nonstream_incomplete_is_preserved_and_truncation_is_not_completion() {
            let (base, _, server) = fixture().await;
            for model in ["incomplete", "headerless", "truncated"] {
                let response = route_responses(
                    AccountProvider::Codex,
                    json!({"accessToken":"fixture","baseUrl":base}),
                    model,
                    json!({"input":"hello"}),
                    &reqwest::Client::new(),
                )
                .await
                .unwrap()
                .unwrap();
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await;
                if model == "truncated" {
                    assert!(bytes.is_err());
                } else {
                    let status = if model == "incomplete" {
                        "incomplete"
                    } else {
                        "completed"
                    };
                    assert_eq!(
                        serde_json::from_slice::<Value>(&bytes.unwrap()).unwrap(),
                        final_response(status)
                    );
                }
            }
            assert_eq!(
                route_responses(
                    AccountProvider::Codex,
                    json!({"accessToken":"fixture","baseUrl":base}),
                    "bad-content",
                    json!({"input":"hello"}),
                    &reqwest::Client::new()
                )
                .await
                .unwrap_err()
                .status,
                StatusCode::BAD_GATEWAY
            );
            server.abort();
        }

        #[tokio::test]
        async fn nonstream_terminal_response_must_match_its_event_and_keep_valid_incomplete() {
            for status in ["completed", "incomplete"] {
                let original = final_response(status);
                let kind = format!("response.{status}");
                let value = json!({"type":kind,"response":original});
                let response = responses_response(
                    stream_response(Body::from(format!("event: {kind}\ndata: {value}\n\n"))),
                    false,
                )
                .unwrap();
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), original);
            }
            for terminal in [
                Value::Null,
                json!({}),
                json!({"id":"resp_fixture","object":"chat.completion","status":"completed"}),
                json!({"id":"resp_fixture","object":"response","status":"in_progress"}),
                json!({"id":"resp_fixture","object":"response","status":"failed"}),
                json!({"id":"resp_fixture","object":"response","status":"incomplete"}),
                json!({"id":"","object":"response","status":"completed"}),
                json!({"object":"response","status":"completed"}),
                json!({"id":"resp_fixture","object":"response","status":"completed","error":{"code":"rate_limit_exceeded","type":"usage_limit_reached","message":"wait","resets_in_seconds":90}}),
            ] {
                let value = json!({"type":"response.completed","response":terminal});
                let response = responses_response(
                    stream_response(Body::from(format!(
                        "event: response.completed\ndata: {value}\n\n"
                    ))),
                    false,
                )
                .unwrap();
                assert!(axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .is_err());
            }
            let value = json!({"type":"response.completed","response":final_response("completed")});
            let response = responses_response(
                stream_response(Body::from(format!(
                    "event: response.incomplete\ndata: {value}\n\n"
                ))),
                false,
            )
            .unwrap();
            assert!(axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .is_err());
        }

        #[tokio::test]
        async fn nonstream_collector_handles_split_utf8_and_cancels_pending_source() {
            use std::time::Duration;
            let wire = events("completed");
            let response = stream_response(Body::from_stream(futures_util::stream::iter(
                wire.into_bytes()
                    .into_iter()
                    .map(|byte| Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(&[byte]))),
            )));
            let response = responses_response(response, false).unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                final_response("completed")
            );

            let (tx, rx) = mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(1);
            let response = responses_response(
                stream_response(Body::from_stream(ReceiverStream::new(rx))),
                false,
            )
            .unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(20), tx.closed())
                .await
                .is_err());
            drop(response);
            tokio::time::timeout(Duration::from_secs(5), tx.closed())
                .await
                .unwrap();
        }
    }

    mod copilot {
        use super::*;
        use axum::http::Method;

        type Calls = Arc<Mutex<Vec<(Method, String, reqwest::header::HeaderMap, Value)>>>;

        fn events(path: &str, tool: bool, continued: bool) -> String {
            let text = if continued { "probe-ok" } else { "ok" };
            let call = json!({"type":"function_call","id":"fc_fixture","call_id":"call_fixture","name":"echo_probe","arguments":"{\"value\":\"probe-ok\"}"});
            let frames = match path {
                "/chat/completions" if tool => vec![
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_fixture","type":"function","function":{"name":"echo_probe","arguments":"{\"value\":"}}]},"finish_reason":null}]}),
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"probe-ok\"}"}}]},"finish_reason":null}]}),
                    json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
                ],
                "/chat/completions" => vec![
                    json!({"choices":[{"delta":{"content":text},"finish_reason":null}]}),
                    json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
                ],
                "/responses" if tool => vec![
                    json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_fixture","call_id":"call_fixture","name":"echo_probe","arguments":""}}),
                    json!({"type":"response.function_call_arguments.delta","item_id":"fc_fixture","delta":"{\"value\":"}),
                    json!({"type":"response.function_call_arguments.delta","item_id":"fc_fixture","delta":"\"probe-ok\"}"}),
                    json!({"type":"response.output_item.done","output_index":0,"item":call}),
                    json!({"type":"response.completed","response":{"object":"response","id":"resp_tool_fixture","status":"completed","error":null,"output":[call],"usage":{"input_tokens":1,"output_tokens":2}}}),
                ],
                "/responses" => vec![
                    json!({"type":"response.output_text.delta","delta":text}),
                    json!({"type":"response.completed","response":{"object":"response","id":"resp_fixture","status":"completed","error":null,"usage":{"input_tokens":1,"output_tokens":2}}}),
                ],
                _ if tool => vec![
                    json!({"type":"message_start","message":{"usage":{"input_tokens":1,"output_tokens":0}}}),
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_fixture","name":"echo_probe","input":{}}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"value\":"}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"probe-ok\"}"}}),
                    json!({"type":"content_block_stop","index":0}),
                    json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":2}}),
                    json!({"type":"message_stop"}),
                ],
                _ => vec![
                    json!({"type":"message_start","message":{"usage":{"input_tokens":1,"output_tokens":0}}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
                    json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
                    json!({"type":"message_stop"}),
                ],
            };
            let mut output = String::new();
            for frame in frames {
                if let Some(kind) = frame["type"].as_str() {
                    output.push_str(&format!("event: {kind}\n"));
                }
                output.push_str(&format!("data: {frame}\n\n"));
            }
            if path == "/chat/completions" {
                output.push_str("data: [DONE]\n\n");
            }
            output
        }

        async fn upstream(State(calls): State<Calls>, request: Request) -> Response {
            let method = request.method().clone();
            let path = request.uri().to_string();
            let headers = request.headers().clone();
            let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let body = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            calls
                .lock()
                .await
                .push((method.clone(), path.clone(), headers, body.clone()));
            if method == Method::GET && path == "/models" {
                return json_response(&json!({"data":[
                    {"id":"chat-test","supported_endpoints":["/chat/completions","/responses","/v1/messages"]},
                    {"id":"responses-test","supported_endpoints":["/responses"]},
                    {"id":"messages-test","supported_endpoints":["/v1/messages"]},
                    {"id":"messages-alias-test","supported_endpoints":["/messages"]},
                    {"id":"disabled","supported_endpoints":["/chat/completions"],"policy":{"state":"disabled"}},
                    {"id":"hidden","supported_endpoints":["/responses"],"visibility":"hide"},
                    {"id":"unpicked","supported_endpoints":["/responses"],"model_picker_enabled":false,"policy":{"state":"enabled"}},
                    {"id":"legacy-chat","capabilities":{"type":"chat"},"model_picker_enabled":false,"policy":{"state":"enabled"}},
                    {"id":"legacy-embedding","capabilities":{"type":"embeddings"}},
                    {"id":"legacy-unknown"},
                    {"id":"legacy-disabled","capabilities":{"type":"chat"},"policy":{"state":"disabled"}},
                    {"id":"empty-endpoints","capabilities":{"type":"chat"},"supported_endpoints":[]},
                    {"id":"api-disabled","supported_endpoints":["/responses"],"supported_in_api":false},
                    {"id":"unsupported","supported_endpoints":["/embeddings"]}
                ]}));
            }
            let expected = match body["model"].as_str() {
                Some("chat-test" | "legacy-chat") => "/chat/completions",
                Some("responses-test" | "unpicked") => "/responses",
                Some("messages-test") => "/v1/messages",
                Some("messages-alias-test") => "/messages",
                _ => "",
            };
            if method != Method::POST || path != expected || expected.is_empty() {
                let mut response =
                    json_response(&json!({"error":{"message":"Unexpected fixture endpoint"}}));
                *response.status_mut() = StatusCode::BAD_REQUEST;
                return response;
            }
            let continued = body["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|message| {
                    message["role"] == "tool"
                        || message["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|part| part["type"] == "tool_result")
                })
                || body["input"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|item| item["type"] == "function_call_output");
            let tool = !continued
                && body["tools"]
                    .as_array()
                    .is_some_and(|tools| !tools.is_empty());
            stream_response(Body::from(events(&path, tool, continued)))
        }

        async fn fixture() -> (Value, Calls, tokio::task::JoinHandle<()>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let app = Router::new().fallback(upstream).with_state(calls.clone());
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            (
                json!({"accessToken":"fixture-copilot","baseUrl":base,"headers":{"Editor-Version":"copilot/1.0.95","Copilot-Integration-Id":"copilot-developer-cli","X-GitHub-Api-Version":"2026-07-01"}}),
                calls,
                server,
            )
        }

        async fn result(response: Response, streaming: bool) -> Value {
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            if !streaming {
                return serde_json::from_slice::<Value>(&bytes).unwrap();
            }
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(text.contains("data: [DONE]"));
            let mut message = json!({"role":"assistant","content":""});
            let mut content = String::new();
            let mut call = json!({"id":"","type":"function","function":{"name":"","arguments":""}});
            let mut arguments = String::new();
            let mut reason = Value::Null;
            for line in text
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter(|line| *line != "[DONE]")
            {
                let chunk: Value = serde_json::from_str(line).unwrap();
                let choice = &chunk["choices"][0];
                if let Some(value) = choice["delta"]["content"].as_str() {
                    content.push_str(value);
                }
                for delta in choice["delta"]["tool_calls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    assert_eq!(delta["index"], 0);
                    if let Some(id) = delta.get("id") {
                        call["id"] = id.clone();
                    }
                    if let Some(name) = delta["function"].get("name") {
                        call["function"]["name"] = name.clone();
                    }
                    if let Some(value) = delta["function"]["arguments"].as_str() {
                        arguments.push_str(value);
                    }
                }
                if !choice["finish_reason"].is_null() {
                    reason = choice["finish_reason"].clone();
                }
            }
            message["content"] = json!(content);
            if !call["id"].as_str().unwrap().is_empty() {
                call["function"]["arguments"] = json!(arguments);
                message["tool_calls"] = json!([call]);
            }
            json!({"choices":[{"message":message,"finish_reason":reason}]})
        }

        #[tokio::test]
        async fn catalog_uses_signed_get_and_filters_account_disabled_models() {
            let (credentials, calls, server) = fixture().await;
            assert_eq!(
                list_models(
                    AccountProvider::Copilot,
                    credentials,
                    &reqwest::Client::new()
                )
                .await
                .unwrap(),
                vec![
                    "chat-test",
                    "legacy-chat",
                    "messages-alias-test",
                    "messages-test",
                    "responses-test",
                    "unpicked",
                ]
            );
            let calls = calls.lock().await;
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, Method::GET);
            assert_eq!(calls[0].1, "/models");
            assert!(calls[0].3.is_null());
            assert_eq!(calls[0].2["authorization"], "Bearer fixture-copilot");
            assert_eq!(
                calls[0].2["copilot-integration-id"],
                "copilot-developer-cli"
            );
            assert_eq!(calls[0].2["editor-version"], "copilot/1.0.95");
            assert_eq!(calls[0].2["x-github-api-version"], "2026-07-01");
            assert_eq!(calls[0].2.get_all("editor-version").iter().count(), 1);
            server.abort();
        }

        #[tokio::test]
        async fn picker_visibility_does_not_deny_copilot_api_models() {
            let (credentials, calls, server) = fixture().await;
            let client = reqwest::Client::new();
            let response = route(
                AccountProvider::Copilot,
                credentials.clone(),
                "unpicked",
                json!({"messages":[{"role":"user","content":"hello"}]}),
                &client,
            )
            .await
            .unwrap();
            assert_eq!(
                result(response, false).await["choices"][0]["message"]["content"],
                "ok"
            );
            let recorded = calls.lock().await;
            assert_eq!(recorded.len(), 2);
            assert_eq!(recorded[0].0, Method::GET);
            assert_eq!(recorded[0].1, "/models");
            assert_eq!(recorded[1].0, Method::POST);
            assert_eq!(recorded[1].1, "/responses");
            assert_eq!(recorded[1].3["model"], "unpicked");
            drop(recorded);
            let other_provider_models = list_models(AccountProvider::Kilo, credentials, &client)
                .await
                .unwrap();
            assert!(!other_provider_models.iter().any(|id| id == "unpicked"));
            server.abort();
        }

        #[tokio::test]
        async fn catalog_selected_endpoints_stream_tools_and_continue_without_quota() {
            let (credentials, calls, server) = fixture().await;
            let client = reqwest::Client::new();
            for (model, path) in [
                ("chat-test", "/chat/completions"),
                ("legacy-chat", "/chat/completions"),
                ("responses-test", "/responses"),
                ("messages-test", "/v1/messages"),
                ("messages-alias-test", "/messages"),
            ] {
                for streaming in [false, true] {
                    let history = json!([{"role":"system","content":"Original system"},{"role":"user","content":"Call echo_probe"}]);
                    let mut body = json!({"stream":streaming,"messages":history,"max_tokens":128});
                    let text = result(
                        route(
                            AccountProvider::Copilot,
                            credentials.clone(),
                            model,
                            body.clone(),
                            &client,
                        )
                        .await
                        .unwrap(),
                        streaming,
                    )
                    .await;
                    assert_eq!(text["choices"][0]["message"]["content"], "ok");
                    body["tools"] = json!([{"type":"function","function":{"name":"echo_probe","parameters":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}}}]);
                    body["tool_choice"] =
                        json!({"type":"function","function":{"name":"echo_probe"}});
                    body["parallel_tool_calls"] = json!(false);
                    let tool = result(
                        route(
                            AccountProvider::Copilot,
                            credentials.clone(),
                            model,
                            body.clone(),
                            &client,
                        )
                        .await
                        .unwrap(),
                        streaming,
                    )
                    .await;
                    assert_eq!(tool["choices"][0]["finish_reason"], "tool_calls");
                    let message = &tool["choices"][0]["message"];
                    assert_eq!(message["tool_calls"][0]["id"], "call_fixture");
                    assert_eq!(message["tool_calls"][0]["function"]["name"], "echo_probe");
                    assert_eq!(
                        serde_json::from_str::<Value>(
                            message["tool_calls"][0]["function"]["arguments"]
                                .as_str()
                                .unwrap()
                        )
                        .unwrap(),
                        json!({"value":"probe-ok"})
                    );
                    body["messages"].as_array_mut().unwrap().extend([
                        message.clone(),
                        json!({"role":"tool","tool_call_id":"call_fixture","content":"probe-ok"}),
                    ]);
                    body["tool_choice"] = json!("auto");
                    let continuation = result(
                        route(
                            AccountProvider::Copilot,
                            credentials.clone(),
                            model,
                            body,
                            &client,
                        )
                        .await
                        .unwrap(),
                        streaming,
                    )
                    .await;
                    assert_eq!(continuation["choices"][0]["message"]["content"], "probe-ok");
                    assert_eq!(continuation["choices"][0]["finish_reason"], "stop");
                    let recorded = calls.lock().await;
                    let stages = &recorded[recorded.len() - 6..];
                    assert_eq!(
                        stages
                            .iter()
                            .map(|call| (call.0.clone(), call.1.as_str()))
                            .collect::<Vec<_>>(),
                        vec![
                            (Method::GET, "/models"),
                            (Method::POST, path),
                            (Method::GET, "/models"),
                            (Method::POST, path),
                            (Method::GET, "/models"),
                            (Method::POST, path)
                        ]
                    );
                    for call in stages {
                        assert_eq!(call.2["authorization"], "Bearer fixture-copilot");
                    }
                    let request = &stages[3].3;
                    assert_eq!(request["model"], model);
                    assert_eq!(request["stream"], true);
                    match path {
                        "/chat/completions" => {
                            assert_eq!(
                                request["tool_choice"],
                                json!({"type":"function","function":{"name":"echo_probe"}})
                            );
                            assert_eq!(stages[5].3["messages"][3]["tool_call_id"], "call_fixture");
                            assert_eq!(stages[5].3["messages"][3]["content"], "probe-ok");
                        }
                        "/responses" => {
                            assert_eq!(request["tools"][0]["type"], "function");
                            assert_eq!(
                                request["tool_choice"],
                                json!({"type":"function","name":"echo_probe"})
                            );
                            let input = stages[5].3["input"].as_array().unwrap();
                            assert!(input.iter().any(|item| item["type"] == "function_call"
                                && item["call_id"] == "call_fixture"
                                && item["arguments"] == "{\"value\":\"probe-ok\"}"));
                            assert!(input
                                .iter()
                                .any(|item| item["type"] == "function_call_output"
                                    && item["call_id"] == "call_fixture"
                                    && item["output"] == "probe-ok"));
                        }
                        _ => {
                            assert_eq!(stages[3].2["anthropic-version"], "2023-06-01");
                            assert_eq!(request["system"], "Original system");
                            assert_eq!(request["max_tokens"], 128);
                            assert_eq!(
                                request["tool_choice"],
                                json!({"type":"tool","name":"echo_probe","disable_parallel_tool_use":true})
                            );
                            assert!(request["tools"][0].get("description").is_none());
                            assert_eq!(
                                stages[5].3["messages"][1]["content"][0]["type"],
                                "tool_use"
                            );
                            assert_eq!(
                                stages[5].3["messages"][1]["content"][0]["id"],
                                "call_fixture"
                            );
                            assert_eq!(
                                stages[5].3["messages"][1]["content"][0]["input"],
                                json!({"value":"probe-ok"})
                            );
                            assert_eq!(
                                stages[5].3["messages"][2]["content"][0],
                                json!({"type":"tool_result","tool_use_id":"call_fixture","content":"probe-ok"})
                            );
                        }
                    }
                }
            }
            assert_eq!(calls.lock().await.len(), 60);
            server.abort();
        }

        #[tokio::test]
        async fn missing_disabled_and_unsupported_models_never_start_inference() {
            let (credentials, calls, server) = fixture().await;
            let client = reqwest::Client::new();
            for (model, status) in [
                ("missing", StatusCode::NOT_FOUND),
                ("disabled", StatusCode::NOT_FOUND),
                ("hidden", StatusCode::NOT_FOUND),
                ("api-disabled", StatusCode::NOT_FOUND),
                ("unsupported", StatusCode::BAD_REQUEST),
                ("legacy-embedding", StatusCode::BAD_REQUEST),
                ("legacy-unknown", StatusCode::BAD_REQUEST),
                ("legacy-disabled", StatusCode::NOT_FOUND),
                ("empty-endpoints", StatusCode::BAD_REQUEST),
            ] {
                let error = route(
                    AccountProvider::Copilot,
                    credentials.clone(),
                    model,
                    json!({"messages":[]}),
                    &client,
                )
                .await
                .unwrap_err();
                assert_eq!(error.status, status);
            }
            let calls = calls.lock().await;
            assert_eq!(calls.len(), 9);
            assert!(calls
                .iter()
                .all(|call| call.0 == Method::GET && call.1 == "/models"));
            server.abort();
        }

        #[test]
        fn anthropic_tool_choices_preserve_forcing_and_parallel_contract() {
            for (choice, expected) in [
                (
                    json!("auto"),
                    json!({"type":"auto","disable_parallel_tool_use":true}),
                ),
                (
                    json!("required"),
                    json!({"type":"any","disable_parallel_tool_use":true}),
                ),
                (json!("none"), json!({"type":"none"})),
            ] {
                assert_eq!(
                    anthropic_request(
                        &json!({"messages":[],"tool_choice":choice,"parallel_tool_calls":false}),
                        "messages-test"
                    )["tool_choice"],
                    expected
                );
            }
        }
    }
}
