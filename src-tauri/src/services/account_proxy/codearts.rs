use std::collections::{BTreeMap, BTreeSet};

use axum::{http::StatusCode, response::Response};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{protocol::WireProtocol, providers, ProxyError};

const SNAP_ENGINE: &str = "https://snap-access.cn-north-4.myhuaweicloud.com";

// Matches the shipped desktop SDK signer and shared inferhub authentication.
pub(crate) fn signed_headers(
    method: &str,
    url: &str,
    credentials: &Value,
    body: &str,
    headers: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    signed_headers_at(
        method,
        url,
        credentials,
        body,
        headers,
        &chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
    )
}

fn encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").expect("string formatting");
        }
    }
    encoded
}

fn signed_headers_at(
    method: &str,
    url: &str,
    credentials: &Value,
    body: &str,
    headers: BTreeMap<String, String>,
    date: &str,
) -> Result<BTreeMap<String, String>, String> {
    let field = |name| {
        credentials[name]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "accountError.loginRequired".to_string())
    };
    let access_key = field("accessKey")?;
    let secret_key = field("secretKey")?;
    let token = field("securityToken")?;
    let url = url::Url::parse(url).map_err(|_| "accountError.format")?;
    let mut headers: BTreeMap<_, _> = headers
        .into_iter()
        .filter(|(key, _)| !key.eq_ignore_ascii_case("authorization"))
        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    headers.insert(
        "host".into(),
        url[url::Position::BeforeHost..url::Position::AfterPort].into(),
    );
    headers.insert("x-security-token".into(), token.into());
    headers.insert("x-sdk-date".into(), date.into());
    let mut uri = url
        .path()
        .split('/')
        .map(encode)
        .collect::<Vec<_>>()
        .join("/");
    if !uri.ends_with('/') {
        uri.push('/');
    }
    let mut query = url
        .query_pairs()
        .map(|(key, value)| format!("{}={}", encode(&key), encode(&value)))
        .collect::<Vec<_>>();
    query.sort();
    let canonical_headers = headers
        .iter()
        .map(|(key, value)| format!("{key}:{value}\n"))
        .collect::<String>();
    let signed = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let body_hash = headers
        .get("x-sdk-content-sha256")
        .cloned()
        .unwrap_or_else(|| format!("{:x}", Sha256::digest(body)));
    let canonical = [
        method.to_uppercase(),
        uri,
        query.join("&"),
        canonical_headers,
        signed.clone(),
        body_hash,
    ]
    .join("\n");
    let to_sign = format!("SDK-HMAC-SHA256\n{date}\n{:x}", Sha256::digest(canonical));
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret_key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(to_sign.as_bytes());
    let signature = format!("{:x}", mac.finalize().into_bytes());
    headers.insert(
        "authorization".into(),
        format!(
            "SDK-HMAC-SHA256 Access={access_key}, SignedHeaders={signed}, Signature={signature}"
        ),
    );
    Ok(headers)
}

pub(super) fn api(credentials: &Value) -> &str {
    #[cfg(test)]
    if let Some(base) = credentials["baseUrl"].as_str() {
        return base.trim_end_matches('/');
    }
    let _ = credentials;
    SNAP_ENGINE
}

fn signed_request(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    credentials: &Value,
    body: &str,
    agent: Option<&str>,
) -> Result<reqwest::RequestBuilder, ProxyError> {
    let mut extra = BTreeMap::from([
        ("content-type".into(), "application/json".into()),
        ("x-language".into(), "zh-cn".into()),
    ]);
    if let Some(agent) = agent {
        extra.insert("agent-type".into(), agent.into());
    }
    let headers = signed_headers(method.as_str(), url, credentials, body, extra)
        .map_err(super::super::account_model_sources::credential_error)?;
    let mut request = client.request(method, url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if !body.is_empty() {
        request = request.body(body.to_owned());
    }
    Ok(request)
}

async fn get(
    client: &reqwest::Client,
    url: &str,
    credentials: &Value,
) -> Result<Value, ProxyError> {
    let response = signed_request(
        client,
        reqwest::Method::GET,
        url,
        credentials,
        "",
        Some("AgentCenter"),
    )?
    .send()
    .await
    .map_err(ProxyError::upstream)?;
    let status = response.status();
    let headers = response.headers().clone();
    let value = response
        .json::<Value>()
        .await
        .map_err(ProxyError::upstream)?;
    if !status.is_success() {
        return Err(providers::upstream_error(status, &headers, &value));
    }
    Ok(value)
}

pub(super) async fn list_models(
    credentials: Value,
    client: &reqwest::Client,
) -> Result<Vec<String>, ProxyError> {
    let base = api(&credentials);
    let agents = get(
        client,
        &format!(
            "{base}/v1/agent-center/agents/useragents?offset=0&limit=100&is_primary_agent=true"
        ),
        &credentials,
    )
    .await?;
    let id = agents["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|agent| {
            agent["agent_name"] == "CodeAgent"
                && agent["alias"]["alias_zh_cn"] == "智能体"
                && agent["show_in_ide"] == true
        })
        .and_then(|agent| agent["agent_id"].as_str())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            ProxyError::new(
                StatusCode::BAD_GATEWAY,
                "CodeArts returned no available CodeAgent",
            )
        })?;
    let detail = get(
        client,
        &format!(
            "{base}/v1/agent-center/agents/detail?agent_id={}",
            encode(id)
        ),
        &credentials,
    )
    .await?;
    catalog_ids(&detail)
}

fn catalog_ids(detail: &Value) -> Result<Vec<String>, ProxyError> {
    let rows = detail["gpts"]["models"].as_array().ok_or_else(|| {
        ProxyError::new(
            StatusCode::BAD_GATEWAY,
            "CodeArts returned no model directory",
        )
    })?;
    let ids: BTreeSet<String> = rows
        .iter()
        .filter(|row| row["model_type"] != "third")
        .filter(|row| {
            !["GLM-5.1", "Qwen3-VL-235B", "maas-glm-4.7"]
                .iter()
                .any(|name| row["model_name"] == *name)
        })
        .filter_map(|row| row["model_parameters"]["model_id"].as_str())
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .collect();
    if ids.is_empty() {
        return Err(ProxyError::new(
            StatusCode::BAD_GATEWAY,
            "CodeArts returned no available chat models",
        ));
    }
    Ok(ids.into_iter().collect())
}

pub(super) async fn route(
    credentials: Value,
    model: &str,
    mut body: Value,
    client: &reqwest::Client,
) -> Result<Response, ProxyError> {
    let streaming = body["stream"].as_bool().unwrap_or(false);
    super::strip_response_metadata(&mut body);
    body["model"] = json!(model);
    body["stream"] = json!(true);
    body["tool_stream"] = json!(true);
    let url = format!("{}/api/v2/chat/completions", api(&credentials));
    let response = signed_request(
        client,
        reqwest::Method::POST,
        &url,
        &credentials,
        &body.to_string(),
        None,
    )?
    .header("accept", "text/event-stream")
    .send()
    .await
    .map_err(ProxyError::upstream)?;
    let status = response.status();
    if !status.is_success() {
        let headers = response.headers().clone();
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        return Err(providers::upstream_error(status, &headers, &value));
    }
    providers::converted(response, WireProtocol::Chat, model, streaming, true).await
}

#[cfg(test)]
mod tests;
