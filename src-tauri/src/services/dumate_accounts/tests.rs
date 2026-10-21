use super::*;
use serde_json::json;

pub(super) fn fixture() -> Credentials {
    Credentials {
        gateway_token: None,
        profile: json!({"profileId":"fixture-profile","loginType":"bce","bceUserId":"fixture-user",
        "bceAccountId":"fixture-account","displayName":"Fixture"}),
        cookies: vec![
            json!({"name":"bce-long-term-sessionid","value":"fixture-secret","domain":"127.0.0.1","path":"/"}),
        ],
    }
}
#[test]
fn stable_identity_is_not_profile_uuid_and_public_summary_has_no_credentials() {
    let a = fixture();
    let mut b = a.clone();
    b.profile["profileId"] = json!("different-profile");
    assert_eq!(a.id(), b.id());
    b.profile["bceAccountId"] = json!("other-account");
    assert_ne!(a.id(), b.id());
    let summary = serde_json::to_string(&a.summary()).unwrap();
    assert!(!summary.contains("secret"));
    assert!(!summary.contains("fixture-profile"));
    assert!(a.summary().remaining.is_none());
    b.profile["loginType"] = json!("enterprise");
    assert!(b.validate().is_err());
    b.profile["enterpriseApiBaseUrl"] = json!("https://enterprise.example.test");
    assert!(b.validate().is_ok());
    assert_ne!(a.id(), b.id());
}
#[tokio::test]
async fn requests_are_serialized_per_account_without_blocking_other_accounts() {
    let a = request_lock("a").unwrap();
    let guard = a.lock().await;
    assert!(request_lock("a").unwrap().try_lock().is_err());
    assert!(request_lock("b").unwrap().try_lock().is_ok());
    drop(guard);
}
#[test]
fn cached_rows_can_be_read_without_decrypting_credentials() {
    let a = fixture().summary();
    let value = serde_json::to_vec(&BTreeMap::from([(
        a.id.clone(),
        Saved {
            account: a,
            credential: "unreadable".into(),
        },
    )]))
    .unwrap();
    let store: Store = serde_json::from_slice(&value).unwrap();
    assert_eq!(store.values().next().unwrap().account.name, "Fixture");
    assert!(credential(&store, store.keys().next().unwrap()).is_err());
}
