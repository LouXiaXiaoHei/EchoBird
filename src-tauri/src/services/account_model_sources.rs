use serde::Serialize;
use serde_json::Value;

use super::{antigravity_accounts, claude_code_accounts, codex_accounts, workbuddy_accounts};
use crate::models::model::{AccountProvider, AccountSource};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedAccountOption {
    pub source: AccountSource,
    pub label: String,
    pub plan: Option<String>,
}

fn option(
    provider: AccountProvider,
    id: String,
    label: String,
    plan: Option<String>,
) -> SavedAccountOption {
    SavedAccountOption {
        source: AccountSource {
            provider,
            account_id: id,
        },
        label,
        plan,
    }
}

/// This directory reads only EchoBird's saved accounts; it never probes a native client.
pub async fn list_accounts(provider: AccountProvider) -> Result<Vec<SavedAccountOption>, String> {
    use AccountProvider::*;
    Ok(match provider {
        ClaudeCode => claude_code_accounts::list_saved_accounts()
            .await?
            .into_iter()
            .map(|a| option(provider, a.id, a.email, a.plan))
            .collect(),
        Codex => codex_accounts::list_saved_accounts()?
            .into_iter()
            .map(|a| option(provider, a.id, a.email, a.plan))
            .collect(),
        WorkBuddy | WorkBuddyAi => workbuddy_accounts::list_saved_accounts(edition(provider))
            .await?
            .into_iter()
            .map(|a| option(provider, a.id, a.name, a.plan))
            .collect(),
        Antigravity => antigravity_accounts::list_saved_accounts()
            .await?
            .into_iter()
            .map(|a| option(provider, a.id, a.email, a.plan))
            .collect(),
        Kilo => super::kilo_accounts::list_saved_accounts()
            .await?
            .into_iter()
            .map(|a| option(provider, a.id, a.email, a.plan))
            .collect(),
        Copilot => super::copilot_accounts::list_saved_accounts()
            .await?
            .into_iter()
            .map(|a| option(provider, a.id, a.login, a.plan))
            .collect(),
    })
}

fn edition(provider: AccountProvider) -> workbuddy_accounts::Edition {
    if provider == AccountProvider::WorkBuddyAi {
        workbuddy_accounts::Edition::Ai
    } else {
        workbuddy_accounts::Edition::Cn
    }
}

pub async fn validate_source(source: &AccountSource) -> Result<(), String> {
    validate_saved_source(source, &list_accounts(source.provider).await?)
}

fn validate_saved_source(
    source: &AccountSource,
    saved: &[SavedAccountOption],
) -> Result<(), String> {
    if saved.iter().any(|a| a.source == *source) {
        Ok(())
    } else {
        Err("accountError.invalidAccount".into())
    }
}

/// Secrets are deliberately not exposed by any Tauri command.
pub(crate) async fn credentials(
    source: &AccountSource,
    force_refresh: bool,
) -> Result<Value, String> {
    use AccountProvider::*;
    match source.provider {
        ClaudeCode => {
            claude_code_accounts::proxy_credentials(&source.account_id, force_refresh).await
        }
        Codex => codex_accounts::proxy_credentials(&source.account_id, force_refresh).await,
        WorkBuddy | WorkBuddyAi => {
            workbuddy_accounts::proxy_credentials(
                edition(source.provider),
                &source.account_id,
                force_refresh,
            )
            .await
        }
        Antigravity => {
            antigravity_accounts::proxy_credentials(&source.account_id, force_refresh).await
        }
        Kilo => super::kilo_accounts::proxy_credentials(&source.account_id, force_refresh).await,
        Copilot => {
            super::copilot_accounts::proxy_credentials(&source.account_id, force_refresh).await
        }
    }
}

