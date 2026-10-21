use super::{api, credential, load_at, path, protect, request_lock, Credentials, STORE_LOCK};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct GatewayToken {
    pub token: String,
    pub expires_at: i64,
}
impl GatewayToken {
    fn valid(&self) -> bool {
        !self.token.is_empty() && self.expires_at > chrono::Utc::now().timestamp() + 60
    }
    fn credentials(&self, id: &str) -> Value {
        json!({"accountId":id,"accessToken":self.token,"expiresAt":self.expires_at})
    }
}

pub(crate) async fn proxy_credentials(id: &str, force_refresh: bool) -> Result<Value, String> {
    prepare(&path()?, id, force_refresh, None, None).await
}
pub(crate) async fn proxy_credentials_after_rejection(
    id: &str,
    rejected: &Value,
) -> Result<Value, String> {
    let token = rejected["accessToken"]
        .as_str()
        .ok_or("accountError.invalidAccount")?;
    prepare(&path()?, id, true, Some(token), None).await
}
async fn prepare(
    store_path: &Path,
    id: &str,
    force: bool,
    rejected: Option<&str>,
    client: Option<&api::Client>,
) -> Result<Value, String> {
    // Serialize preparation, not inference. Other accounts and already-started
    // streams do not wait for this account's token exchange.
    let lock = request_lock(id)?;
    let _request = lock.lock().await;
    let mut c: Credentials = {
        let _store = STORE_LOCK.lock().await;
        credential(&load_at(store_path)?, id)?
    };
    if c.enterprise() {
        return Err("accountError.unavailable".into());
    }
    if let Some(token) = c.gateway_token.as_ref().filter(|token| token.valid()) {
        if !force || rejected.is_some_and(|rejected| rejected != token.token) {
            return Ok(token.credentials(id));
        }
    }
    let owned;
    let client = match client {
        Some(client) => client,
        None => {
            owned = api::Client::authenticated(&c)?;
            &owned
        }
    };
    let token = client.gateway_token(&c).await?;
    if rejected == Some(token.token.as_str()) {
        return Err("accountError.loginRequired".into());
    }
    let result = token.credentials(id);
    c.gateway_token = Some(token);
    let _store = STORE_LOCK.lock().await;
    let mut store = load_at(store_path)?;
    let current = credential(&store, id)?;
    if current.profile != c.profile || current.cookies != c.cookies {
        return Err("accountError.invalidAccount".into());
    }
    store
        .get_mut(id)
        .ok_or("accountError.invalidAccount")?
        .credential = protect(&c)?;
    super::super::cursor_auth::write(store_path, &store)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::{Saved, Store};
    use super::*;
    use axum::{routing::get, Json, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn encrypted_token_cache_coalesces_concurrent_preparation_and_rejected_token_waves() {
        let root = std::env::temp_dir().join(format!("dumate-proxy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("accounts.json");
        let c = super::super::tests::fixture();
        let id = c.id();
        let store = Store::from([(
            id.clone(),
            Saved {
                account: c.summary(),
                credential: protect(&c).unwrap(),
            },
        )]);
        super::super::super::cursor_auth::write(&path, &store).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let hits = count.clone();
        let app=Router::new().route("/api/dumate/app/token/get",get(move |headers:axum::http::HeaderMap| {let hits=hits.clone();async move {
            assert!(headers["cookie"].to_str().unwrap().contains("fixture-secret"));
            let n=hits.fetch_add(1,Ordering::SeqCst)+1;
            Json(json!({"code":0,"result":{"token":format!("fixture-gateway-{n}"),"expired_at":(chrono::Utc::now()+chrono::Duration::hours(2)).to_rfc3339()}}))
        }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client =
            api::Client::new(format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (a, b) = tokio::join!(
            prepare(&path, &id, false, None, Some(&client)),
            prepare(&path, &id, false, None, Some(&client))
        );
        let a = a.unwrap();
        assert_eq!(a, b.unwrap());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let rejected = a["accessToken"].as_str().unwrap();
        let (a, b) = tokio::join!(
            prepare(&path, &id, true, Some(rejected), Some(&client)),
            prepare(&path, &id, true, Some(rejected), Some(&client))
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let bytes = std::fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains("fixture-gateway"));
        assert!(!bytes.contains("fixture-secret"));
        let mut store = load_at(&path).unwrap();
        let mut expired = credential(&store, &id).unwrap();
        expired.gateway_token.as_mut().unwrap().expires_at = 0;
        store.get_mut(&id).unwrap().credential = protect(&expired).unwrap();
        super::super::super::cursor_auth::write(&path, &store).unwrap();
        prepare(&path, &id, false, None, Some(&client))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        std::fs::remove_file(&path).unwrap();
        assert!(prepare(&path, &id, false, None, Some(&client))
            .await
            .is_err());
        assert_eq!(count.load(Ordering::SeqCst), 3);
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }
}
