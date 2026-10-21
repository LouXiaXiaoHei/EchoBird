use super::{Account, Credentials, CreditBucket, DailyReward, Edition};
use serde_json::{json, Value};
use std::time::Duration;

pub(super) fn text<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| v[*key].as_str().filter(|s| !s.trim().is_empty()))
}
pub(super) fn timestamp(v: &Value) -> Option<i64> {
    let n = v
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp())
        .or_else(|| v.as_i64().or_else(|| v.as_str()?.parse().ok()))?;
    let n = if n >= 1_000_000_000_000 { n / 1000 } else { n };
    (n > 0 && n < 253_402_214_400).then_some(n)
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}
fn alias<'a>(v: &'a Value, a: &str, b: &str) -> &'a Value {
    v.get(a).or_else(|| v.get(b)).unwrap_or(&Value::Null)
}
fn data(v: &Value) -> &Value {
    v.get("data").filter(|v| v.is_object()).unwrap_or(v)
}
fn context(v: &Value) -> Value {
    let root = data(v);
    let mut result = root.clone();
    if let Some(user) = root["user"].as_object() {
        if let Some(result) = result.as_object_mut() {
            result.extend(user.clone());
        }
    }
    result
}

pub(super) struct Client {
    http: reqwest::Client,
    pub(super) edition: Edition,
    pub(super) base: String,
    version: String,
    release: String,
}

