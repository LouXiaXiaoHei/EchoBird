use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::OnceCell;

pub(crate) const SAMPLE_TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) const SAMPLE_COUNT: usize = 5;
const MAX_LATENCY_MS: u64 = 1000;
type Measurement = Result<u64, &'static str>;

struct Batch {
    targets: HashMap<String, Arc<OnceCell<Measurement>>>,
    updated: Instant,
}

static BATCHES: OnceLock<Mutex<HashMap<String, Batch>>> = OnceLock::new();

pub(crate) fn finish_batch(batch_id: &str) {
    if let Some(batches) = BATCHES.get() {
        batches.lock().unwrap().remove(batch_id);
    }
}

pub(crate) fn origin(endpoint: &str) -> Option<String> {
    let url = url::Url::parse(endpoint).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        .then(|| url.origin().ascii_serialization())
}

pub(crate) async fn measure(endpoint: &str, batch_id: Option<&str>) -> Measurement {
    let origin = origin(endpoint).ok_or("unavailable")?;
    let cell = if let Some(batch_id) = batch_id {
        let mut batches = BATCHES.get_or_init(Default::default).lock().unwrap();
        // Explicit round cleanup is primary; prune abandoned webview rounds as well.
        batches.retain(|_, batch| batch.updated.elapsed() < Duration::from_secs(90));
        let batch = batches.entry(batch_id.to_owned()).or_insert_with(|| Batch {
            targets: HashMap::new(),
            updated: Instant::now(),
        });
        batch.updated = Instant::now();
        batch.targets.entry(origin.clone()).or_default().clone()
    } else {
        Arc::default()
    };
    *cell.get_or_init(|| sample(&origin)).await
}

async fn sample(origin: &str) -> Measurement {
    let mut samples = [0; SAMPLE_COUNT];
    for sample in &mut samples {
        // Each sample includes a new connection, without an earlier sample's pool.
        let client = reqwest::Client::builder()
            .build()
            .map_err(|_| "request_failed")?;
        let started = Instant::now();
        let response = client.head(origin).timeout(SAMPLE_TIMEOUT).send().await;
        let elapsed = started.elapsed();
        match response {
            // Any HTTP status proves reachability; no authentication or inference.
            Ok(_) if elapsed <= SAMPLE_TIMEOUT => *sample = elapsed.as_millis() as u64,
            Ok(_) => return Err("timeout"),
            Err(error) => {
                return Err(if error.is_timeout() {
                    "timeout"
                } else {
                    "request_failed"
                })
            }
        }
    }
    summarize(samples)
}

fn summarize(mut samples: [u64; SAMPLE_COUNT]) -> Measurement {
    samples.sort_unstable();
    let sum = samples[1..SAMPLE_COUNT - 1].iter().sum::<u64>();
    // Classify the unrounded mean so a value just over the limit stays a timeout.
    if sum > MAX_LATENCY_MS * 3 {
        Err("timeout")
    } else {
        Ok((sum + 1) / 3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::any, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn trimmed_mean_applies_the_latency_limit_before_rounding() {
        assert_eq!(summarize([950, 100, 200, 400, 10]), Ok(233));
        assert_eq!(summarize([690, 700, 710, 720, 1200]), Ok(710));
        assert_eq!(summarize([1000, 101, 101, 102, 0]), Ok(101));
        assert_eq!(summarize([1000, 101, 102, 102, 0]), Ok(102));
        assert_eq!(summarize([0; 5]), Ok(0));
        assert_eq!(summarize([1000; 5]), Ok(1000));
        assert_eq!(summarize([0, 999, 1000, 1001, 3000]), Ok(1000));
        assert_eq!(summarize([0, 999, 1000, 1002, 3000]), Err("timeout"));
        assert_eq!(summarize([1300; 5]), Err("timeout"));
    }

    #[tokio::test]
    async fn same_origin_is_sampled_once_per_round_including_concurrent_callers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let app = Router::new().fallback(any(move || {
            captured.fetch_add(1, Ordering::SeqCst);
            async {
                tokio::time::sleep(Duration::from_millis(5)).await;
                StatusCode::UNAUTHORIZED
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let batch = uuid::Uuid::new_v4().to_string();
        let other_path = format!("{endpoint}/anthropic?ignored=true");
        let (first, second) = tokio::join!(
            measure(&endpoint, Some(&batch)),
            measure(&other_path, Some(&batch))
        );
        assert!(first.is_ok());
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        assert_eq!(measure(&endpoint, Some(&batch)).await, first);
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        let next_batch = uuid::Uuid::new_v4().to_string();
        assert!(measure(&endpoint, Some(&next_batch)).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 10);
        finish_batch(&batch);
        assert!(measure(&endpoint, Some(&batch)).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 15);
        finish_batch(&batch);
        finish_batch(&next_batch);
        server.abort();
    }

    #[tokio::test]
    async fn timeout_after_successful_samples_is_not_trimmed_out_or_retried_in_the_round() {
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let app = Router::new().fallback(any(move || {
            let index = captured.fetch_add(1, Ordering::SeqCst);
            async move {
                if index == 2 {
                    std::future::pending::<()>().await;
                }
                StatusCode::OK
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let batch = uuid::Uuid::new_v4().to_string();
        assert_eq!(measure(&endpoint, Some(&batch)).await, Err("timeout"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(measure(&endpoint, Some(&batch)).await, Err("timeout"));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        finish_batch(&batch);
        assert!(measure(&endpoint, None).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 8);
        server.abort();
    }

    #[tokio::test]
    async fn a_slow_response_over_one_second_is_sampled_and_trimmed_after_all_five_requests() {
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let app = Router::new().fallback(any(move || {
            let index = captured.fetch_add(1, Ordering::SeqCst);
            async move {
                let delay = if index == 2 { 1200 } else { 25 };
                tokio::time::sleep(Duration::from_millis(delay)).await;
                StatusCode::OK
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let result = measure(&endpoint, None).await.unwrap();
        assert!((25..=1000).contains(&result));
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        server.abort();
    }

    #[tokio::test]
    async fn each_sample_uses_a_new_connection_and_failure_is_not_averaged() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut sockets = Vec::new();
            for index in 0..3 {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(3), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut request = [0; 4096];
                let count = socket.read(&mut request).await.unwrap();
                assert!(request[..count].starts_with(b"HEAD / HTTP/1.1\r\n"));
                let reply: &[u8] = if index == 2 {
                    b"invalid HTTP\r\n\r\n"
                } else {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
                };
                socket.write_all(reply).await.unwrap();
                sockets.push(socket);
            }
        });
        assert_eq!(measure(&endpoint, None).await, Err("request_failed"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn different_ports_and_invalid_targets_do_not_share_a_result() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(any(|| async { StatusCode::OK }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let silent_endpoint = format!("http://{}", silent.local_addr().unwrap());
        let batch = uuid::Uuid::new_v4().to_string();
        assert!(measure(&endpoint, Some(&batch)).await.is_ok());
        assert_eq!(
            measure(&silent_endpoint, Some(&batch)).await,
            Err("timeout")
        );
        assert_eq!(
            measure("account://saved", Some(&batch)).await,
            Err("unavailable")
        );
        assert_eq!(measure("invalid", Some(&batch)).await, Err("unavailable"));
        finish_batch(&batch);
        server.abort();
    }
}
