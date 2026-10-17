use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use std::fs;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("echobird-kimi-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.0.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn saved(region: Region) -> Saved {
    let account = profile(
        &json!({"user_id":"fixture-user", "nickname":"Kimi user", "user_level_name":"Plus"}),
        region,
    )
    .unwrap();
    let jwt = URL_SAFE_NO_PAD.encode(br#"{"user_id":"fixture-user"}"#);
    Saved {
        account,
        token: Token {
            access_token: format!("header.{jwt}.signature"),
            refresh_token: "fixture-refresh".into(),
            expires_at: 2_000_000_000.0,
            expires_in: 3600.0,
            scope: "code".into(),
            token_type: "Bearer".into(),
        },
    }
}
fn models() -> Value {
    json!({"data":[{"id":"fixture-model-a","protocol":"anthropic","limit":{"context":200000},"reasoning":{"type":"only"},"modalities":{"input":["text","image"]}},{"id":"fixture-model-b","protocol":"openai","limit":{"context":64000}}]})
}

#[test]
fn regions_have_separate_identity_and_native_credential_slots() {
    assert_ne!(
        saved(Region::Cn).account.id,
        saved(Region::Global).account.id
    );
    assert_eq!(Region::Cn.key(), "oauth/kimi-code");
    let scope = r#"{"oauthHost":"https://auth.kimi.ai","baseUrl":"https://api.kimi.ai/coding/v1"}"#;
    assert_eq!(
        Region::Global.key(),
        format!(
            "oauth/kimi-code-env-{}",
            &format!("{:x}", Sha256::digest(scope))[..16]
        )
    );
    assert!(serde_json::from_value::<Region>(json!("other")).is_err());
}

#[test]
fn profile_preserves_five_tiers_and_unknown_is_not_free() {
    for plan in ["Free", "Plus", "Pro", "Max", "Ultra"] {
        let account = profile(&json!({"user_id":"u", "email":"kimi@example.test", "nickname":"ignored", "user_level_name":plan.to_lowercase()}), Region::Cn).unwrap();
        assert_eq!(account.email, "kimi@example.test");
        assert_eq!(account.plan.as_deref(), Some(plan));
    }
    assert!(profile(&json!({"user_id":"u"}), Region::Cn)
        .unwrap()
        .plan
        .is_none());
    assert!(profile(&json!({"nickname":"name"}), Region::Cn).is_err());
}

#[test]
fn quota_uses_only_supplied_windows_remaining_direction_and_real_reset() {
    let mut account = saved(Region::Cn).account;
    quota(&mut account, &json!({"usages":{"limit_5h":{"used_ratio":0.25,"reset_time":"2030-01-01T00:00:00Z"},"limit_7d":{"used_ratio":0},"limit_month_total":{"used_ratio":1.2},"limit_month_code":{"used_ratio":-0.2}}})).unwrap();
    assert_eq!(account.quota_windows.len(), 4);
    assert_eq!(
        account
            .quota_windows
            .iter()
            .map(|w| w.remaining_percent)
            .collect::<Vec<_>>(),
        [75.0, 100.0, 0.0, 100.0]
    );
    assert_eq!(account.quota_windows[0].reset_at, Some(1_893_456_000));
    assert_eq!(account.quota_windows[1].reset_at, None);
    assert!(account.wallet.is_none());
    quota(
        &mut account,
        &json!({"usages":{"limit_5h":{"used_ratio":0.5},"limit_7d":{}}}),
    )
    .unwrap();
    assert_eq!(account.quota_windows.len(), 1);
    assert!(quota(&mut account, &json!({"error":"unavailable"})).is_err());
}

#[test]
fn wallet_keeps_native_units_and_does_not_invent_a_missing_balance() {
    let mut account = saved(Region::Cn).account;
    quota(&mut account, &json!({"boosterWallet":{"balance":{"type":"BOOSTER","amount":2000000000_i64,"amountLeft":1350000000_i64},"monthlyChargeLimit":{"currency":"CNY"}}})).unwrap();
    let wallet = account.wallet.as_ref().unwrap();
    assert_eq!(wallet.remaining, 13.5);
    assert_eq!(wallet.total, 20.0);
    assert_eq!(wallet.currency.as_deref(), Some("CNY"));
    quota(&mut account, &json!({"usages":{},"boosterWallet":{"balance":{"type":"BOOSTER","amount":2000000000_i64}}})).unwrap();
    assert!(account.wallet.is_none());
}

#[test]
fn quota_accepts_numeric_strings_without_fabricating_invalid_values() {
    let mut account = saved(Region::Cn).account;
    quota(&mut account, &json!({"usages":{"limit_5h":{"used_ratio":"0.25"},"limit_7d":{"used_ratio":"bad"}},"boosterWallet":{"balance":{"type":"BOOSTER","amount":"2000000000","amountLeft":"1350000000"}}})).unwrap();
    assert_eq!(account.quota_windows.len(), 1);
    assert_eq!(account.quota_windows[0].remaining_percent, 75.0);
    assert_eq!(account.wallet.as_ref().unwrap().remaining, 13.5);
    for invalid in [
        json!(""),
        json!("NaN"),
        json!("inf"),
        json!(false),
        Value::Null,
    ] {
        quota(&mut account, &json!({"usages":{"limit_5h":{"used_ratio":invalid}},"boosterWallet":{"balance":{"type":"BOOSTER","amount":"2000000000","amountLeft":invalid}}})).unwrap();
        assert!(account.quota_windows.is_empty());
        assert!(account.wallet.is_none());
    }
}

#[test]
#[cfg(windows)]
fn failed_native_config_write_restores_existing_or_missing_credentials() {
    for existing in [false, true] {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.toml");
        let original = "# retain config\ndefault_model = 'personal'\n";
        fs::write(&path, original).unwrap();
        let credential_path = fixture.0.join("credentials/kimi-code.json");
        let old = json!({"original":"saved-native-token"});
        if existing {
            super::super::cursor_auth::write(&credential_path, &old).unwrap();
        }
        let permissions = fs::metadata(&path).unwrap().permissions();
        let mut read_only = permissions.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&path, read_only).unwrap();
        let result = native::apply_at(&fixture.0, &saved(Region::Cn), &models());
        let actual: Option<Value> = super::super::cursor_auth::read(&credential_path).ok();
        fs::set_permissions(&path, permissions).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(actual, existing.then_some(old));
    }
}

