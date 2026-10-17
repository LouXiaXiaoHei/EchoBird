use super::*;

#[tokio::test]
#[ignore = "Explicit daily-welfare diagnosis: native credentials plus a single read-only GET, no claims or writes"]
async fn reads_live_daily_welfare_status_without_claiming() {
    let native = native::read().unwrap().expect("Desktop must be logged in");
    let account = account(&native).unwrap();
    let credentials = credentials(&Saved { account, native }).unwrap();
    let response = signed_json(
        "GET",
        &format!("{POLICY}/v1/ops/delivery?channel=DESKTOP"),
        &credentials,
        "",
    )
    .await
    .unwrap();
    let fields = |value: &Value| {
        value
            .as_object()
            .map(|object| object.keys().cloned().collect::<Vec<_>>())
    };
    let items: Vec<_> = response["data"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let mut summary = json!({"fields":fields(item)});
            for key in [
                "type",
                "claimed",
                "claimable",
                "disabled",
                "status",
                "claimButtonText",
                "claimButtonTip",
                "benefitAmount",
                "title",
            ] {
                if let Some(value) = item.get(key) {
                    summary[key] = value.clone();
                }
            }
            summary
        })
        .collect();
    println!(
        "{}",
        json!({"responseFields":fields(&response),"code":response["code"],"dataFields":fields(&response["data"]),"items":items,"parsed":daily_campaign(&response).map(|id|id.is_none()),"httpMethod":"GET","writes":0})
    );
    assert!(daily_campaign(&response).is_ok());
}

#[test]
fn welfare_accepts_a_data_envelope_without_an_optional_code_but_rejects_explicit_errors() {
    assert_eq!(
        welfare_data(&json!({"data":{"id":"fixture"}})).unwrap(),
        &json!({"id":"fixture"})
    );
    assert!(welfare_data(&json!({"code":403,"data":{"message":"denied"}})).is_err());
    assert!(welfare_data(&json!({})).is_err());
}

fn context() -> Context {
    let (private, public) = key_pair().unwrap();
    Context {
        private,
        public,
        verifier: "fixture-pkce-verifier".into(),
        redirect_uri: "http://127.0.0.1:12345/oauth/callback".into(),
    }
}
fn response() -> Value {
    json!({"credentials":{"access_key_id":"fixture-ak","secret_access_key":"fixture-sk","security_token":"fixture-token","expiration":"2099-01-01T00:00:00Z"},"refresh_token":"fixture-refresh"})
}
fn user() -> Value {
    json!({"principal_id":"fixture-user","account_id":"fixture-domain","principal_urn":"iam::fixture-domain:user/name"})
}

#[test]
fn desktop_login_preserves_oauth_context_and_native_shape() {
    let context = context();
    let native = native_from_login(&context, &response(), &user()).unwrap();
    let account = account(&native).unwrap();
    assert_eq!(account.tool_id, "codearts");
    assert!(account.id.starts_with("codearts:"));
    assert_eq!(account.email, "name");
    assert!(account.balance.is_none());
    assert!(account.plan.is_none());
    let saved = Saved { account, native };
    let credentials = credentials(&saved).unwrap();
    assert_eq!(credentials["accessKey"], "fixture-ak");
    assert_eq!(credentials["secretKey"], "fixture-sk");
    assert_eq!(credentials["securityToken"], "fixture-token");
    let (restored, refresh) = refresh_context(&saved).unwrap();
    assert_eq!(refresh, "fixture-refresh");
    assert_eq!(restored.verifier, context.verifier);
    assert_eq!(restored.private, context.private);
}