impl Client {
    pub(super) fn installed(edition: Edition) -> Result<Self, String> {
        let exe = crate::services::tool_manager::get_tool_exe_path(edition.tool())
            .ok_or("accountError.unavailable")?;
        let metadata =
            crate::services::tool_manager::packaged_desktop_metadata(std::path::Path::new(&exe))
                .ok_or("accountError.unavailable")?;
        if metadata["name"] != edition.package() {
            return Err("accountError.invalidAccount".into());
        }
        Self::new(
            edition,
            edition.base(),
            text(&metadata, &["qoderworkUpgradeVersion", "version"]).unwrap_or("unknown"),
        )
    }
    pub(super) fn new(edition: Edition, base: &str, version: &str) -> Result<Self, String> {
        Ok(Self {
            edition,
            base: base.into(),
            version: version.split('-').next().unwrap_or(version).into(),
            release: version.into(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|_| "accountError.network")?,
        })
    }
    pub(super) async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value, String> {
        let platform = match std::env::consts::OS {
            "windows" => "win32",
            "macos" => "darwin",
            other => other,
        };
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        let mut url = reqwest::Url::parse(&format!("{}{path}", self.base))
            .map_err(|_| "accountError.format")?;
        if path.starts_with("/api/v1/adapter/user/account-context")
            || path == "/api/v1/deviceToken/refresh"
        {
            use base64::Engine;
            let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(
                    &json!({"v":1,"user_agent":format!("EchoBird/{}",env!("CARGO_PKG_VERSION"))}),
                )
                .map_err(|_| "accountError.format")?,
            );
            url.query_pairs_mut().append_pair("state", &state);
        }
        let mut request = self
            .http
            .request(method, url)
            .header("Accept", "application/json")
            .header(
                "User-Agent",
                if path.starts_with("/sash/") {
                    "QoderWork".to_owned()
                } else {
                    format!("qoderwork/{}", self.version)
                },
            )
            .header("X-Request-Id", uuid::Uuid::new_v4().to_string())
            .header("X-QwenWork-Version", &self.version)
            .header("X-QwenWork-Release-Version", &self.release)
            .header(
                "X-QwenWork-Build",
                self.release.split('-').nth(1).unwrap_or("unknown"),
            )
            .header("X-QwenWork-Platform", platform)
            .header("X-QwenWork-Arch", arch)
            .header("X-QwenWork-Channel", "stable");
        if self.edition == Edition::QwenWorkCn {
            request = request.header("X-QwenWork-App-Mode", "standard");
        }
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if path.starts_with("/sash/") {
            request = request.header("Content-Type", "application/json");
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|_| "accountError.network")?;
        if response.status().as_u16() == 404 && path.starts_with("/api/v1/deviceToken/poll?") {
            return Ok(Value::Null);
        }
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err("accountError.loginRequired".into());
        }
        if !response.status().is_success() {
            log::warn!(
                "QwenWork request {} returned HTTP {}",
                path.split('?').next().unwrap_or(""),
                response.status()
            );
            return Err("accountError.network".into());
        }
        response
            .json()
            .await
            .map_err(|_| "accountError.format".into())
    }
    pub(super) async fn user(&self, token: &str) -> Result<Value, String> {
        let v = self
            .request(
                reqwest::Method::GET,
                "/api/v1/adapter/user/account-context?include=user,plan,quota,page,data_sharing",
                Some(token),
                None,
            )
            .await?;
        normalize_user(&v)
    }
    pub(super) async fn refresh(&self, c: &Credentials) -> Result<Credentials, String> {
        let user = &c.auth["user"];
        let mut body = json!({"refresh_token": c.auth["refreshToken"], "target": if user["isBiz"] == true { "biz" } else { "c" },
            "client_id":self.edition.client_id(),"redirect_uri":self.edition.redirect()});
        if user["isBiz"] == true {
            body["team_id"] = user["teamId"].clone();
        }
        let v = self
            .request(
                reqwest::Method::POST,
                "/api/v1/deviceToken/refresh",
                None,
                Some(&body),
            )
            .await?;
        let v = data(&v);
        let mut next = c.clone();
        next.auth["token"] =
            json!(text(v, &["device_token", "token"]).ok_or("accountError.format")?);
        next.auth["refreshToken"] =
            json!(text(v, &["refresh_token"]).ok_or("accountError.format")?);
        next.auth["expiresAt"] = json!(expiry(v, "expires_at", "expires_in")?);
        if let Ok(expires) = expiry(v, "refresh_token_expires_at", "refresh_token_expires_in") {
            next.auth["refreshTokenExpiresAt"] = json!(expires);
        }
        next.validate()?;
        Ok(next)
    }
    pub(super) async fn quota(&self, c: &Credentials) -> Result<Quota, String> {
        let env = self
            .request(
                reqwest::Method::GET,
                "/api/v1/adapter/user/account-context?include=user,plan,quota,page,data_sharing",
                Some(token(c)?),
                None,
            )
            .await?;
        let user = normalize_user(&env)?;
        let mut current = c.clone();
        current.auth["user"] = user.clone();
        if current.id() != c.id() {
            return Err("accountError.invalidAccount".into());
        }
        let enterprise = user["isBiz"] == true;
        let quota = if enterprise {
            self.request(
                reqwest::Method::GET,
                "/api/v2/quota/usage",
                Some(token(c)?),
                None,
            )
            .await?
        } else {
            env.clone()
        };
        let mut result = parse_quota(&env, &quota, enterprise)?;
        if !enterprise && self.edition == Edition::QwenWorkCn {
            if let Ok(wallets) = self
                .request(
                    reqwest::Method::GET,
                    "/api/v1/adapter/user/wallets",
                    Some(token(c)?),
                    None,
                )
                .await
            {
                let wallets = data(&wallets);
                let buckets: Vec<_> = [
                    ("daily_credits", "Daily"),
                    ("monthly_credits", "Monthly"),
                    ("longterm_credits", "Long-term"),
                ]
                .into_iter()
                .map(|(key, name)| CreditBucket {
                    name: name.into(),
                    remaining: number(alias(&wallets[key], "total_balance", "totalBalance")),
                    total: result
                        .buckets
                        .iter()
                        .find(|bucket| bucket.name == name)
                        .and_then(|bucket| bucket.total),
                    expires_at: result
                        .buckets
                        .iter()
                        .find(|bucket| bucket.name == name)
                        .and_then(|bucket| bucket.expires_at),
                })
                .collect();
                if buckets.iter().all(|b| b.remaining.is_some()) {
                    result.buckets = buckets;
                }
            }
        }
        Ok(result)
    }
    pub(super) async fn daily(&self, c: &Credentials) -> Result<DailyReward, String> {
        if self.edition != Edition::QwenWorkCn {
            return Err("accountError.unavailable".into());
        }
        let v = self
            .request(
                reqwest::Method::GET,
                "/sash/api/v1/me/daily-check-in/status",
                Some(token(c)?),
                None,
            )
            .await?;
        parse_daily(&v)
    }
    pub(super) async fn claim(
        &self,
        c: &Credentials,
        mut daily: DailyReward,
    ) -> Result<DailyReward, String> {
        if self.edition != Edition::QwenWorkCn {
            return Err("accountError.unavailable".into());
        }
        if daily.status != "CLAIMABLE" {
            return Ok(daily);
        }
        let v = self
            .request(
                reqwest::Method::POST,
                "/sash/api/v1/me/daily-check-in/claim",
                Some(token(c)?),
                None,
            )
            .await?;
        let v = data(&v);
        if !matches!(v["result"].as_str(), Some("CLAIMED" | "ALREADY_CLAIMED")) {
            return Err("accountError.format".into());
        }
        daily.status = "CLAIMED_TODAY".into();
        daily.checked_at = chrono::Utc::now().timestamp();
        daily.reward_credits = number(&v["rewardCredits"]).or(daily.reward_credits);
        daily.expires_at = timestamp(&v["expiresAt"]).or(daily.expires_at);
        Ok(daily)
    }
}