pub(crate) async fn credentials_after_rejection(
    source: &AccountSource,
    rejected_credentials: &Value,
) -> Result<Value, String> {
    use AccountProvider::*;
    match source.provider {
        ClaudeCode => {
            claude_code_accounts::proxy_credentials_after_rejection(
                &source.account_id,
                rejected_credentials,
            )
            .await
        }
        Codex => {
            codex_accounts::proxy_credentials_after_rejection(
                &source.account_id,
                rejected_credentials,
            )
            .await
        }
        WorkBuddy | WorkBuddyAi => {
            workbuddy_accounts::proxy_credentials_after_rejection(
                edition(source.provider),
                &source.account_id,
                rejected_credentials,
            )
            .await
        }
        Antigravity => {
            antigravity_accounts::proxy_credentials_after_rejection(
                &source.account_id,
                rejected_credentials,
            )
            .await
        }
        Kilo | Copilot => credentials(source, true).await,
    }
}

pub(crate) fn credential_error(message: String) -> super::account_proxy::ProxyError {
    use axum::http::StatusCode;
    let key = message.split('|').next().unwrap_or_default();
    let http_status = message
        .split('|')
        .find_map(|part| part.trim().strip_prefix("HTTP "))
        .and_then(|status| status.parse::<u16>().ok())
        .and_then(|status| StatusCode::from_u16(status).ok());
    let status = match key {
        "accountError.invalidAccount"
        | "accountError.noAccount"
        | "accountError.loginRequired"
        | "accountError.invalidGrant" => StatusCode::UNAUTHORIZED,
        "accountError.rateLimited" => StatusCode::TOO_MANY_REQUESTS,
        "accountError.network" | "accountError.unavailable" => http_status
            .filter(|status| status.is_server_error() || *status == StatusCode::TOO_MANY_REQUESTS)
            .unwrap_or(StatusCode::BAD_GATEWAY),
        "accountError.auth" => http_status.unwrap_or(StatusCode::UNAUTHORIZED),
        "accountError.format"
        | "accountError.authResponse"
        | "accountError.home"
        | "accountError.keychain"
        | "accountError.initializeClient" => StatusCode::BAD_REQUEST,
        _ => http_status.unwrap_or(StatusCode::BAD_GATEWAY),
    };
    let code = (key.starts_with("accountError.")
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'))
    .then(|| key.to_string());
    let mut error = super::account_proxy::ProxyError::new(status, message);
    error.error_code = code;
    error
}

pub(crate) fn oauth_refresh_error(status: u16, body: &Value) -> String {
    let code = body["error"]
        .as_str()
        .or_else(|| body["error"]["code"].as_str());
    if status == 400
        && code.is_some_and(|code| {
            matches!(
                code,
                "invalid_grant"
                    | "refresh_token_expired"
                    | "refresh_token_reused"
                    | "refresh_token_invalidated"
            )
        })
    {
        format!("accountError.loginRequired|HTTP {status}")
    } else {
        format!("accountError.auth|HTTP {status}")
    }
}

pub async fn list_models(source: &AccountSource) -> Result<Vec<String>, String> {
    let secret = credentials(source, false).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| "accountError.network")?;
    super::account_proxy::list_models(source.provider, secret, &client)
        .await
        .map_err(|e| e.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::json;

    #[test]
    fn credential_failures_preserve_known_categories_and_http_statuses() {
        use axum::http::StatusCode;
        for (message, expected) in [
            ("accountError.invalidAccount", StatusCode::UNAUTHORIZED),
            (
                "accountError.loginRequired|HTTP 400|invalid_grant",
                StatusCode::UNAUTHORIZED,
            ),
            (
                "accountError.auth|HTTP 503",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                "accountError.network|HTTP 429",
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                "accountError.rateLimited|HTTP 429",
                StatusCode::TOO_MANY_REQUESTS,
            ),
            ("accountError.quota|HTTP 402", StatusCode::PAYMENT_REQUIRED),
            (
                "accountError.network|request timed out",
                StatusCode::BAD_GATEWAY,
            ),
            ("accountError.authResponse", StatusCode::BAD_REQUEST),
            ("accountError.format", StatusCode::BAD_REQUEST),
            ("accountError.expired", StatusCode::BAD_GATEWAY),
            ("unknown credential failure", StatusCode::BAD_GATEWAY),
        ] {
            let error = credential_error(message.into());
            assert_eq!(error.status, expected, "{message}");
            assert_eq!(error.message, message);
            assert!(!error.model_scope);
        }
        assert!(credential_error("unknown credential failure".into())
            .error_code
            .is_none());
        assert_eq!(
            credential_error("accountError.rateLimited".into())
                .error_code
                .as_deref(),
            Some("accountError.rateLimited")
        );
    }

    #[test]
    fn only_explicit_refresh_credential_failures_require_login_again() {
        use axum::http::StatusCode;
        for code in [
            "invalid_grant",
            "refresh_token_expired",
            "refresh_token_reused",
            "refresh_token_invalidated",
        ] {
            for body in [json!({"error": code}), json!({"error": {"code": code}})] {
                let message = oauth_refresh_error(400, &body);
                assert_eq!(message, "accountError.loginRequired|HTTP 400");
                assert_eq!(credential_error(message).status, StatusCode::UNAUTHORIZED);
            }
        }
        for body in [
            json!({"error":"invalid_request"}),
            json!({"error":{"code":"invalid_client"}}),
            json!({"message":"invalid_grant"}),
            Value::Null,
        ] {
            let message = oauth_refresh_error(400, &body);
            assert_eq!(message, "accountError.auth|HTTP 400");
            assert_eq!(credential_error(message).status, StatusCode::BAD_REQUEST);
        }
        for (status, expected) in [
            (401, StatusCode::UNAUTHORIZED),
            (429, StatusCode::TOO_MANY_REQUESTS),
            (503, StatusCode::SERVICE_UNAVAILABLE),
        ] {
            assert_eq!(
                credential_error(oauth_refresh_error(
                    status,
                    &json!({"error":"invalid_grant"})
                ))
                .status,
                expected
            );
        }
    }

    #[test]
    fn source_validation_is_bound_to_saved_account_and_provider() {
        let saved = vec![option(
            AccountProvider::WorkBuddy,
            "saved-id".into(),
            "Fixture".into(),
            None,
        )];
        let mut source = AccountSource {
            provider: AccountProvider::WorkBuddy,
            account_id: "saved-id".into(),
        };
        assert!(validate_saved_source(&source, &saved).is_ok());
        source.provider = AccountProvider::WorkBuddyAi;
        assert!(validate_saved_source(&source, &saved).is_err());
        source.provider = AccountProvider::WorkBuddy;
        source.account_id = "missing-id".into();
        assert!(validate_saved_source(&source, &saved).is_err());
        assert!(validate_saved_source(&source, &[]).is_err());
        let value = serde_json::to_value(&saved).unwrap();
        assert_eq!(value[0]["source"]["accountId"], "saved-id");
        assert!(value[0].get("accessToken").is_none());
    }

    #[tokio::test]
    #[ignore = "explicit live saved-account catalog verification; select ECHOBIRD_LIVE_PROXY_PROVIDER"]
    async fn live_saved_account_catalog_probe() -> Result<(), String> {
        let name = std::env::var("ECHOBIRD_LIVE_PROXY_PROVIDER").map_err(|_| {
            "ECHOBIRD_LIVE_PROXY_PROVIDER must select an authorized provider".to_owned()
        })?;
        let provider: AccountProvider = serde_json::from_value(Value::String(name))
            .map_err(|_| "invalid ECHOBIRD_LIVE_PROXY_PROVIDER".to_owned())?;
        let accounts = list_accounts(provider)
            .await
            .map_err(|_| format!("provider={provider:?} modelCount=0 status=store_error"))?;
        let source = accounts
            .first()
            .ok_or_else(|| format!("provider={provider:?} modelCount=0 status=no_saved_account"))?
            .source
            .clone();
        let secret = credentials(&source, false).await.map_err(|error| {
            let code = error.split('|').next().unwrap_or_default();
            let code = if code.starts_with("accountError.")
                && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
            {
                code
            } else {
                "credential_error"
            };
            format!("provider={provider:?} modelCount=0 status={code}")
        })?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| "catalog client unavailable".to_owned())?;
        let models = super::super::account_proxy::list_models(provider, secret, &client)
            .await
            .map_err(|error| {
                format!(
                    "provider={provider:?} modelCount=0 status=catalog_http_{}",
                    error.status.as_u16()
                )
            })?;
        if models.is_empty() {
            return Err(format!(
                "provider={provider:?} modelCount=0 status=empty_catalog"
            ));
        }
        eprintln!(
            "provider={provider:?} modelCount={} status=ok",
            models.len()
        );
        if std::env::var("ECHOBIRD_LIVE_PROXY_SHOW_MODELS").as_deref() == Ok("1") {
            eprintln!("models={models:?}");
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "explicitly save an existing authorized native sign-in in EchoBird; ECHOBIRD_LIVE_PROXY_IMPORT_NATIVE=1 required"]
    async fn live_import_installed_account_probe() -> Result<(), String> {
        if std::env::var("ECHOBIRD_LIVE_PROXY_IMPORT_NATIVE").as_deref() != Ok("1") {
            return Err("native account import flag is required".into());
        }
        let provider = serde_json::from_value::<AccountProvider>(json!(std::env::var(
            "ECHOBIRD_LIVE_PROXY_PROVIDER"
        )
        .map_err(|_| "provider is required")?))
        .map_err(|_| "invalid provider")?;
        let result = match provider {
            AccountProvider::Kilo => super::super::kilo_accounts::capture_current_account()
                .await
                .map(|_| ()),
            AccountProvider::Copilot => super::super::copilot_accounts::import_native_accounts()
                .await
                .map(|_| ()),
            _ => return Err("this probe accepts Kilo or Copilot only".into()),
        };
        result.map_err(|_| format!("provider={provider:?} status=native_import_failed"))?;
        let count = list_accounts(provider).await?.len();
        if count == 0 {
            return Err(format!("provider={provider:?} status=no_native_account"));
        }
        eprintln!("provider={provider:?} savedAccountCount={count} status=ok");
        Ok(())
    }

    #[tokio::test]
    #[ignore = "explicit inference against a saved account; requires provider, model, and ECHOBIRD_LIVE_PROXY_INFERENCE=1"]
    async fn live_saved_account_inference_probe() -> Result<(), String> {
        if std::env::var("ECHOBIRD_LIVE_PROXY_INFERENCE").as_deref() != Ok("1") {
            return Err("live inference flag is required".into());
        }
        let provider = serde_json::from_value::<AccountProvider>(json!(std::env::var(
            "ECHOBIRD_LIVE_PROXY_PROVIDER"
        )
        .map_err(|_| "provider is required")?))
        .map_err(|_| "invalid provider")?;
        let model = std::env::var("ECHOBIRD_LIVE_PROXY_MODEL").map_err(|_| "model is required")?;
        let source = list_accounts(provider)
            .await?
            .first()
            .ok_or("no saved account")?
            .source
            .clone();
        let secret = credentials(&source, false)
            .await
            .map_err(|_| "saved credentials unavailable")?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .map_err(|_| "client unavailable")?;
        let models = super::super::account_proxy::list_models(provider, secret.clone(), &client)
            .await
            .map_err(|e| format!("catalog status={}", e.status.as_u16()))?;
        if !models.contains(&model) {
            return Err("selected model is absent from account catalog".into());
        }
        let request = json!({"stream":false,"max_tokens":128,
            "messages":[{"role":"user","content":"Reply with only OK."}]});
        // Only the tool result contains this value, so ignoring it cannot pass.
        let result_marker = format!("probe-result-{}", uuid::Uuid::new_v4());
        let mut reply = Value::Null;
        for phase in ["text", "stream", "tool", "continuation"] {
            let mut body = request.clone();
            if phase == "stream" {
                body["stream"] = json!(true);
            }
            if matches!(phase, "tool" | "continuation") {
                body["tools"] = json!([{"type":"function","function":{
                    "name":"echo_probe","description":"Return the probe marker.",
                    "parameters":{"type":"object","properties":{"marker":{"type":"string","enum":["probe-ok"]}},
                        "required":["marker"],"additionalProperties":false}}}]);
                if phase == "tool" {
                    body["messages"][0]["content"] = json!("Call echo_probe with marker probe-ok.");
                    body["tool_choice"] =
                        json!({"type":"function","function":{"name":"echo_probe"}});
                } else {
                    let call = &reply["tool_calls"][0];
                    body["messages"] = json!([
                        {"role":"user","content":"Call echo_probe with marker probe-ok."},
                        reply,
                        {"role":"tool","tool_call_id":call["id"],"content":result_marker},
                        {"role":"user","content":"The tool succeeded. Reply with only its returned value."}
                    ]);
                    body["tool_choice"] = json!("none");
                }
            }
            let response = super::super::smart_router::route_live_account_probe(
                source.clone(),
                model.clone(),
                body,
            )
            .await?;
            if !response.status().is_success() {
                let status = response.status();
                let bytes = to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .map_err(|_| "error response collection failed")?;
                let value = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
                let message = value["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let hints: Vec<_> = [
                    "max_tokens",
                    "max_completion_tokens",
                    "temperature",
                    "reasoning",
                    "unsupported",
                    "invalid",
                    "model",
                    "quota",
                    "integration",
                    "permission",
                ]
                .into_iter()
                .filter(|hint| message.contains(hint))
                .collect();
                return Err(format!(
                    "provider={provider:?} phase={phase} status={} hints={hints:?}",
                    status.as_u16()
                ));
            }
            if response.headers().get("X-EchoBird-Routed-Via").is_none() {
                return Err("response did not pass through the smart router".into());
            }
            let bytes = to_bytes(response.into_body(), 1024 * 1024)
                .await
                .map_err(|_| format!("phase={phase} response collection failed"))?;
            if phase == "stream" {
                let frames = std::str::from_utf8(&bytes).map_err(|_| "invalid stream encoding")?;
                let content = frames
                    .lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .filter_map(|data| serde_json::from_str::<Value>(data).ok())
                    .any(|value| {
                        value["choices"][0]["delta"]["content"]
                            .as_str()
                            .is_some_and(|text| !text.is_empty())
                    });
                if !content || !frames.contains("data: [DONE]") {
                    return Err("stream did not contain text and completion".into());
                }
            } else {
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| "invalid JSON response")?;
                reply = value["choices"][0]["message"].clone();
                if phase == "tool" {
                    let call = &reply["tool_calls"][0];
                    let arguments = serde_json::from_str::<Value>(
                        call["function"]["arguments"].as_str().unwrap_or_default(),
                    )
                    .ok();
                    if reply["tool_calls"].as_array().map(Vec::len) != Some(1)
                        || call["function"]["name"] != "echo_probe"
                        || !call["id"].is_string()
                        || arguments.as_ref().and_then(|args| args["marker"].as_str())
                            != Some("probe-ok")
                    {
                        let finish = match value["choices"][0]["finish_reason"].as_str() {
                            Some("length") => "length",
                            Some("tool_calls") => "tool_calls",
                            Some("stop") => "stop",
                            _ => "other",
                        };
                        return Err(format!(
                            "incomplete tool response: finish={finish} count={:?} named={} id={} argsBytes={} argsJson={} marker={} text={} completionTokens={:?} reasoningTokens={:?}",
                            reply["tool_calls"].as_array().map(Vec::len),
                            call["function"]["name"] == "echo_probe",
                            call["id"].is_string(),
                            call["function"]["arguments"].as_str().map_or(0, str::len),
                            arguments.is_some(),
                            arguments.as_ref().and_then(|args| args["marker"].as_str()) == Some("probe-ok"),
                            reply["content"].as_str().is_some_and(|text| !text.is_empty()),
                            value["usage"]["completion_tokens"].as_u64(),
                            value["usage"]["completion_tokens_details"]["reasoning_tokens"].as_u64(),
                        ));
                    }
                } else if reply["content"]
                    .as_str()
                    .map_or(true, |text| text.trim().is_empty())
                {
                    return Err(format!("phase={phase} has no assistant text"));
                }
                if phase == "continuation"
                    && (reply["content"].as_str().map(str::trim) != Some(result_marker.as_str())
                        || reply["tool_calls"]
                            .as_array()
                            .is_some_and(|calls| !calls.is_empty()))
                {
                    return Err("continuation ignored the tool result or tool_choice=none".into());
                }
            }
            eprintln!("provider={provider:?} phase={phase} status=ok");
        }
        Ok(())
    }
}