#[test]
fn newer_native_credentials_only_update_the_same_account_and_preserve_public_metadata() {
    let native = native_from_login(&context(), &response(), &user()).unwrap();
    let mut saved = Saved {
        account: account(&native).unwrap(),
        native: native.clone(),
    };
    saved.account.plan = Some("Free".into());
    saved.account.balance = Some(6498.62);
    saved.account.total = Some(6500.0);
    saved.account.subscription_end_at = Some(1234);
    saved.account.daily_claimed_at = Some(5678);
    let metadata = serde_json::to_value(&saved.account).unwrap();
    let mut newer = native.clone();
    newer["session"]["expiresAt"] =
        json!(native["session"]["expiresAt"].as_i64().unwrap() + 3600000);
    newer["session"]["accessKey"] = json!("fixture-new-ak");
    newer["session"]["securityToken"] = json!("fixture-new-token");
    newer["session"]["refreshToken"] = json!("fixture-rotated-refresh");

    let mut other = newer.clone();
    other["session"]["userId"] = json!("fixture-other-user");
    assert!(!update_native_session(&mut saved, &other).unwrap());
    let mut malformed = newer.clone();
    malformed["session"]["secretKey"] = Value::Null;
    assert!(update_native_session(&mut saved, &malformed).is_err());
    assert_eq!(saved.native, native);

    assert!(update_native_session(&mut saved, &newer).unwrap());
    assert_eq!(saved.native, newer);
    assert_eq!(serde_json::to_value(&saved.account).unwrap(), metadata);
    assert_eq!(
        refresh_context(&saved).unwrap().1,
        "fixture-rotated-refresh"
    );
    assert!(!update_native_session(&mut saved, &native).unwrap());
    assert!(!update_native_session(&mut saved, &newer).unwrap());
    assert_eq!(saved.native, newer);
}

#[test]
fn saved_account_list_survives_missing_unreadable_or_invalid_native_state() {
    let native = native_from_login(&context(), &response(), &user()).unwrap();
    let mut saved = Saved {
        account: account(&native).unwrap(),
        native: native.clone(),
    };
    saved.account.plan = Some("Free".into());
    saved.account.balance = Some(6498.62);
    saved.account.total = Some(6500.0);
    saved.account.subscription_end_at = Some(1234);
    saved.account.daily_claimed_at = Some(5678);
    // A stale persisted flag must not mark a failed native probe as active.
    saved.account.active = true;
    let rows = BTreeMap::from([(saved.account.id.clone(), saved.clone())]);
    for native in [
        Ok(None),
        Err("accountError.read".into()),
        Err("accountError.keychain".into()),
        Err("accountError.format".into()),
        Ok(Some(json!({"session":{}}))),
    ] {
        let listed = account_list(rows.clone(), native);
        let mut expected = saved.account.clone();
        expected.active = false;
        assert_eq!(serde_json::to_value(listed).unwrap(), json!([expected]));
    }
    let listed = account_list(rows.clone(), Ok(Some(native.clone())));
    assert_eq!(
        serde_json::to_value(listed).unwrap(),
        json!([saved.account])
    );
    let mut other = native;
    other["session"]["userId"] = json!("another-user");
    assert!(!account_list(rows, Ok(Some(other)))[0].active);
}

#[test]
fn saving_and_deleting_desktop_rows_preserves_legacy_records_and_unknown_metadata() {
    let desktop = json!({"account":{"id":"codearts:fixture","toolId":"codearts","email":"old","futureAccountField":{"preserved":true}},"session":"enc:v1:old","futureRecordField":[1,2]});
    let ide = json!({"account":{"id":"codeartsagent:legacy","toolId":"codeartsagent"},"session":"opaque-legacy","futureField":17});
    let cli = json!({"account":{"id":"codeartscli:legacy","toolId":"codeartscli"},"session":"opaque-cli"});
    let original: BTreeMap<String, Value> = BTreeMap::from([
        ("codearts:fixture".into(), desktop),
        ("codeartsagent:legacy".into(), ide.clone()),
        ("codeartscli:legacy".into(), cli.clone()),
        ("unknown".into(), json!("unrecognized record")),
    ]);
    let listed: Vec<_> = original
        .iter()
        .filter(|(id, row)| desktop_record(id, row))
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(listed, vec!["codearts:fixture"]);
    let updated = json!({"account":{"id":"codearts:fixture","toolId":"codearts","email":"new"},"session":"enc:v1:new"});
    let saved = merge_store(
        original,
        BTreeMap::from([("codearts:fixture".into(), updated)]),
    )
    .unwrap();
    assert_eq!(
        saved["codearts:fixture"]["account"]["futureAccountField"],
        json!({"preserved":true})
    );
    assert_eq!(
        saved["codearts:fixture"]["futureRecordField"],
        json!([1, 2])
    );
    assert_eq!(saved["codearts:fixture"]["account"]["email"], "new");
    assert_eq!(saved["codeartsagent:legacy"], ide);
    assert_eq!(saved["codeartscli:legacy"], cli);
    let deleted = merge_store(saved, BTreeMap::new()).unwrap();
    assert!(!deleted.contains_key("codearts:fixture"));
    assert_eq!(deleted["codeartsagent:legacy"], ide);
    assert_eq!(deleted["codeartscli:legacy"], cli);
    assert_eq!(deleted["unknown"], json!("unrecognized record"));
}