pub(super) fn token(c: &Credentials) -> Result<&str, String> {
    c.auth["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "accountError.invalidAccount".into())
}
pub(super) fn expiry(v: &Value, absolute: &str, duration: &str) -> Result<String, String> {
    let seconds = timestamp(&v[absolute])
        .or_else(|| {
            v[duration]
                .as_i64()
                .filter(|n| *n > 0)
                .and_then(|n| chrono::Utc::now().timestamp().checked_add(n))
        })
        .or_else(|| {
            (absolute == "expires_at" && v[absolute].is_null() && v[duration].is_null())
                .then(|| chrono::Utc::now().timestamp() + 3600)
        })
        .ok_or("accountError.format")?;
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .ok_or_else(|| "accountError.format".into())
}
pub(super) fn normalize_user(env: &Value) -> Result<Value, String> {
    let v = context(env);
    let id = text(&v, &["id", "user_id", "uid"]).ok_or("accountError.format")?;
    let org = &v["organization"];
    let is_biz = alias(&v, "is_biz", "isBiz").as_bool().or_else(|| {
        text(&v, &["org_type", "orgType"])
            .or_else(|| text(org, &["org_type", "orgType", "type"]))
            .map(|t| t.eq_ignore_ascii_case("enterprise"))
    });
    let plan = &v["plan"];
    let detail = &v["detail"];
    let third_party: Vec<_> = alias(&v, "third_party_identities", "thirdPartyIdentities").as_array().into_iter().flatten()
        .filter_map(|identity| Some(json!({"provider":text(identity,&["provider"])? ,"openId":text(identity,&["open_id","openId"])?}))).collect();
    let team = text(&v, &["team_id", "teamId"]).or_else(|| text(org, &["team_id", "teamId"]));
    if is_biz == Some(true) && team.is_none() {
        return Err("accountError.format".into());
    }
    Ok(
        json!({"id":id,"name":text(&v,&["name","username","user_name"]),"email":text(&v,&["email"]).unwrap_or(""),
        "imageUrl":text(&v,&["avatar_url","avatar","image_url"]),"username":text(&v,&["username","user_name","name"]),
        "tier":text(plan,&["name"]).or_else(||text(&v,&["tier","subscription_tier"])),
        "planName":text(plan,&["name"]),"planId":text(plan,&["pid"]),"planNextDueDate":text(plan,&["next_due_date","nextDueDate"]),
        "planSubscriptionActive":alias(plan,"subscription_status","subscriptionStatus").as_bool().unwrap_or(false),
        "isBiz":is_biz,"orgId":text(&v,&["orgId","organization_id","organizationId"]).or_else(||text(org,&["org_id","orgId","id"])),
        "teamId":team,"orgName":text(org,&["org_name","orgName","name"]),"orgLogoUrl":text(org,&["logo_url","logoUrl","logo","avatar_url","avatar"]),
        "orgType":text(&v,&["org_type","orgType"]).or_else(||text(org,&["org_type","orgType","type"])),
        "orgTags":alias(&v,"organization_tags","organizationTags"),"thirdPartyIdentities":third_party,
        "active":v["active"].as_bool(),"detail":{"dingtalkUid":text(detail,&["dingtalk_uid","dingtalkUid"]),"orgRole":text(detail,&["org_role","orgRole"]),"teamRole":text(org,&["role"])}}),
    )
}

pub(super) struct Quota {
    plan: Option<String>,
    plan_name: Option<String>,
    expires: Option<i64>,
    remaining: Option<f64>,
    total: Option<f64>,
    buckets: Vec<CreditBucket>,
}
impl Quota {
    pub(super) fn apply(self, account: &mut Account) {
        account.plan = self.plan;
        account.plan_name = self.plan_name;
        account.expires_at = self.expires;
        account.remaining = self.remaining;
        account.total = self.total;
        account.credit_buckets = self.buckets;
    }
}
fn bucket(v: &Value, name: &str) -> Option<CreditBucket> {
    if !v.is_object() {
        return None;
    }
    let total = number(alias(v, "total", "cap"));
    let remaining =
        number(&v["remaining"]).or_else(|| Some((total? - number(&v["used"])?).max(0.0)));
    (remaining.is_some() || total.is_some()).then(|| CreditBucket {
        name: name.into(),
        remaining,
        total,
        expires_at: timestamp(alias(v, "expires_at", "expiresAt")),
    })
}
fn parse_quota(env: &Value, value: &Value, enterprise: bool) -> Result<Quota, String> {
    let env = context(env);
    let v = if enterprise {
        data(value).clone()
    } else {
        env["quota"].clone()
    };
    if !v.is_object() {
        return Err("accountError.format".into());
    }
    let base = alias(&v, "user_quota", "userQuota");
    let primary = bucket(
        if base.is_object() { base } else { &v },
        if enterprise { "Member" } else { "Credits" },
    );
    let mut buckets = vec![];
    if !enterprise {
        let daily = v
            .get("daily_quota")
            .or_else(|| v.get("dailyQuota"))
            .or_else(|| v.get("daily_credits"))
            .or_else(|| v.get("dailyCredits"))
            .unwrap_or(&Value::Null);
        let daily_limit = number(alias(&v, "daily_refresh_amount", "dailyRefreshAmount"))
            .or_else(|| number(alias(daily, "refresh_amount", "refreshAmount")));
        for (a, b, name) in [
            ("daily_quota", "dailyQuota", "Daily"),
            ("monthly_quota", "monthlyQuota", "Monthly"),
            ("other_quota", "otherQuota", "Long-term"),
        ] {
            let mut row = bucket(
                if name == "Daily" {
                    daily
                } else {
                    alias(&v, a, b)
                },
                name,
            );
            if name == "Daily" {
                if row.is_none() && daily_limit.is_some() {
                    row = Some(CreditBucket {
                        name: name.into(),
                        remaining: None,
                        total: daily_limit,
                        expires_at: None,
                    });
                }
                if let Some(row) = row.as_mut() {
                    row.total = row.total.or(daily_limit);
                }
            }
            if let Some(row) = row {
                buckets.push(row);
            }
        }
    }
    if buckets.is_empty() {
        if let Some(row) = &primary {
            buckets.push(row.clone());
        }
    }
    if let Some(row) = bucket(alias(&v, "add_on_quota", "addOnQuota"), "Add-on") {
        buckets.push(row);
    }
    if enterprise {
        if let Some(row) = bucket(&v["org"], "Organization") {
            buckets.push(row);
        }
    } else if let Some(row) = bucket(
        alias(&v, "org_resource_package", "orgResourcePackage"),
        "Organization",
    ) {
        buckets.push(row);
    }
    Ok(Quota {
        plan: text(&env["plan"], &["pid"]).map(str::to_owned),
        plan_name: text(&env["plan"], &["name"]).map(str::to_owned),
        expires: timestamp(alias(&env["plan"], "next_due_date", "nextDueDate")),
        remaining: number(&v["remaining"]).or_else(|| primary.as_ref().and_then(|p| p.remaining)),
        total: if number(&v["remaining"]).is_some() {
            number(&v["total"])
        } else {
            primary.as_ref().and_then(|p| p.total)
        },
        buckets,
    })
}
fn parse_daily(v: &Value) -> Result<DailyReward, String> {
    let v = data(v);
    let status = text(v, &["status"])
        .filter(|s| {
            matches!(
                *s,
                "CLAIMABLE" | "CLAIMED_TODAY" | "DISABLED" | "NOT_STARTED" | "ENDED"
            )
        })
        .ok_or("accountError.format")?;
    Ok(DailyReward {
        status: status.into(),
        reward_credits: number(&v["rewardCredits"]),
        checked_at: chrono::Utc::now().timestamp(),
        next_claim_at: timestamp(&v["nextClaimAt"]),
        expires_at: timestamp(&v["rewardExpiresAt"]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        routing::{get, post},
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn account_context() -> Value {
        json!({"data":{"user":{"id":"fixture-user","name":"Fixture","is_biz":false},"plan":{"pid":"qwen-office-pro","name":"Pro","next_due_date":"2035-01-01T00:00:00Z"},"quota":{"remaining":70,"user_quota":{"total":100,"used":30}}}})
    }

    #[test]
    fn unknown_quota_is_never_zero_and_organization_pool_is_not_added_to_member_allowance() {
        let env = account_context();
        let mut quota = parse_quota(&env, &env, false).unwrap();
        let mut account = super::super::tests::fixture(Edition::QwenWork).summary();
        quota.apply(&mut account);
        assert_eq!(account.remaining, Some(70.0));
        let empty = json!({"user":{"id":"fixture-user","is_biz":false},"quota":{},"plan":{"pid":"new-plan","name":"Future"}});
        quota = parse_quota(&empty, &empty, false).unwrap();
        quota.apply(&mut account);
        assert_eq!(account.remaining, None);
        assert_eq!(account.total, None);
        assert!(account.credit_buckets.is_empty());
        assert_eq!(account.plan.as_deref(), Some("new-plan"));
        let enterprise = json!({"user_id":"fixture-user","user_quota":{"total":100,"used":20},"org":{"remaining":15},"add_on_quota":{"remaining":0}});
        parse_quota(&env, &enterprise, true)
            .unwrap()
            .apply(&mut account);
        assert_eq!(account.remaining, Some(80.0));
        assert_eq!(account.credit_buckets[1].remaining, Some(0.0));
        assert_eq!(account.credit_buckets[2].remaining, Some(15.0));
        assert_eq!(timestamp(&json!(253_402_214_400i64)), None);
        assert_eq!(timestamp(&json!("9999-12-31T00:00:00Z")), None);
    }

    #[tokio::test]
    async fn wallet_balances_keep_the_real_daily_limit_and_never_use_a_subpool_as_total_capacity() {
        for has_daily_limit in [true, false] {
            let mut context = json!({"data":{"user":{"id":"fixture-user","is_biz":false},"plan":{"pid":"free"},"quota":{"remaining":2099.7501,"user_quota":{"total":2000,"remaining":2000}}}});
            if has_daily_limit {
                context["data"]["quota"]["daily_refresh_amount"] = json!(100);
            }
            let app=Router::new()
                .route("/api/v1/adapter/user/account-context",get(move||{let value=context.clone();async move{Json(value)}}))
                .route("/api/v1/adapter/user/wallets",get(||async{Json(json!({"data":{"daily_credits":{"total_balance":99.7501},"monthly_credits":{"total_balance":0},"longterm_credits":{"total_balance":2000}}}))}));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let client = Client::new(Edition::QwenWorkCn, &base, "fixture").unwrap();
            let c = super::super::tests::fixture(Edition::QwenWorkCn);
            let quota = client.quota(&c).await.unwrap();
            let mut account = c.summary();
            quota.apply(&mut account);
            assert_eq!(account.remaining, Some(2099.7501));
            let daily = account
                .credit_buckets
                .iter()
                .find(|bucket| bucket.name == "Daily")
                .unwrap();
            assert_eq!(
                (account.total, daily.total),
                (None, has_daily_limit.then_some(100.0))
            );
            assert_eq!(daily.remaining, Some(99.7501));
            server.abort();
        }
    }

    #[tokio::test]
    async fn refresh_and_check_in_have_separate_real_http_operations_and_preserve_native_identity()
    {
        let claims = Arc::new(AtomicUsize::new(0));
        let hits = claims.clone();
        let app=Router::new()
            .route("/api/v1/adapter/user/account-context",get(||async{Json(account_context())}))
            .route("/api/v1/adapter/user/wallets",get(||async{Json(json!({"data":{"daily_credits":{"total_balance":0},"monthly_credits":{"total_balance":70},"longterm_credits":{"total_balance":0}}}))}))
            .route("/api/v1/deviceToken/refresh",post(|Json(v):Json<Value>|async move{
                assert_eq!(v["refresh_token"],"fixture-refresh-secret");assert_eq!(v["target"],"c");assert!(v.get("team_id").is_none());
                Json(json!({"device_token":"next-access","refresh_token":"next-refresh","expires_in":3600}))
            }))
            .route("/sash/api/v1/me/daily-check-in/status",get(|headers:axum::http::HeaderMap|async move{
                assert_eq!(headers["authorization"],"Bearer fixture-access-secret");
                Json(json!({"data":{"status":"CLAIMABLE","rewardCredits":127,"nextClaimAt":2_000_000_000_000i64}}))
            }))
            .route("/sash/api/v1/me/daily-check-in/claim",post(move|body:axum::body::Bytes|{let hits=hits.clone();async move{
                assert!(body.is_empty());hits.fetch_add(1,Ordering::SeqCst);Json(json!({"data":{"result":"CLAIMED","rewardCredits":127}}))
            }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = Client::new(Edition::QwenWorkCn, &base, "fixture").unwrap();
        let c = super::super::tests::fixture(Edition::QwenWorkCn);
        let rotated = client.refresh(&c).await.unwrap();
        assert_eq!(rotated.auth["token"], "next-access");
        assert_eq!(rotated.id(), c.id());
        assert_eq!(rotated.auth["loginDeviceId"], c.auth["loginDeviceId"]);
        client.quota(&c).await.unwrap();
        let daily = client.daily(&c).await.unwrap();
        assert_eq!(daily.reward_credits, Some(127.0));
        assert_eq!(daily.next_claim_at, Some(2_000_000_000));
        assert_eq!(claims.load(Ordering::SeqCst), 0);
        let daily = client.claim(&c, daily).await.unwrap();
        assert_eq!(daily.status, "CLAIMED_TODAY");
        assert_eq!(claims.load(Ordering::SeqCst), 1);
        client.claim(&c, daily).await.unwrap();
        assert_eq!(claims.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[test]
    fn disabled_and_malformed_daily_status_never_become_claimable() {
        for status in ["DISABLED", "NOT_STARTED", "ENDED", "CLAIMED_TODAY"] {
            assert_eq!(
                parse_daily(&json!({"data":{"status":status}}))
                    .unwrap()
                    .status,
                status
            );
        }
        assert!(parse_daily(&json!({"result":"CLAIMED"})).is_err());
        assert!(parse_daily(&json!({"status":"future-state"})).is_err());
        assert!(normalize_user(&json!({"user":{"id":"fixture","is_biz":true}})).is_err());
    }

    #[tokio::test]
    async fn international_accounts_never_query_or_claim_rewards_with_cached_claimable_status() {
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let app = Router::new().fallback(move || {
            let requests = requests.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                Json(json!({"data":{"status":"CLAIMABLE","result":"CLAIMED","rewardCredits":127}}))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let global = Client::new(Edition::QwenWork, &base, "fixture").unwrap();
        let c = super::super::tests::fixture(Edition::QwenWork);
        let cached = parse_daily(&json!({"status":"CLAIMABLE","rewardCredits":200})).unwrap();
        assert!(matches!(global.daily(&c).await,Err(e) if e=="accountError.unavailable"));
        assert!(matches!(global.claim(&c,cached).await,Err(e) if e=="accountError.unavailable"));
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let domestic = Client::new(Edition::QwenWorkCn, &base, "fixture").unwrap();
        let c = super::super::tests::fixture(Edition::QwenWorkCn);
        let daily = domestic.daily(&c).await.unwrap();
        domestic.claim(&c, daily).await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        server.abort();
    }
}
