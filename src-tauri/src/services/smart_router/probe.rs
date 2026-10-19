use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

const PROBE_TIMEOUT: Duration = Duration::from_millis(1000);
static PROBES: OnceLock<Mutex<HashMap<String, (CancellationToken, Instant)>>> = OnceLock::new();

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeResult {
    latency_ms: Option<u64>,
    error: Option<String>,
}

impl ProbeResult {
    fn failure(error: impl Into<String>) -> Self {
        Self {
            latency_ms: None,
            error: Some(error.into()),
        }
    }
}

fn cancellation(request_id: &str) -> CancellationToken {
    let mut probes = PROBES.get_or_init(Default::default).lock().unwrap();
    probes.retain(|_, (_, created)| created.elapsed() < Duration::from_secs(90));
    probes
        .entry(request_id.to_string())
        .or_insert_with(|| (CancellationToken::new(), Instant::now()))
        .0
        .clone()
}

pub(crate) fn cancel_probe(request_id: &str) {
    // Retain an early cancellation until a queued IPC request registers.
    cancellation(request_id).cancel();
}

struct ProbeGuard<'a>(&'a str);
impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        if let Some(probes) = PROBES.get() {
            probes.lock().unwrap().remove(self.0);
        }
    }
}

pub(crate) async fn probe_candidate(
    internal_id: &str,
    model_id: &str,
    request_id: &str,
) -> ProbeResult {
    let cancel = cancellation(request_id);
    let _guard = ProbeGuard(request_id);
    let request = async {
        let Some(model) = crate::services::smart_router::get_candidate_models()
            .into_iter()
            .find(|model| {
                model.internal_id == internal_id && model.model_id.as_deref() == Some(model_id)
            })
        else {
            return ProbeResult::failure("unavailable");
        };
        let endpoint = if let Some(source) = &model.account_source {
            match crate::services::account_model_sources::connection_endpoint(source).await {
                Ok(endpoint) => endpoint,
                Err(_) => return ProbeResult::failure("unavailable"),
            }
        } else if model.base_url.trim().is_empty() {
            model.anthropic_url.unwrap_or_default()
        } else {
            model.base_url
        };
        let Ok(client) = reqwest::Client::builder().build() else {
            return ProbeResult::failure("request_failed");
        };
        run_probe(&client, &endpoint, &cancel, PROBE_TIMEOUT).await
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => ProbeResult::failure("cancelled"),
        result = tokio::time::timeout(PROBE_TIMEOUT, request) => result.unwrap_or_else(|_| ProbeResult::failure("timeout")),
    }
}

fn origin(endpoint: &str) -> Option<String> {
    let url = url::Url::parse(endpoint).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        .then(|| url.origin().ascii_serialization())
}

async fn run_probe(
    client: &reqwest::Client,
    endpoint: &str,
    cancel: &CancellationToken,
    timeout: Duration,
) -> ProbeResult {
    let Some(origin) = origin(endpoint) else {
        return ProbeResult::failure("unavailable");
    };
    let started = Instant::now();
    tokio::select! {
        biased;
        _ = cancel.cancelled() => ProbeResult::failure("cancelled"),
        response = client.head(origin).timeout(timeout).send() => match response {
            // Any HTTP response proves reachability, matching Model Center's HEAD probe.
            Ok(_) if started.elapsed() <= timeout => ProbeResult { latency_ms: Some(started.elapsed().as_millis() as u64), error: None },
            Ok(_) => ProbeResult::failure("timeout"),
            Err(error) => ProbeResult::failure(if error.is_timeout() { "timeout" } else { "request_failed" }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::{HeaderMap, Method, StatusCode, Uri},
        routing::any,
        Router,
    };
    use std::sync::Arc;

    #[test]
    fn origins_keep_ports_and_ipv6_without_sending_credentials_or_api_paths() {
        assert_eq!(
            origin("https://key:secret@example.test:8443/v1?token=secret"),
            Some("https://example.test:8443".into())
        );
        assert_eq!(
            origin("http://[::1]:53682/v1"),
            Some("http://[::1]:53682".into())
        );
        assert_eq!(origin("account://echobird"), None);
        assert_eq!(origin("invalid"), None);
        assert_eq!(PROBE_TIMEOUT, Duration::from_millis(1000));
    }

    #[tokio::test]
    async fn measures_head_headers_in_milliseconds_even_for_http_errors() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let app =
            Router::new().fallback(any(move |method: Method, uri: Uri, headers: HeaderMap| {
                captured.lock().unwrap().push((method, uri, headers));
                async {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    StatusCode::METHOD_NOT_ALLOWED
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        let result = run_probe(
            &client,
            &endpoint,
            &CancellationToken::new(),
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(result.error, None);
        assert!((25..1000).contains(&result.latency_ms.unwrap()));
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(
            run_probe(&client, &endpoint, &token, PROBE_TIMEOUT)
                .await
                .error
                .as_deref(),
            Some("cancelled")
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, Method::HEAD);
        assert_eq!(calls[0].1.path(), "/");
        assert!(!calls[0].2.contains_key("authorization"));
        server.abort();
    }

    #[tokio::test]
    async fn delayed_headers_timeout_and_in_flight_requests_can_be_cancelled() {
        let app = Router::new().fallback(any(|| async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            StatusCode::OK
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        let result = run_probe(
            &client,
            &endpoint,
            &CancellationToken::new(),
            Duration::from_millis(20),
        )
        .await;
        assert_eq!(result.error.as_deref(), Some("timeout"));
        assert_eq!(result.latency_ms, None);
        let token = CancellationToken::new();
        let request = run_probe(&client, &endpoint, &token, PROBE_TIMEOUT);
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            token.cancel();
        };
        let (result, _) = tokio::join!(request, cancel);
        assert_eq!(result.error.as_deref(), Some("cancelled"));
        assert_eq!(result.latency_ms, None);
        server.abort();
    }

    #[test]
    fn early_ipc_cancellation_is_retained_and_then_cleaned_up() {
        let id = "head-probe-test-early-cancellation";
        cancel_probe(id);
        assert!(cancellation(id).is_cancelled());
        drop(ProbeGuard(id));
        assert!(!PROBES.get().unwrap().lock().unwrap().contains_key(id));
    }
}