#[tokio::test]
async fn legacy_product_ids_are_rejected_before_storage_network_or_native_operations() {
    for id in [
        "codeartsagent:legacy",
        "codeartscli:legacy",
        "other:legacy",
        "",
    ] {
        assert_eq!(
            proxy_credentials(id, false).await.err().unwrap(),
            "accountError.invalidAccount"
        );
        assert_eq!(
            proxy_credentials_after_rejection(id, &Value::Null)
                .await
                .err()
                .unwrap(),
            "accountError.invalidAccount"
        );
        assert_eq!(
            refresh_account(id).await.err().unwrap(),
            "accountError.invalidAccount"
        );
        assert_eq!(
            claim_credits(id).await.err().unwrap(),
            "accountError.invalidAccount"
        );
        assert_eq!(
            switch_account(id).await.err().unwrap(),
            "accountError.invalidAccount"
        );
        assert_eq!(
            delete_account(id).await.err().unwrap(),
            "accountError.invalidAccount"
        );
    }
    let native = native_from_login(&context(), &response(), &user()).unwrap();
    let mut account = account(&native).unwrap();
    account.tool_id = "codeartscli".into();
    let saved = Saved { account, native };
    assert!(credentials(&saved).is_err());
    assert!(native::apply(&saved).is_err());
}

#[test]
fn dpop_signature_verifies_with_its_public_jwk_and_binds_method_and_token_endpoint() {
    let context = context();
    let url = format!("{STS}/v1/oauth2/tokens");
    let proof = dpop(&context, &url).unwrap();
    let parts: Vec<_> = proof.split('.').collect();
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    let payload: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(header["alg"], "ES256");
    assert_eq!(header["typ"], "dpop+jwt");
    assert!(header["jwk"].get("d").is_none());
    assert_eq!(payload["htm"], "POST");
    assert_eq!(payload["htu"], url);
    let mut public = vec![4];
    public.extend(
        URL_SAFE_NO_PAD
            .decode(context.public["x"].as_str().unwrap())
            .unwrap(),
    );
    public.extend(
        URL_SAFE_NO_PAD
            .decode(context.public["y"].as_str().unwrap())
            .unwrap(),
    );
    let input = format!("{}.{}", parts[0], parts[1]);
    let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    let verifier =
        ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, &public);
    verifier.verify(input.as_bytes(), &signature).unwrap();
    assert!(verifier.verify(b"modified", &signature).is_err());
}

#[test]
fn quota_maps_trial_credit_packages_to_free_and_preserves_paid_tiers_and_credits() {
    for (package, plan) in [
        ("Trial", "Free"),
        ("trial", "Free"),
        ("Free", "Free"),
        ("Basic", "Basic"),
        ("Pro", "Pro"),
        ("EnterprisePro", "EnterprisePro"),
        ("EnterpriseUltimate", "EnterpriseUltimate"),
    ] {
        let value = json!({"package":{"package_name_en":package},"metrics":[{"name":"usageBasicPackageCredit","package_credit_remain":999},{"name":"usageTotalPackageCredit","package_credit_amount":6500,"package_credit_used":1.38,"package_credit_remain":"6498.62"}]});
        assert_eq!(
            package_summary(&value).unwrap(),
            (Some(plan.into()), Some(6498.62))
        );
        assert_eq!(package_total(&value), Some(6500.0));
    }
    assert_eq!(
        package_summary(&json!({"package":{},"metrics":[]})).unwrap(),
        (None, None)
    );
    assert_eq!(
        package_summary(
            &json!({"metrics":[{"name":"usageTotalPackageCredit","package_credit_remain":0}]})
        )
        .unwrap()
        .1,
        Some(0.0)
    );
    assert!(package_summary(&json!({"error_code":"failed"})).is_err());
    assert!(package_summary(
        &json!({"metrics":[{"name":"usageTotalPackageCredit","package_credit_remain":-1}]})
    )
    .unwrap()
    .1
    .is_none());
}

