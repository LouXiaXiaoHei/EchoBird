//! Account credentials stay in the backend. These adapters never switch native clients.

mod bridge;
mod protocol;
mod providers;
mod responses;
mod tools;

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::Response,
};
use serde_json::{json, Value};

use crate::models::model::AccountProvider;

pub(crate) use protocol::SseParser;
pub use providers::responses_response;
pub(crate) use providers::stream_error;
pub use responses::{chat_to_responses, chat_to_responses_with_tools, responses_to_chat};

/// Internal Responses state belongs only on a Responses-capable account hop.
pub fn strip_response_metadata(body: &mut Value) {
    if let Some(object) = body.as_object_mut() {
        object.remove(tools::RESPONSES_REQUEST);
    }
    for message in body
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if let Some(object) = message.as_object_mut() {
            object.remove("_echobird_response_items");
        }
        for call in message
            .get_mut("tool_calls")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(object) = call.as_object_mut() {
                object.remove("_echobird_response_item_id");
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyError {
    pub status: StatusCode,
    pub message: String,
    pub error_code: Option<String>,
    pub error_type: Option<String>,
    pub retry_after: Option<HeaderValue>,
    pub model_scope: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProxyFailureMetadata {
    pub model_scope: bool,
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProxyError {}

impl ProxyError {
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            error_code: None,
            error_type: None,
            retry_after: None,
            model_scope: false,
        }
    }

    pub fn into_response(self) -> Response {
        let mut error = json!({"message":self.message,"type":self.error_type.as_deref().unwrap_or("upstream_error")});
        if let Some(code) = self.error_code {
            error["code"] = json!(code);
        }
        let mut response = json_response(&json!({"error":error}));
        *response.status_mut() = self.status;
        if let Some(retry_after) = self.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, retry_after);
        }
        response.extensions_mut().insert(ProxyFailureMetadata {
            model_scope: self.model_scope,
        });
        response
    }

    pub(crate) fn upstream(error: impl std::fmt::Display) -> Self {
        Self::new(
            StatusCode::BAD_GATEWAY,
            format!("Account upstream failed: {error}"),
        )
    }
}

pub async fn route(
    provider: AccountProvider,
    credentials: Value,
    model: &str,
    body: Value,
    client: &reqwest::Client,
) -> Result<Response, ProxyError> {
    if provider == AccountProvider::ClaudeCode {
        bridge::route(credentials, model, body).await
    } else {
        providers::route(provider, credentials, model, body, client).await
    }
}

pub async fn route_responses(
    provider: AccountProvider,
    credentials: Value,
    model: &str,
    body: Value,
    client: &reqwest::Client,
) -> Result<Option<Response>, ProxyError> {
    providers::route_responses(provider, credentials, model, body, client).await
}

pub async fn list_models(
    provider: AccountProvider,
    credentials: Value,
    client: &reqwest::Client,
) -> Result<Vec<String>, ProxyError> {
    if provider == AccountProvider::ClaudeCode {
        bridge::list_models(credentials).await
    } else {
        providers::list_models(provider, credentials, client).await
    }
}

pub(crate) fn json_response(value: &Value) -> Response {
    Response::builder()
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .expect("fixed response headers")
}

pub(crate) fn stream_response(body: Body) -> Response {
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(body)
        .expect("fixed response headers")
}
