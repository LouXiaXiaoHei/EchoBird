use super::{api, Credentials, Edition};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    login_id: String,
    verification_uri: String,
    expires_at: i64,
}
static PENDING: Mutex<Option<Arc<Attempt>>> = Mutex::new(None);
struct Attempt {
    id: String,
    edition: Edition,
    verifier: String,
    challenge: String,
    nonce: String,
    machine: String,
    expires: i64,
    cancel: CancellationToken,
    outcome: Mutex<Option<Result<Credentials, String>>>,
}
pub(super) async fn start(edition: Edition) -> Result<LoginStart, String> {
    let client = api::Client::installed(edition)?;
    let directory = super::native::data_dir(edition)?;
    if !directory.is_dir() || (cfg!(windows) && !directory.join("Local State").is_file()) {
        return Err("accountError.initializeClient".into());
    }
    let mut random = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut random);
    let verifier = URL_SAFE_NO_PAD.encode(random);
    let a = Arc::new(Attempt {
        id: uuid::Uuid::new_v4().to_string(),
        edition,
        challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(&verifier)),
        verifier,
        nonce: uuid::Uuid::new_v4().to_string(),
        machine: super::native::machine_id(edition)?,
        expires: chrono::Utc::now().timestamp() + 60,
        cancel: CancellationToken::new(),
        outcome: Mutex::new(None),
    });
    let result = LoginStart {
        login_id: a.id.clone(),
        verification_uri: login_url(&a)?,
        expires_at: a.expires,
    };
    if let Some(previous) = PENDING
        .lock()
        .map_err(|_| "accountError.read")?
        .replace(a.clone())
    {
        previous.cancel.cancel();
    }
    tokio::spawn(async move {
        let result = tokio::select! {
            _=a.cancel.cancelled()=>Err("accountError.cancelled".into()),
            _=tokio::time::sleep(Duration::from_secs((a.expires-chrono::Utc::now().timestamp()).max(0) as u64))=>Err("accountError.expired".into()),
            result=authorize(&a,&client)=>result,
        };
        if let Ok(mut outcome) = a.outcome.lock() {
            *outcome = Some(result);
        }
    });
    Ok(result)
}
fn login_url(a: &Attempt) -> Result<String, String> {
    let mut url = url::Url::parse(&format!("{}/device/selectAccounts", a.edition.base()))
        .map_err(|_| "accountError.format")?;
    url.query_pairs_mut()
        .append_pair("challenge", &a.challenge)
        .append_pair("challenge_method", "S256")
        .append_pair("nonce", &a.nonce)
        .append_pair("machine_id", &a.machine)
        .append_pair("client_id", a.edition.client_id())
        .append_pair("redirect_uri", a.edition.redirect());
    Ok(url.into())
}
fn check(a: &Attempt) -> Result<(), String> {
    if a.cancel.is_cancelled() {
        return Err("accountError.cancelled".into());
    }
    if chrono::Utc::now().timestamp() >= a.expires {
        return Err("accountError.expired".into());
    }
    Ok(())
}
pub(super) fn poll(id: &str) -> Result<Option<Credentials>, String> {
    let pending = PENDING.lock().map_err(|_| "accountError.read")?;
    let a = pending
        .as_ref()
        .filter(|a| a.id == id)
        .ok_or("accountError.cancelled")?;
    check(a)?;
    let outcome = a.outcome.lock().map_err(|_| "accountError.read")?.clone();
    outcome.transpose()
}
pub(super) fn complete<T>(id: &str, save: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let mut pending = PENDING.lock().map_err(|_| "accountError.read")?;
    let a = pending
        .as_ref()
        .filter(|a| a.id == id)
        .ok_or("accountError.cancelled")?;
    check(a)?;
    let value = save()?;
    pending.take();
    Ok(value)
}
pub(super) fn cancel(id: &str) -> Result<(), String> {
    let mut pending = PENDING.lock().map_err(|_| "accountError.read")?;
    if pending.as_ref().is_some_and(|a| a.id == id) {
        if let Some(a) = pending.take() {
            a.cancel.cancel();
        }
    }
    Ok(())
}
async fn authorize(a: &Attempt, client: &api::Client) -> Result<Credentials, String> {
    let mut url = url::Url::parse("https://localhost/").map_err(|_| "accountError.format")?;
    url.query_pairs_mut()
        .append_pair("nonce", &a.nonce)
        .append_pair("verifier", &a.verifier)
        .append_pair("challenge_method", "S256");
    let path = format!("/api/v1/deviceToken/poll?{}", url.query().unwrap_or(""));
    let mut failures = 0;
    loop {
        check(a)?;
        match client
            .request(reqwest::Method::GET, &path, None, None)
            .await
        {
            Ok(v) if api::text(&v, &["token"]).is_some() => return finish(a, client, &v).await,
            Err(e) if e == "accountError.loginRequired" => return Err(e),
            Err(e) => {
                failures += 1;
                if failures >= 5 {
                    return Err(e);
                }
            }
            Ok(_) => {
                failures = 0;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
async fn finish(
    a: &Attempt,
    client: &api::Client,
    v: &serde_json::Value,
) -> Result<Credentials, String> {
    if v["nonce"] != a.nonce
        || v["code_challenge"] != a.challenge
        || v["code_challenge_method"] != "S256"
    {
        return Err("accountError.invalidAccount".into());
    }
    let token = api::text(v, &["token"]).ok_or("accountError.format")?;
    let refresh = api::text(v, &["refresh_token"]).ok_or("accountError.format")?;
    let user = client.user(token).await?;
    let c = Credentials {
        edition: a.edition,
        auth: json!({"schemaVersion":2,"authRealm":"standard","token":token,"refreshToken":refresh,
        "expiresAt":api::expiry(v,"expires_at","expires_in")?,
        "refreshTokenExpiresAt":api::expiry(v,"refresh_token_expires_at","refresh_token_expires_in").ok(),
        "user":user,"identityVersion":0,"loginMethod":"browser","refreshStrategy":"device_token",
        "loginDeviceId":a.machine,"loginTimestamp":chrono::Utc::now().timestamp_millis(),
        "oauthClientId":a.edition.client_id(),"oauthRedirectUri":a.edition.redirect()}),
    };
    c.validate()?;
    check(a)?;
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn attempt(edition: Edition) -> Attempt {
        Attempt {
            id: uuid::Uuid::new_v4().to_string(),
            edition,
            verifier: "fixture-verifier".into(),
            challenge: "fixture-challenge".into(),
            nonce: "fixture-nonce".into(),
            machine: "fixture-machine".into(),
            expires: chrono::Utc::now().timestamp() + 60,
            cancel: CancellationToken::new(),
            outcome: Mutex::new(None),
        }
    }
    #[test]
    fn browser_authorization_is_bound_to_region_nonce_challenge_and_registered_client() {
        for edition in [Edition::QwenWork, Edition::QwenWorkCn] {
            let a = attempt(edition);
            let url = url::Url::parse(&login_url(&a).unwrap()).unwrap();
            assert_eq!(url.origin().ascii_serialization(), edition.base());
            let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
            assert_eq!(query["client_id"], edition.client_id());
            assert_eq!(query["redirect_uri"], edition.redirect());
            assert_eq!(query["nonce"], a.nonce);
        }
    }
    #[tokio::test]
    async fn bound_login_succeeds_but_mismatched_and_cancelled_attempts_cannot_save_credentials() {
        use axum::{routing::get, Json, Router};
        let app=Router::new().route("/api/v1/adapter/user/account-context",get(||async{Json(json!({"data":{"user":{"id":"fixture-user","is_biz":false},"plan":{"pid":"qwen-office-plus","name":"Plus"}}}))}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let a = attempt(Edition::QwenWorkCn);
        let client = api::Client::new(a.edition, &base, "fixture").unwrap();
        let mut value = json!({"nonce":a.nonce,"code_challenge":a.challenge,"code_challenge_method":"S256","token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600});
        let c = finish(&a, &client, &value).await.unwrap();
        assert_eq!(c.auth["user"]["id"], "fixture-user");
        assert_eq!(c.edition, Edition::QwenWorkCn);
        value["code_challenge"] = json!("wrong");
        assert!(finish(&a, &client, &value).await.is_err());
        let a = Arc::new(a);
        let id = a.id.clone();
        *PENDING.lock().unwrap() = Some(a);
        cancel(&id).unwrap();
        let mut written = false;
        let result = complete(&id, || {
            written = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!written);
        server.abort();
    }
}