#[test]
fn credit_total_comes_from_the_same_official_bucket_as_remaining_credits() {
    let response = json!({"metrics":[
        {"name":"usageBonusPackageCredit","package_credit_amount":6000,"package_credit_remain":5998.62},
        {"name":"usageTotalPackageCredit","package_credit_amount":6500,"package_credit_used":1.38,"package_credit_remain":6498.62}
    ]});
    assert_eq!(package_summary(&response).unwrap().1, Some(6498.62));
    assert_eq!(package_total(&response), Some(6500.0));
    assert_eq!(
        package_total(
            &json!({"metrics":[{"name":"usageTotalPackageCredit","package_credit_amount":"6500.00"}]})
        ),
        Some(6500.0)
    );
    assert_eq!(
        package_total(
            &json!({"metrics":[{"name":"usageTotalPackageCredit","package_credit_amount":0}]})
        ),
        Some(0.0)
    );
    assert!(package_total(&json!({"metrics":[]})).is_none());
    assert!(package_total(
        &json!({"metrics":[{"name":"usageTotalPackageCredit","package_credit_amount":-1}]})
    )
    .is_none());
}

#[test]
fn subscription_expiry_uses_the_paid_package_end_date_and_keeps_old_caches_compatible() {
    let response = json!({"package":{"package_name_en":"Pro"},"end_date":"2026-11-09"});
    let plan = package_summary(&response).unwrap().0;
    let expected = chrono::DateTime::parse_from_rfc3339("2026-11-09T00:00:00+08:00")
        .unwrap()
        .timestamp();
    assert_eq!(
        package_subscription_end(&response, plan.as_deref()),
        Some(expected)
    );
    for end in ["2026-11-09T00:00:00+08:00", "2026-11-08T16:00:00Z"] {
        assert_eq!(
            package_subscription_end(&json!({"end_date":end}), Some("Pro")),
            Some(expected)
        );
    }
    for package in ["Free", "Trial"] {
        let response = json!({"package":{"package_name_en":package},"end_date":"2026-11-09"});
        let plan = package_summary(&response).unwrap().0;
        assert_eq!(package_subscription_end(&response, plan.as_deref()), None);
    }
    assert_eq!(package_subscription_end(&response, None), None);
    for end in [
        Value::Null,
        json!(""),
        json!("invalid"),
        json!("2026-02-30"),
        json!("9999-12-31"),
        json!(0),
    ] {
        assert_eq!(
            package_subscription_end(&json!({"end_date":end}), Some("Pro")),
            None
        );
    }
    assert_eq!(
        package_subscription_end(
            &json!({"session":{"expiresAt":expected},"metrics":[{"end_date":"2026-11-09"}]}),
            Some("Pro")
        ),
        None
    );
    let old = json!({"id":"codearts:fixture","email":"fixture","toolId":"codearts","plan":"Pro","balance":6498.62,"total":6500,"active":false});
    let mut cached: Account = serde_json::from_value(old).unwrap();
    assert_eq!(cached.subscription_end_at, None);
    cached.subscription_end_at = Some(expected);
    let saved = serde_json::to_value(cached).unwrap();
    assert_eq!(saved["subscriptionEndAt"], expected);
    assert_eq!(
        serde_json::from_value::<Account>(saved)
            .unwrap()
            .subscription_end_at,
        Some(expected)
    );
}

#[test]
fn daily_welfare_filters_campaign_type_and_respects_server_claim_state() {
    let delivery = |items| json!({"code":0,"data":{"items":items}});
    assert_eq!(daily_campaign(&delivery(json!([{"type":"invite","id":"wrong"},{"type":"daily_claim","campaignId":"daily","claimable":true}]))).unwrap(),Some(json!("daily")));
    for status in ["CLAIMED", "CONFIRMED", "CONSUMED"] {
        assert_eq!(
            daily_campaign(&delivery(
                json!([{"type":"DAILY_CLAIM","id":"daily","status":status}])
            ))
            .unwrap(),
            None
        );
    }
    assert!(daily_campaign(&delivery(json!([{"type":"invite","id":"wrong"}]))).is_err());
    assert!(daily_campaign(&delivery(
        json!([{"type":"daily_claim","claimable":false,"id":"disabled"}])
    ))
    .is_err());
    assert!(daily_campaign(&delivery(json!([{"type":"daily_claim"}]))).is_err());
    assert!(daily_campaign(&json!({"code":403,"data":{"items":[]}})).is_err());
}