#[test]
fn empty_successful_quota_is_absent_for_any_plan_and_clears_previous_usage() {
    for region in [Region::Cn, Region::Global] {
        for plan in ["Free", "Plus", "Pro", "Max", "Ultra"] {
            for payload in [json!({}), json!({"usages":null})] {
                let mut account = saved(region).account;
                account.plan = Some(plan.into());
                quota(&mut account, &json!({"usages":{"limit_5h":{"used_ratio":0.25}},"boosterWallet":{"balance":{"type":"BOOSTER","amount":2000000000_i64,"amountLeft":1350000000_i64}}})).unwrap();
                assert!(!account.quota_windows.is_empty());
                assert!(account.wallet.is_some());
                quota(&mut account, &payload).unwrap();
                assert!(account.quota_windows.is_empty());
                assert!(account.wallet.is_none());
                assert_eq!(account.plan.as_deref(), Some(plan));
            }
        }
    }
}

#[test]
fn quota_errors_are_not_treated_as_empty_usage_or_clear_cached_data() {
    for payload in [
        json!({"error":"unavailable"}),
        json!({"error":"unavailable","usages":{}}),
        json!({"usages":[]}),
        json!({"usages":false}),
        json!([]),
        Value::Null,
    ] {
        let mut account = saved(Region::Cn).account;
        quota(
            &mut account,
            &json!({"usages":{"limit_5h":{"used_ratio":0.25}}}),
        )
        .unwrap();
        assert!(quota(&mut account, &payload).is_err());
        assert_eq!(account.quota_windows[0].remaining_percent, 75.0);
    }
}

#[test]
fn native_apply_preserves_other_config_and_uses_the_same_account_for_both_clients() {
    let fixture = Fixture::new();
    let path = fixture.0.join("config.toml");
    fs::write(&path, "# keep this comment\ndefault_model = 'custom'\n[providers.personal]\ntype = 'openai'\napi_key = 'custom-secret'\n[models.custom]\nprovider = 'personal'\nmodel = 'personal-model'\n[thinking]\nenabled = true\n[permission]\nmode = 'ask'\n").unwrap();
    for region in [Region::Cn, Region::Global] {
        let row = saved(region);
        native::apply_at(&fixture.0, &row, &models()).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# keep this comment"));
        let doc: toml_edit::DocumentMut = raw.parse().unwrap();
        assert_eq!(
            doc["providers"]["personal"]["api_key"].as_str(),
            Some("custom-secret")
        );
        assert_eq!(doc["thinking"]["enabled"].as_bool(), Some(true));
        assert_eq!(doc["permission"]["mode"].as_str(), Some("ask"));
        assert_eq!(
            doc["providers"]["managed:kimi-code"]["base_url"].as_str(),
            Some(region.base())
        );
        assert_eq!(
            doc["models"]["kimi-code/fixture-model-a"]["beta_api"].as_bool(),
            Some(true)
        );
        assert!(!raw.contains("fixture-refresh"));
        assert_eq!(native::active_id(&fixture.0), Some(row.account.id.clone()));
        assert!(native::token_for(&fixture.0, &row.account).is_some());
    }
    assert!(fixture.0.join("credentials/kimi-code.json").exists());
    let mut other = saved(Region::Global).account;
    other.id = "different-account".into();
    assert!(native::token_for(&fixture.0, &other).is_none());
}

#[test]
fn corrupt_native_config_or_invalid_catalog_never_changes_credentials() {
    for raw in [
        "[invalid",
        "models = 12",
        "[providers]\n'managed:kimi-code' = 'invalid'",
    ] {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.toml");
        fs::write(&path, raw).unwrap();
        assert!(native::apply_at(&fixture.0, &saved(Region::Cn), &models()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), raw);
        assert!(!fixture.0.join("credentials").exists());
    }
    let fixture = Fixture::new();
    assert!(native::apply_at(&fixture.0, &saved(Region::Cn), &json!({"data":[]})).is_err());
    assert!(!fixture.0.join("credentials").exists());
}

#[test]
fn account_ipc_never_contains_credentials() {
    let wire = serde_json::to_string(&saved(Region::Cn).account).unwrap();
    assert!(!wire.contains("refresh_token"));
    assert!(!wire.contains("access_token"));
    assert!(!wire.contains("fixture-refresh"));
}
