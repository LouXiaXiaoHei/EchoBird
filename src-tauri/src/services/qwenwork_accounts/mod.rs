mod api;
mod native;
mod oauth;
pub use oauth::LoginStart;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

static STORE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    QwenWork,
    QwenWorkCn,
}

impl Edition {
    pub(crate) fn tool(self) -> &'static str {
        match self {
            Self::QwenWork => "qwenwork",
            Self::QwenWorkCn => "qwenworkcn",
        }
    }
    fn data_name(self) -> &'static str {
        match self {
            Self::QwenWork => "QwenWork",
            Self::QwenWorkCn => "QwenWorkCN",
        }
    }
    fn package(self) -> &'static str {
        match self {
            Self::QwenWork => "qwen-work",
            Self::QwenWorkCn => "qwen-work-cn",
        }
    }
    fn base(self) -> &'static str {
        match self {
            Self::QwenWork => "https://gateway.qwenwork.ai",
            Self::QwenWorkCn => "https://gateway.qwenwork.cn",
        }
    }
    fn client_id(self) -> &'static str {
        match self {
            Self::QwenWork => "cc65e5fc-05bd-4f5d-a4e8-0df19aa3d75a",
            Self::QwenWorkCn => "e883ade2-e6e3-4d6d-adf7-f92ceff5fdcb",
        }
    }
    fn redirect(self) -> &'static str {
        match self {
            Self::QwenWork => "qwenwork://",
            Self::QwenWorkCn => "qwenwork-cn://",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditBucket {
    pub name: String,
    pub remaining: Option<f64>,
    pub total: Option<f64>,
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyReward {
    pub status: String,
    pub reward_credits: Option<f64>,
    pub checked_at: i64,
    pub next_claim_at: Option<i64>,
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub edition: Edition,
    pub name: String,
    pub email: Option<String>,
    pub organization: Option<String>,
    pub plan: Option<String>,
    pub plan_name: Option<String>,
    pub expires_at: Option<i64>,
    pub active: bool,
    pub remaining: Option<f64>,
    pub total: Option<f64>,
    pub credit_buckets: Vec<CreditBucket>,
    pub daily_reward: Option<DailyReward>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    edition: Edition,
    auth: Value,
}

impl Credentials {
    fn validate(&self) -> Result<(), String> {
        if self.auth["schemaVersion"] != 2
            || [
                self.auth["token"].as_str(),
                self.auth["refreshToken"].as_str(),
                self.auth["user"]["id"].as_str(),
            ]
            .iter()
            .any(|value| value.map_or(true, |value| value.trim().is_empty()))
            || api::timestamp(&self.auth["expiresAt"]).is_none()
            || (self.auth["user"]["isBiz"] == true
                && api::text(&self.auth["user"], &["teamId"]).is_none())
        {
            return Err("accountError.invalidAccount".into());
        }
        Ok(())
    }
    fn id(&self) -> String {
        let user = &self.auth["user"];
        format!(
            "{:x}",
            Sha256::digest(format!(
                "{}\0{}\0{}\0{}\0{}",
                self.edition.tool(),
                user["id"].as_str().unwrap_or(""),
                user["isBiz"] == true,
                user["orgId"].as_str().unwrap_or(""),
                user["teamId"].as_str().unwrap_or("")
            ))
        )
    }
    fn summary(&self) -> Account {
        let user = &self.auth["user"];
        Account {
            id: self.id(),
            edition: self.edition,
            name: api::text(user, &["name", "email", "id"])
                .unwrap_or_default()
                .into(),
            email: api::text(user, &["email"]).map(str::to_owned),
            organization: api::text(user, &["orgName"]).map(str::to_owned),
            plan: api::text(user, &["planId"]).map(str::to_owned),
            plan_name: api::text(user, &["planName"]).map(str::to_owned),
            expires_at: api::timestamp(&user["planNextDueDate"]),
            active: false,
            remaining: None,
            total: None,
            credit_buckets: vec![],
            daily_reward: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    account: Account,
    credential: String,
}
type Store = BTreeMap<String, Saved>;

fn path(edition: Edition) -> Result<PathBuf, String> {
    Ok(dirs::home_dir()
        .ok_or("accountError.home")?
        .join(format!(".echobird/{}-accounts.json", edition.tool())))
}
fn load(path: &Path) -> Result<Store, String> {
    if !path.exists() {
        return Ok(Store::new());
    }
    super::cursor_auth::read(path)
}
fn protect(c: &Credentials) -> Result<String, String> {
    c.validate()?;
    let value = super::model_manager::encrypt_key_for_storage(
        &serde_json::to_string(c).map_err(|_| "accountError.format")?,
    );
    if !value.starts_with("enc:v1:") {
        return Err("accountError.keychain".into());
    }
    Ok(value)
}
fn credential(store: &Store, edition: Edition, id: &str) -> Result<Credentials, String> {
    let saved = store
        .get(id)
        .filter(|saved| saved.account.edition == edition && saved.account.id == id)
        .ok_or("accountError.invalidAccount")?;
    if !saved.credential.starts_with("enc:v1:") {
        return Err("accountError.keychain".into());
    }
    let c: Credentials = serde_json::from_str(&super::model_manager::decrypt_key_for_use(
        &saved.credential,
    ))
    .map_err(|_| "accountError.keychain")?;
    c.validate()?;
    if c.edition != edition || c.id() != id {
        return Err("accountError.invalidAccount".into());
    }
    Ok(c)
}
fn request_lock(edition: Edition, id: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    static LOCKS: OnceLock<Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    Ok(LOCKS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "accountError.read")?
        .entry(format!("{}:{id}", edition.tool()))
        .or_default()
        .clone())
}

pub async fn list(edition: Edition) -> Result<Vec<Account>, String> {
    let _guard = STORE_LOCK.lock().await;
    Ok(load(&path(edition)?)?
        .into_values()
        .filter(|s| s.account.edition == edition)
        .map(|s| s.account)
        .collect())
}
pub async fn start_login(edition: Edition) -> Result<LoginStart, String> {
    oauth::start(edition).await
}
pub fn cancel_login(id: &str) -> Result<(), String> {
    oauth::cancel(id)
}
pub async fn poll_login(id: &str) -> Result<Option<Account>, String> {
    let Some(c) = oauth::poll(id)? else {
        return Ok(None);
    };
    let request = request_lock(c.edition, &c.id())?;
    let _request = request.lock().await;
    let _guard = STORE_LOCK.lock().await;
    oauth::complete(id, || {
        let path = path(c.edition)?;
        let mut store = load(&path)?;
        let mut account = c.summary();
        if let Some(previous) = store.get(&account.id) {
            account.active = previous.account.active;
        }
        store.insert(
            account.id.clone(),
            Saved {
                account: account.clone(),
                credential: protect(&c)?,
            },
        );
        super::cursor_auth::write(&path, &store)?;
        Ok(Some(account))
    })
}
pub async fn delete(edition: Edition, id: &str) -> Result<(), String> {
    let request = request_lock(edition, id)?;
    let _request = request.lock().await;
    let _guard = STORE_LOCK.lock().await;
    let path = path(edition)?;
    let mut store = load(&path)?;
    if !store
        .get(id)
        .is_some_and(|row| row.account.edition == edition)
    {
        return Err("accountError.invalidAccount".into());
    }
    store.remove(id);
    super::cursor_auth::write(&path, &store)?;
    pending_rotations()
        .lock()
        .map_err(|_| "accountError.write")?
        .retain(|_, c| c.edition != edition || c.id() != id);
    Ok(())
}

fn pending_rotations() -> &'static Mutex<BTreeMap<String, Credentials>> {
    static ROTATIONS: OnceLock<Mutex<BTreeMap<String, Credentials>>> = OnceLock::new();
    ROTATIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}
fn rotation_key(c: &Credentials) -> Result<String, String> {
    Ok(format!(
        "{}:{:x}",
        c.id(),
        Sha256::digest(
            c.auth["refreshToken"]
                .as_str()
                .ok_or("accountError.invalidAccount")?
        )
    ))
}
async fn renew_once(
    c: &Credentials,
    client: &api::Client,
) -> Result<(String, Credentials), String> {
    let key = rotation_key(c)?;
    let cached = pending_rotations()
        .lock()
        .map_err(|_| "accountError.read")?
        .get(&key)
        .cloned();
    let next = match cached {
        Some(next) => next,
        None => {
            let next = client.refresh(c).await?;
            pending_rotations()
                .lock()
                .map_err(|_| "accountError.write")?
                .insert(key.clone(), next.clone());
            next
        }
    };
    Ok((key, next))
}

async fn credentials(edition: Edition, id: &str) -> Result<Credentials, String> {
    let _guard = STORE_LOCK.lock().await;
    credential(&load(&path(edition)?)?, edition, id)
}

async fn rotate(c: &Credentials, force: bool, client: &api::Client) -> Result<Credentials, String> {
    if !force
        && !pending_rotations()
            .lock()
            .map_err(|_| "accountError.read")?
            .contains_key(&rotation_key(c)?)
        && api::timestamp(&c.auth["expiresAt"])
            .is_some_and(|t| t > chrono::Utc::now().timestamp() + 300)
    {
        return Ok(c.clone());
    }
    let (key, next) = renew_once(c, client).await?;
    let _guard = STORE_LOCK.lock().await;
    let path = path(c.edition)?;
    let mut store = load(&path)?;
    let current = credential(&store, c.edition, &c.id())?;
    if current.auth != c.auth {
        pending_rotations()
            .lock()
            .map_err(|_| "accountError.write")?
            .remove(&key);
        return Ok(current);
    }
    store
        .get_mut(&c.id())
        .ok_or("accountError.invalidAccount")?
        .credential = protect(&next)?;
    super::cursor_auth::write(&path, &store)?;
    pending_rotations()
        .lock()
        .map_err(|_| "accountError.write")?
        .remove(&key);
    Ok(next)
}

pub async fn refresh(edition: Edition, id: &str) -> Result<Account, String> {
    let request = request_lock(edition, id)?;
    let _request = request.lock().await;
    let client = api::Client::installed(edition)?;
    let c = rotate(&credentials(edition, id).await?, false, &client).await?;
    let result = client.quota(&c).await;
    let quota = if matches!(&result, Err(e) if e == "accountError.loginRequired") {
        let c = rotate(&c, true, &client).await?;
        client.quota(&c).await?
    } else {
        result?
    };
    let _guard = STORE_LOCK.lock().await;
    let path = path(edition)?;
    let mut store = load(&path)?;
    credential(&store, edition, id)?;
    let saved = store.get_mut(id).ok_or("accountError.invalidAccount")?;
    quota.apply(&mut saved.account);
    saved.account.daily_reward = None;
    let account = saved.account.clone();
    super::cursor_auth::write(&path, &store)?;
    Ok(account)
}

pub async fn apply(edition: Edition, id: &str) -> Result<Account, String> {
    let request = request_lock(edition, id)?;
    let _request = request.lock().await;
    let client = api::Client::installed(edition)?;
    let mut c = rotate(&credentials(edition, id).await?, false, &client).await?;
    c.auth["user"] = client.user(api::token(&c)?).await?;
    if c.id() != id {
        return Err("accountError.invalidAccount".into());
    }
    let transaction = native::apply(&c).await?;
    let _guard = STORE_LOCK.lock().await;
    let path = path(edition)?;
    let mut store = load(&path)?;
    credential(&store, edition, id)?;
    for saved in store.values_mut() {
        saved.account.active = saved.account.id == id;
    }
    let account = store
        .get(id)
        .ok_or("accountError.invalidAccount")?
        .account
        .clone();
    super::cursor_auth::write(&path, &store)?;
    transaction.commit();
    Ok(account)
}

#[cfg(test)]
mod tests;