#[tokio::test]
async fn daily_welfare_accepts_official_user_login_type_and_does_not_reclaim_claimed_points() {
    let (result, requests) = welfare_fixture(vec![json!({"code":0,"data":{"channel":"DESKTOP","items":[
        {"type":"NEW_USER_REGISTER","campaignId":"registration","status":"CLAIMED","claimable":false},
        {"type":"USER_LOGIN","campaignId":"daily-login","status":"CLAIMED","claimable":false,"benefitAmount":1000}
    ]}})]).await;
    result.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "GET /v1/ops/delivery?channel=DESKTOP");

    let (result, requests) = welfare_fixture(vec![
        json!({"code":0,"data":{"items":[{"type":"NEW_USER_REGISTER","campaignId":"wrong","claimable":true},{"type":"USER_LOGIN","campaignId":"daily-login","status":null,"claimable":true}]}}),
        json!({"code":0,"data":{"userBenefitId":"daily-benefit"}}),
        json!({"code":0,"data":{}}),
    ]).await;
    result.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].1["campaignId"], "daily-login");
}

async fn welfare_fixture(replies: Vec<Value>) -> (Result<(), String>, Vec<(String, Value)>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let replies = Arc::new(Mutex::new(std::collections::VecDeque::from(replies)));
    let router = axum::Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri, body: bytes::Bytes| {
            let captured = captured.clone();
            let replies = replies.clone();
            async move {
                let body = if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&body).unwrap()
                };
                captured
                    .lock()
                    .unwrap()
                    .push((format!("{method} {uri}"), body));
                let reply = replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| json!({"code":500}));
                axum::Json(reply)
            }
        },
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let credentials =
        json!({"accessKey":"fixture-ak","secretKey":"fixture-sk","securityToken":"fixture-token"});
    let result = claim_welfare(&credentials, &base).await;
    server.abort();
    let requests = requests.lock().unwrap().clone();
    (result, requests)
}

#[tokio::test]
async fn daily_welfare_http_contract_uses_source_channel_and_confirms_when_requested() {
    {
        let channel = "DESKTOP";
        let (result,requests)=welfare_fixture(vec![
            json!({"code":0,"data":{"items":[{"type":"invite","id":"wrong"},{"type":"daily_claim","id":"daily-123"}]}}),
            json!({"code":0,"data":{"userBenefitId":"benefit-123"}}),
            json!({"code":200,"data":{}}),
        ]).await;
        result.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests[0].0,
            format!("GET /v1/ops/delivery?channel={channel}")
        );
        assert_eq!(requests[1].0, "POST /v1/ops/claim");
        assert_eq!(requests[1].1["campaignId"], "daily-123");
        assert_eq!(requests[1].1["channel"], channel);
        assert!(requests[1].1["idempotentKey"]
            .as_str()
            .unwrap()
            .starts_with("claim_daily-123_"));
        assert_eq!(
            requests[2],
            (
                "POST /v1/ops/confirm".into(),
                json!({"userBenefitId":"benefit-123"})
            )
        );
    }
    let (result, requests) = welfare_fixture(vec![
        json!({"code":0,"data":{"items":[{"type":"daily_claim","status":"CONFIRMED"}]}}),
    ])
    .await;
    result.unwrap();
    assert_eq!(requests.len(), 1);
    let delivery = json!({"code":0,"data":{"items":[{"type":"daily_claim","id":"daily"}]}});
    for data in [json!({}), json!({"id":null})] {
        let (result, requests) =
            welfare_fixture(vec![delivery.clone(), json!({"code":0,"data":data})]).await;
        result.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].0, "POST /v1/ops/claim");
    }
    for replies in [
        vec![json!({"code":0,"data":{"items":[]}})],
        vec![delivery.clone(), json!({"code":403})],
        vec![
            delivery,
            json!({"code":0,"data":{"id":"benefit"}}),
            json!({"code":500}),
        ],
    ] {
        let count = replies.len();
        let (result, requests) = welfare_fixture(replies).await;
        assert!(result.is_err());
        assert_eq!(requests.len(), count);
    }
}

