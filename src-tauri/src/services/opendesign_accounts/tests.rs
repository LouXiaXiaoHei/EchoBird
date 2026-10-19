use super::*;

fn write_native(path: &Path, saved: &Saved) -> Result<(), String> {
    super::super::native_account_files::write(path, &native_bytes(path, saved)?)
}

#[tokio::test]
async fn official_device_wire_keeps_secrets_in_backend_and_handles_pending_then_approval() {
    use axum::{
        extract::{Request, State},
        response::Json,
        Router,
    };
    use std::sync::Arc;
    type Calls = Arc<tokio::sync::Mutex<Vec<(String, Value)>>>;
    async fn endpoint(State(calls): State<Calls>, request: Request) -> Json<Value> {
        assert_eq!(request.method(), "POST");
        assert!(!request.headers().contains_key("authorization"));
        let path = request.uri().path().to_owned();
        let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let mut records = calls.lock().await;
        records.push((path.clone(), body));
        if path == "/api/v1/cli/device-authorizations" {
            return Json(
                json!({"deviceId":"device","deviceSecret":"private-device-secret",
                "activationUrl":"https://open-design.ai/cloud/cli/activate?code=fixture","pollIntervalSeconds":2}),
            );
        }
        assert_eq!(path, "/api/v1/cli/device-authorizations/device/token");
        if records.len() == 2 {
            return Json(json!({"status":"pending"}));
        }
        Json(profile("approved"))
    }
    let calls = Calls::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(endpoint).with_state(calls.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = client().unwrap();
    let data = start_device(&client, &base).await.unwrap();
    let pending = Pending {
        id: "login".into(),
        device_id: data["deviceId"].as_str().unwrap().into(),
        secret: data["deviceSecret"].as_str().unwrap().into(),
        expires: now() + 60,
    };
    assert!(poll_device(&client, &base, &pending)
        .await
        .unwrap()
        .is_none());
    let saved = poll_device(&client, &base, &pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.account.email, "approved@example.test");
    let records = calls.lock().await;
    assert_eq!(records[0].1["profile"], "prod");
    assert_eq!(records[0].1["host"], "EchoBird");
    assert_eq!(
        records[1].1,
        json!({"deviceSecret":"private-device-secret"})
    );
    assert_eq!(records.len(), 3);
    let public = serde_json::to_string(&saved.account).unwrap();
    assert!(!public.contains("private-device-secret"));
    assert!(!public.contains("sk-amr-"));
    server.abort();
}

fn profile(id: &str) -> Value {
    json!({"runtimeKey":format!("sk-amr-{id}"), "controlKey":format!("vela_ctrl_{id}"),
        "user":{"id":id,"email":format!("{id}@example.test"),"plan":"go"}})
}

#[test]
fn account_validation_does_not_serialize_credentials_to_frontend() {
    let saved = account_from_profile(&profile("one")).unwrap();
    let view = serde_json::to_string(&saved.account).unwrap();
    assert!(!view.contains("sk-amr-"));
    assert!(!view.contains("vela_ctrl_"));
    assert_eq!(saved.account.plan.as_deref(), Some("go"));
    let mut invalid = profile("one");
    invalid["runtimeKey"] = json!("vela_ctrl_wrong-plane");
    assert!(account_from_profile(&invalid).is_err());
    invalid = profile("one");
    invalid["user"]["id"] = json!("");
    assert!(account_from_profile(&invalid).is_err());
}

#[test]
fn native_reads_do_not_create_files_and_switch_preserves_other_profiles_and_settings() {
    let dir = std::env::temp_dir().join(format!("echobird-opendesign-{}", uuid::Uuid::new_v4()));
    let path = dir.join("amr/config.json");
    assert!(native_at(&path).unwrap().is_none());
    assert!(!path.exists());
    let initial = json!({"version":1,"keep":{"setting":true},"profiles":{
        "local":{"runtimeKey":"local-key","apiUrl":"http://localhost:9000"},
        "prod":{"extra":42,"workspaceId":"workspace-existing","runtimeKey":"sk-amr-old",
            "controlKey":"vela_ctrl_old","user":{"id":"old","email":"old@example.test"}}
    }});
    super::super::cursor_auth::write(&path, &initial).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        native_at(&path).unwrap().unwrap().account.email,
        "old@example.test"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let saved = account_from_profile(&profile("new")).unwrap();
    write_native(&path, &saved).unwrap();
    let updated = read_config(&path).unwrap();
    assert_eq!(updated["version"], initial["version"]);
    assert_eq!(updated["keep"], initial["keep"]);
    assert_eq!(updated["profiles"]["local"], initial["profiles"]["local"]);
    assert_eq!(updated["profiles"]["prod"]["extra"], 42);
    assert_eq!(
        native_at(&path).unwrap().unwrap().account.id,
        saved.account.id
    );
    let before = std::fs::read(&path).unwrap();
    write_native(&path, &saved).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::write(&path, "invalid").unwrap();
    assert!(write_native(&path, &saved).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_quota_is_not_zero_and_vendor_windows_show_remaining_allowance() {
    let mut saved = account_from_profile(&profile("one")).unwrap();
    assert!(saved.account.balance_usd.is_none());
    assert!(saved.account.quota_windows.is_empty());
    apply_quota(
        &mut saved.account,
        &json!({"balanceUsd":"2.50","codingPlan":{"windows":[
            {"durationSeconds":604800,"usedCredits":"25","limitCredits":"100","resetsAt":"2026-10-23T00:00:00Z"},
            {"durationSeconds":18000,"usedCredits":40,"limitCredits":50},
            {"durationSeconds":3600,"usedCredits":0,"limitCredits":0}
        ]}}),
        &json!({"membershipTier":"go","subscriptionCurrentPeriodEnd":"2026-11-16T00:00:00Z"}),
    )
    .unwrap();
    assert_eq!(saved.account.balance_usd, Some(2.5));
    assert_eq!(saved.account.quota_windows.len(), 2);
    assert_eq!(saved.account.quota_windows[0].label, "5h");
    assert_eq!(saved.account.quota_windows[0].remaining_percent, 20.0);
    assert_eq!(saved.account.quota_windows[1].label, "7d");
    assert_eq!(saved.account.quota_windows[1].remaining_percent, 75.0);
    assert!(saved.account.quota_windows[1].reset_at.is_some());
    assert!(saved.account.subscription_end_at.is_some());
    apply_quota(&mut saved.account, &json!({}), &json!({})).unwrap();
    assert!(saved.account.balance_usd.is_none());
    assert!(saved.account.quota_windows.is_empty());
}

#[test]
fn successful_summary_recognizes_free_and_preserves_returned_tiers() {
    let mut saved = account_from_profile(&profile("one")).unwrap();
    for summary in [
        json!({}),
        json!({"membershipTier":null}),
        json!({"membershipTier":"  "}),
    ] {
        saved.account.plan = Some("pro".into());
        apply_quota(&mut saved.account, &json!({}), &summary).unwrap();
        assert_eq!(saved.account.plan.as_deref(), Some("free"));
        assert_eq!(saved.account.balance_usd, None);
    }
    for tier in ["free", "go", "plus", "pro", "max", "team"] {
        apply_quota(
            &mut saved.account,
            &json!({}),
            &json!({"membershipTier":format!(" {tier} ")}),
        )
        .unwrap();
        assert_eq!(saved.account.plan.as_deref(), Some(tier));
    }
}

#[test]
fn invalid_summary_preserves_cached_plan_and_quota() {
    let mut saved = account_from_profile(&profile("one")).unwrap();
    saved.account.balance_usd = Some(2.5);
    let before = serde_json::to_value(&saved.account).unwrap();
    for summary in [Value::Null, json!([]), json!("invalid")] {
        assert_eq!(
            apply_quota(&mut saved.account, &json!({}), &summary).unwrap_err(),
            "accountError.authResponse"
        );
        assert_eq!(serde_json::to_value(&saved.account).unwrap(), before);
    }
}

#[test]
fn cancelled_replaced_and_expired_logins_cannot_be_saved() {
    let pending = Some(Pending {
        id: "current".into(),
        device_id: "device".into(),
        secret: "private".into(),
        expires: 100,
    });
    assert!(current_login(&pending, "current", 99).is_ok());
    assert!(current_login(&pending, "current", 100).is_err());
    assert!(current_login(&pending, "previous", 99).is_err());
    assert!(current_login(&None, "current", 99).is_err());
    assert!(authorization_url(
        "https://open-design.ai/cloud/cli/activate?code=fixture"
    ));
    for url in [
        "http://open-design.ai/cloud/cli/activate",
        "https://open-design.ai.attacker.test/cloud/cli/activate",
        "https://attacker.test/cloud/cli/activate",
        "https://open-design.ai:444/cloud/cli/activate",
        "https://user@open-design.ai/cloud/cli/activate",
    ] {
        assert!(!authorization_url(url));
    }
}

#[test]
fn importing_native_keys_preserves_cached_allowance_without_querying_upstream() {
    let mut rows = Store::new();
    let mut saved = account_from_profile(&profile("one")).unwrap();
    saved.account.balance_usd = Some(3.0);
    remember(&mut rows, saved);
    let next = account_from_profile(&profile("one")).unwrap();
    let account = remember(&mut rows, next);
    assert_eq!(rows.len(), 1);
    assert_eq!(account.balance_usd, Some(3.0));
}
