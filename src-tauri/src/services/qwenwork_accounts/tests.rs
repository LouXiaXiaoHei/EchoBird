use super::*;
use serde_json::json;

pub(super) fn fixture(edition: Edition) -> Credentials {
    Credentials {
        edition,
        auth: json!({"schemaVersion":2,"token":"fixture-access-secret","refreshToken":"fixture-refresh-secret",
        "expiresAt":"2035-01-01T00:00:00.000Z","user":{"id":"fixture-user","name":"Fixture","email":"fixture@example.test","isBiz":false,
        "planId":"qwen-office-pro","planName":"Pro"},"loginDeviceId":"fixture-machine","identityVersion":0,"loginMethod":"browser","refreshStrategy":"device_token"}),
    }
}

#[test]
fn identities_are_isolated_by_region_user_and_organization_and_summaries_have_no_secrets() {
    let a = fixture(Edition::QwenWork);
    let mut b = a.clone();
    b.edition = Edition::QwenWorkCn;
    let mut c = a.clone();
    c.auth["user"]["isBiz"] = json!(true);
    c.auth["user"]["teamId"] = json!("team-a");
    c.auth["user"]["orgId"] = json!("org-a");
    let mut d = c.clone();
    d.auth["user"]["teamId"] = json!("team-b");
    let ids = std::collections::BTreeSet::from([a.id(), b.id(), c.id(), d.id()]);
    assert_eq!(ids.len(), 4);
    let public = serde_json::to_string(&a.summary()).unwrap();
    assert!(!public.contains("secret"));
    assert!(!public.contains("fixture-machine"));
    assert_eq!(a.summary().remaining, None);
    assert!(a.summary().daily_reward.is_none());
    c.auth["user"]["teamId"] = Value::Null;
    assert!(c.validate().is_err());
}

#[test]
fn passive_saved_listing_never_decrypts_native_credentials_or_changes_files() {
    let root = std::env::temp_dir().join(format!("qwenwork-list-{}", uuid::Uuid::new_v4()));
    let account = fixture(Edition::QwenWorkCn).summary();
    let bytes = serde_json::to_vec(&BTreeMap::from([(
        account.id.clone(),
        Saved {
            account,
            credential: "intentionally-unreadable".into(),
        },
    )]))
    .unwrap();
    std::fs::write(&root, &bytes).unwrap();
    let read = load(&root).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(std::fs::read(&root).unwrap(), bytes);
    assert!(credential(&read, Edition::QwenWork, read.keys().next().unwrap()).is_err());
    std::fs::remove_file(root).unwrap();
}

#[tokio::test]
async fn account_requests_do_not_block_other_regions_or_accounts() {
    let a = request_lock(Edition::QwenWork, "a").unwrap();
    let _blocked = a.lock().await;
    for (ed, id) in [(Edition::QwenWorkCn, "a"), (Edition::QwenWork, "b")] {
        let lock = request_lock(ed, id).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), lock.lock())
                .await
                .is_ok()
        );
    }
}

#[tokio::test]
async fn retrying_an_unpersisted_single_use_rotation_does_not_reuse_the_old_refresh_token() {
    use axum::{routing::post, Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let count = Arc::new(AtomicUsize::new(0));
    let hits = count.clone();
    let app=Router::new().route("/api/v1/deviceToken/refresh",post(move||{let hits=hits.clone();async move {
        if hits.fetch_add(1,Ordering::SeqCst)>0 { return (axum::http::StatusCode::UNAUTHORIZED,Json(json!({"error":"consumed"}))); }
        (axum::http::StatusCode::OK,Json(json!({"device_token":"rotated-once","refresh_token":"next-pair","expires_in":3600})))
    }}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = api::Client::new(Edition::QwenWorkCn, &base, "fixture").unwrap();
    let mut c = fixture(Edition::QwenWorkCn);
    c.auth["user"]["id"] = json!(uuid::Uuid::new_v4().to_string());
    let (key, first) = renew_once(&c, &client).await.unwrap();
    // No commit: a retry must reuse the rotated pair after a local save failure.
    let (_, retry) = renew_once(&c, &client).await.unwrap();
    assert_eq!(retry.auth, first.auth);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(client.refresh(&c).await.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 2);
    pending_rotations().lock().unwrap().remove(&key);
    server.abort();
}
