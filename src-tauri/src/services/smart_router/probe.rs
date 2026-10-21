use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::services::connection_probe;

const PROBE_TIMEOUT: Duration = connection_probe::SAMPLE_TIMEOUT;
const ROUND_TIMEOUT: Duration = PROBE_TIMEOUT
    .saturating_mul(connection_probe::SAMPLE_COUNT as u32)
    .saturating_add(Duration::from_secs(1));
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
    batch_id: Option<&str>,
    router_id: Option<String>,
) -> ProbeResult {
    let cancel = cancellation(request_id);
    let _guard = ProbeGuard(request_id);
    let request = async {
        let Ok(models) = crate::services::smart_router::profiles::checked(
            router_id,
            crate::services::smart_router::get_candidate_models,
        ) else {
            return ProbeResult::failure("unavailable");
        };
        let Some(model) = models.into_iter().find(|model| {
            model.internal_id == internal_id && model.model_id.as_deref() == Some(model_id)
        }) else {
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
        run_probe(&endpoint, &cancel, ROUND_TIMEOUT, batch_id).await
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => ProbeResult::failure("cancelled"),
        result = tokio::time::timeout(ROUND_TIMEOUT, request) => result.unwrap_or_else(|_| ProbeResult::failure("timeout")),
    }
}

async fn run_probe(
    endpoint: &str,
    cancel: &CancellationToken,
    timeout: Duration,
    batch_id: Option<&str>,
) -> ProbeResult {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => ProbeResult::failure("cancelled"),
        result = tokio::time::timeout(timeout, connection_probe::measure(endpoint, batch_id)) => match result {
            Ok(Ok(latency_ms)) => ProbeResult { latency_ms: Some(latency_ms), error: None },
            Ok(Err(error)) => ProbeResult::failure(error),
            Err(_) => ProbeResult::failure("timeout"),
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
    use connection_probe::origin;
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
        assert_eq!(PROBE_TIMEOUT, Duration::from_secs(3));
        assert_eq!(ROUND_TIMEOUT, Duration::from_secs(16));
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
        let result = run_probe(&endpoint, &CancellationToken::new(), ROUND_TIMEOUT, None).await;
        assert_eq!(result.error, None);
        assert!((25..1000).contains(&result.latency_ms.unwrap()));
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(
            run_probe(&endpoint, &token, PROBE_TIMEOUT, None)
                .await
                .error
                .as_deref(),
            Some("cancelled")
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 5);
        for call in calls.iter() {
            assert_eq!(call.0, Method::HEAD);
            assert_eq!(call.1.path(), "/");
            assert!(!call.2.contains_key("authorization"));
        }
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
        let result = run_probe(
            &endpoint,
            &CancellationToken::new(),
            Duration::from_millis(20),
            None,
        )
        .await;
        assert_eq!(result.error.as_deref(), Some("timeout"));
        assert_eq!(result.latency_ms, None);
        let token = CancellationToken::new();
        let request = run_probe(&endpoint, &token, PROBE_TIMEOUT, None);
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            token.cancel();
        };
        let (result, _) = tokio::join!(request, cancel);
        assert_eq!(result.error.as_deref(), Some("cancelled"));
        assert_eq!(result.latency_ms, None);
        server.abort();
    }

    #[tokio::test]
    async fn cancellation_during_later_samples_stops_the_round_without_publishing_a_value() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let pending = Arc::new(tokio::sync::Notify::new());
        let captured_pending = pending.clone();
        let app = Router::new().fallback(any(move || {
            let index = captured.fetch_add(1, Ordering::SeqCst);
            let pending = captured_pending.clone();
            async move {
                if index == 1 {
                    pending.notify_one();
                    std::future::pending::<()>().await;
                }
                StatusCode::OK
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let token = CancellationToken::new();
        let batch = uuid::Uuid::new_v4().to_string();
        let request = run_probe(&endpoint, &token, ROUND_TIMEOUT, Some(&batch));
        let cancel = async {
            tokio::time::timeout(Duration::from_secs(3), pending.notified())
                .await
                .unwrap();
            token.cancel();
        };
        let (result, _) = tokio::join!(request, cancel);
        assert_eq!(result.error.as_deref(), Some("cancelled"));
        assert_eq!(result.latency_ms, None);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        connection_probe::finish_batch(&batch);
        server.abort();
    }

    #[tokio::test]
    async fn all_five_slow_responses_finish_before_the_final_latency_timeout_is_reported() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let completed = Arc::new(AtomicUsize::new(0));
        let captured = completed.clone();
        let app = Router::new().fallback(any(move || {
            let completed = captured.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(1300)).await;
                completed.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let result = run_probe(&endpoint, &CancellationToken::new(), ROUND_TIMEOUT, None).await;
        assert_eq!(completed.load(Ordering::SeqCst), 5);
        assert_eq!(result.error.as_deref(), Some("timeout"));
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