#[tokio::test]
async fn replacement_login_rejects_old_cancellation_and_empty_callbacks_do_not_consume_it() {
    let first = start_login().await.unwrap();
    let replacement = start_login().await.unwrap();
    let url = url::Url::parse(&replacement.verification_uri).unwrap();
    let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "codearts");
    assert!(replacement.expires_at <= now() + 60);
    cancel_login(&first.login_id).unwrap();
    assert!(poll_login(&first.login_id).await.is_err());
    assert!(poll_login(&replacement.login_id).await.unwrap().is_none());
    let callback_url = LOGIN
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .context
        .redirect_uri
        .clone();
    assert_eq!(
        client()
            .unwrap()
            .get(&callback_url)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert!(!LOGIN
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .callback_used
        .load(Ordering::Acquire));
    assert!(poll_login(&replacement.login_id).await.unwrap().is_none());
    assert_eq!(
        client()
            .unwrap()
            .get(&callback_url)
            .query(&[("error", "access_denied")])
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert!(LOGIN
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .callback_used
        .load(Ordering::Acquire));
    assert!(poll_login(&replacement.login_id).await.is_err());
    cancel_login(&replacement.login_id).unwrap();
}

#[test]
#[ignore = "Reads the installed CodeArts desktop credential store; no network or native writes"]
fn reads_native_desktop_session_without_network_or_writes() {
    let native = native::read()
        .unwrap_or_else(|error| panic!("codearts native read: {error}"))
        .expect("codearts native session is missing");
    let account = account(&native).unwrap();
    assert_eq!(account.tool_id, "codearts");
    let credentials = credentials(&Saved { account, native }).unwrap();
    assert!(credentials["accessKey"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
    assert!(credentials["secretKey"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
}

#[tokio::test]
#[ignore = "Explicit live verification: CodeArts desktop native credentials and identity/credit GET requests only"]
async fn verifies_desktop_live_identity_and_credits() {
    let native = native::read()
        .unwrap()
        .expect("CodeArts desktop must be logged in");
    let account = account(&native).unwrap();
    let credentials = credentials(&Saved { account, native }).unwrap();
    let identity = signed_json(
        "GET",
        &format!("{STS}/v5/caller-identity"),
        &credentials,
        "",
    )
    .await
    .unwrap();
    assert!(identity["account_id"].is_string());
    assert!(identity["principal_id"].is_string());
    let package = signed_json(
        "GET",
        &format!("{POLICY}/snap-manager/v1/statistics/plugin"),
        &credentials,
        "",
    )
    .await
    .unwrap();
    let (plan, balance) = package_summary(&package).unwrap();
    println!(
        "{}",
        json!({"product":"codearts","plan":plan,"credits":balance,"httpMethods":["GET"],"nativeWrites":0})
    );
}

#[tokio::test]
#[ignore = "Explicit credit diagnosis: read saved/native desktop credentials and GET credit metrics without writes"]
async fn inspects_saved_and_native_credit_metrics_read_only() {
    let native = native::read().unwrap().expect("Desktop must be logged in");
    let active = account(&native).unwrap();
    let mut rows = vec![(
        "native",
        Saved {
            account: active.clone(),
            native,
        },
    )];
    rows.extend(load().unwrap().into_values().map(|saved| ("saved", saved)));
    let mut canonical_identity = None;
    let mut cloud_identity = None;
    for (source, saved) in rows {
        let credentials = credentials(&saved).unwrap();
        let identity = signed_json(
            "GET",
            &format!("{STS}/v5/caller-identity"),
            &credentials,
            "",
        )
        .await
        .unwrap();
        let user = signed_json(
            "GET",
            &format!("{POLICY}/snap-manager/v1/current/user"),
            &credentials,
            "",
        )
        .await
        .unwrap();
        let cloud = (
            identity["account_id"].clone(),
            identity["principal_id"].clone(),
        );
        let canonical = (user["domain_id"].clone(), user["user_id"].clone());
        if source == "native" {
            canonical_identity = Some(canonical.clone());
            cloud_identity = Some(cloud.clone());
        }
        let package = signed_json(
            "GET",
            &format!("{POLICY}/snap-manager/v1/statistics/plugin"),
            &credentials,
            "",
        )
        .await
        .unwrap();
        let metrics: Vec<_> = package["metrics"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|metric| {
                let mut output = json!({"name":metric["name"]});
                if let Some(object) = metric.as_object() {
                    for (key, value) in object {
                        if key.starts_with("package_credit") || key.starts_with("usage_credit") {
                            output[key] = value.clone();
                        }
                    }
                }
                output
            })
            .collect();
        println!(
            "{}",
            json!({"source":source,"sameAsNativeIdentity":saved.account.id==active.id,"sameCloudIdentity":cloud_identity.as_ref()==Some(&cloud),"sameCanonicalIdentity":canonical_identity.as_ref()==Some(&canonical),"cachedPlan":saved.account.plan,"cachedBalance":saved.account.balance,"parsed":package_summary(&package),"packageEndDate":package["end_date"],"subscriptionEndAt":package_subscription_end(&package,package_summary(&package).unwrap().0.as_deref()),"metrics":metrics,"writes":0})
        );
    }
}
