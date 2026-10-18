//! Official Claude Desktop sessions; the CLI OAuth store is intentionally separate.
pub use super::claude_code_accounts::ClaudeCodeAccount as Account;
use super::cursor_auth::{read, write};
use super::electron_storage::{cipher, decrypt, decrypt_bytes, encrypt, Cipher};
use rusqlite::{types::Value as SqlValue, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

const AUTH_NAMES: &[&str] = &[
    "sessionKey",
    "sessionKeyV2",
    "sessionKeyV3",
    "sessionKeyLC",
    "sessionKeyV3LC",
    "lastActiveOrg",
    "authLastActiveOrg",
];
const AUTH_CONFIG: &[&str] = &[
    "lastKnownAccountUuid",
    "oauth:tokenCache",
    "oauth:tokenCacheV2",
];
const LOGIN_SECONDS: i64 = 60;
static ACCOUNT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PENDING: std::sync::Mutex<Option<Pending>> = std::sync::Mutex::new(None);

#[derive(Clone, Serialize, Deserialize)]
enum Cell {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}
impl Cell {
    fn sql(&self) -> SqlValue {
        match self {
            Self::Null => SqlValue::Null,
            Self::Integer(v) => SqlValue::Integer(*v),
            Self::Real(v) => SqlValue::Real(*v),
            Self::Text(v) => SqlValue::Text(v.clone()),
            Self::Blob(v) => SqlValue::Blob(v.clone()),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Cookie(BTreeMap<String, Cell>);
impl Cookie {
    fn text(&self, name: &str) -> Option<&str> {
        match self.0.get(name)? {
            Cell::Text(v) => Some(v),
            _ => None,
        }
    }
    fn integer(&self, name: &str) -> Option<i64> {
        match self.0.get(name)? {
            Cell::Integer(v) => Some(*v),
            _ => None,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    cookies: Vec<Cookie>,
    config: BTreeMap<String, Value>,
    cookie_version: i64,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    account: Account,
    account_uuid: String,
    organization_uuid: String,
    snapshot: Snapshot,
}
type Store = BTreeMap<String, Saved>;
#[derive(Clone)]
struct Pending {
    id: String,
    dir: PathBuf,
    expires: i64,
    awaiting_exit: bool,
}
impl Pending {
    fn validate(&self, id: &str, now: i64) -> Result<(), String> {
        if self.id != id {
            return Err("accountError.cancelled".into());
        }
        if now >= self.expires {
            return Err("accountError.expired".into());
        }
        Ok(())
    }
    fn begin_exit(&mut self, now: i64) {
        if !self.awaiting_exit {
            self.awaiting_exit = true;
            self.expires = now + LOGIN_SECONDS;
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    login_id: String,
    expires_at: i64,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginPoll {
    account: Option<Account>,
    awaiting_client_exit: bool,
    expires_at: Option<i64>,
}

fn data_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    if let Some(uri) = super::tool_manager::get_tool_launch_uri("claudedesktop") {
        if super::tool_manager::get_tool_exe_path("claudedesktop").is_none() {
            if let Some(dir) = super::msix::roaming_data_dir(&uri, "Claude") {
                return Ok(dir);
            }
        }
    }
    dirs::config_dir()
        .map(|p| p.join("Claude"))
        .ok_or_else(|| "accountError.home".into())
}
fn store_path() -> Result<PathBuf, String> {
    Ok(dirs::home_dir()
        .ok_or("accountError.home")?
        .join(".echobird/claude-desktop-accounts.json"))
}
fn load_store() -> Result<Store, String> {
    let path = store_path()?;
    if path.exists() {
        read(&path)
    } else {
        Ok(Store::new())
    }
}
fn cookie_path(dir: &Path) -> Result<PathBuf, String> {
    [
        "Network/Cookies",
        "Cookies",
        "Default/Network/Cookies",
        "Default/Cookies",
    ]
    .iter()
    .map(|p| dir.join(p))
    .find(|p| p.is_file())
    .ok_or_else(|| "accountError.initializeClient".into())
}
fn cookie_db(dir: &Path, writable: bool) -> Result<Connection, String> {
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(cookie_path(dir)?, flags)
        .map_err(|_| "accountError.closeClient")?;
    db.busy_timeout(Duration::from_millis(100))
        .map_err(|_| "accountError.read")?;
    Ok(db)
}
fn is_auth_cookie(host: &str, name: &str) -> bool {
    AUTH_NAMES.contains(&name) && matches!(host.trim_start_matches('.'), "claude.ai" | "claude.com")
}
fn cookies(db: &Connection) -> Result<Vec<Cookie>, String> {
    let names = AUTH_NAMES.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let mut statement = db.prepare(&format!("SELECT * FROM cookies WHERE name IN ({names}) AND host_key IN ('claude.ai','.claude.ai','claude.com','.claude.com')")).map_err(|_| "accountError.read")?;
    let columns = statement
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let result = statement
        .query_map(rusqlite::params_from_iter(AUTH_NAMES), |row| {
            let mut values = BTreeMap::new();
            for (i, name) in columns.iter().enumerate() {
                let cell = match row.get::<_, SqlValue>(i)? {
                    SqlValue::Null => Cell::Null,
                    SqlValue::Integer(v) => Cell::Integer(v),
                    SqlValue::Real(v) => Cell::Real(v),
                    SqlValue::Text(v) => Cell::Text(v),
                    SqlValue::Blob(v) => Cell::Blob(v),
                };
                values.insert(name.clone(), cell);
            }
            Ok(Cookie(values))
        })
        .map_err(|_| "accountError.read")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "accountError.read".into());
    result
}
fn native_config(dir: &Path) -> Result<Value, String> {
    if dir.join("config.json").exists() {
        read(&dir.join("config.json"))
    } else {
        Ok(json!({}))
    }
}
fn snapshot(dir: &Path) -> Result<Snapshot, String> {
    let config = native_config(dir)?;
    let db = cookie_db(dir, false)?;
    let cookie_version = db
        .query_row("SELECT value FROM meta WHERE key='version'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    Ok(Snapshot {
        cookies: cookies(&db)?,
        config: AUTH_CONFIG
            .iter()
            .filter_map(|k| config.get(*k).map(|v| ((*k).into(), v.clone())))
            .collect(),
        cookie_version,
    })
}
fn primary_cookie<'a>(snapshot: &'a Snapshot, name: &str) -> Option<&'a Cookie> {
    let now = (chrono::Utc::now().timestamp() + 11_644_473_600) * 1_000_000;
    snapshot
        .cookies
        .iter()
        .filter(|c| {
            c.text("name") == Some(name)
                && c.text("path") == Some("/")
                && (c.integer("has_expires") == Some(0)
                    || c.integer("expires_utc").is_some_and(|e| e > now))
        })
        .max_by_key(|c| {
            (
                c.text("host_key").is_some_and(|s| s.contains("claude.ai")),
                c.integer("last_update_utc"),
            )
        })
}
fn cookie_value(version: i64, cookie: &Cookie, cipher: &Cipher) -> Result<String, String> {
    if let Some(value) = cookie.text("value").filter(|v| !v.is_empty()) {
        return Ok(value.into());
    }
    let Some(Cell::Blob(bytes)) = cookie.0.get("encrypted_value") else {
        return Err("accountError.format".into());
    };
    let bytes = decrypt_bytes(cipher, bytes)?;
    let bytes = if version >= 24 {
        bytes
            .strip_prefix(
                Sha256::digest(
                    cookie
                        .text("host_key")
                        .ok_or("accountError.format")?
                        .as_bytes(),
                )
                .as_slice(),
            )
            .ok_or("accountError.format")?
    } else {
        &bytes
    };
    String::from_utf8(bytes.to_vec()).map_err(|_| "accountError.format".into())
}
fn session_values(dir: &Path, snapshot: &Snapshot) -> Result<(String, String), String> {
    let key = cipher(dir)?;
    let session = cookie_value(
        snapshot.cookie_version,
        primary_cookie(snapshot, "sessionKey").ok_or("accountError.loginRequired")?,
        &key,
    )?;
    let org = cookie_value(
        snapshot.cookie_version,
        primary_cookie(snapshot, "lastActiveOrg").ok_or("accountError.loginRequired")?,
        &key,
    )?;
    uuid::Uuid::parse_str(&org).map_err(|_| "accountError.authResponse")?;
    if session.is_empty() || session.contains(['\r', '\n', ';']) {
        return Err("accountError.format".into());
    }
    Ok((session, org))
}
async fn request(session: &str, org: &str, endpoint: &str) -> Result<Value, String> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "accountError.network")?
        .get(format!("https://claude.ai/api/{endpoint}"))
        .header(reqwest::header::ACCEPT, "application/json")
        .header(
            reqwest::header::COOKIE,
            format!("sessionKey={session}; lastActiveOrg={org}"),
        )
        .header("x-organization-uuid", org)
        .send()
        .await
        .map_err(|_| "accountError.network")?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("accountError.loginRequired".into());
    }
    if !response.status().is_success() {
        return Err(format!("accountError.quota|HTTP {}", response.status()));
    }
    response
        .json()
        .await
        .map_err(|_| "accountError.authResponse".into())
}
fn account_from_profile(body: &Value, org: &str) -> Result<(Account, String), String> {
    let account = &body["account"];
    let uuid = account["uuid"]
        .as_str()
        .filter(|s| uuid::Uuid::parse_str(s).is_ok())
        .ok_or("accountError.authResponse")?;
    let email = account["email_address"]
        .as_str()
        .or_else(|| account["email"].as_str())
        .filter(|s| !s.is_empty())
        .ok_or("accountError.authResponse")?;
    let organization = account["memberships"]
        .as_array()
        .and_then(|memberships| {
            memberships
                .iter()
                .map(|m| &m["organization"])
                .find(|o| o["uuid"].as_str() == Some(org))
        })
        .or_else(|| {
            [
                &body["organization"],
                &body["active_organization"],
                &body["activeOrganization"],
            ]
            .into_iter()
            .find(|o| o["uuid"].as_str() == Some(org))
        })
        .ok_or("accountError.authResponse")?;
    let caps = organization["capabilities"].as_array();
    let has = |key: &str| caps.is_some_and(|c| c.iter().any(|v| v.as_str() == Some(key)));
    let plan = if organization["organization_type"].as_str() == Some("claude_enterprise") {
        Some("Enterprise")
    } else if has("claude_max") {
        Some("Max")
    } else if has("raven") || has("claude_team") {
        Some("Team")
    } else if has("claude_pro") {
        Some("Pro")
    } else if has("claude_free")
        || has("free")
        || (has("chat") && organization["billing_type"].as_str() == Some("none"))
    {
        Some("Free")
    } else {
        None
    };
    Ok((
        Account {
            id: format!("{:x}", Sha256::digest(format!("{uuid}\n{org}").as_bytes())),
            email: email.into(),
            plan: plan.map(str::to_owned),
            five_hour: None,
            seven_day: None,
            active: true,
        },
        uuid.into(),
    ))
}
async fn profile(session: &str, org: &str) -> Result<(Account, String), String> {
    let body = request(session, org, &format!("bootstrap/{org}/app_start?statsig_hashing_algorithm=djb2&growthbook_format=sdk&include_system_prompts=false")).await?;
    account_from_profile(&body, org)
}
fn usage_window(
    body: &Value,
    legacy_field: &str,
    kind: &str,
) -> Option<super::claude_code_accounts::ClaudeCodeQuota> {
    super::claude_code_accounts::quota(&body[legacy_field]).or_else(|| {
        let limit = body["limits"].as_array()?.iter().find(|limit| {
            (limit["kind"].as_str() == Some(kind) || limit["group"].as_str() == Some(kind))
                && limit["scope"]["model"].is_null()
        })?;
        let used = limit["percent"]
            .as_f64()
            .or_else(|| limit["percent"].as_str()?.parse::<f64>().ok())?;
        let mut quota = super::claude_code_accounts::quota(
            &json!({"utilization":used,"resets_at":limit["resets_at"]}),
        )?;
        if let Some(seconds) = limit["resets_at"].as_i64() {
            quota.reset_at = Some(seconds);
        }
        Some(quota)
    })
}
fn set_usage(account: &mut Account, body: &Value) -> Result<(), String> {
    account.five_hour = usage_window(body, "five_hour", "session");
    account.seven_day = usage_window(body, "seven_day", "weekly");
    let free_without_limits = account.plan.as_deref() == Some("Free")
        && body["limits"]
            .as_array()
            .is_some_and(|limits| limits.is_empty());
    if account.five_hour.is_none() && account.seven_day.is_none() && !free_without_limits {
        return Err("accountError.quota".into());
    }
    Ok(())
}
fn fingerprint(snapshot: &Snapshot) -> Option<String> {
    primary_cookie(snapshot, "sessionKey").map(|c| {
        let fields = [Some(c), primary_cookie(snapshot, "lastActiveOrg")]
            .into_iter()
            .flat_map(|c| {
                ["host_key", "name", "path", "value", "encrypted_value"]
                    .into_iter()
                    .map(move |k| c.and_then(|c| c.0.get(k)))
            })
            .collect::<Vec<_>>();
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&fields).unwrap_or_default())
        )
    })
}
fn scope_token_caches(
    snapshot: &mut Snapshot,
    key: &Cipher,
    account_uuid: &str,
    org: &str,
) -> Result<(), String> {
    // Native caches use acct:<account>|<client>:<org>:<host>:<scope> keys.
    for name in ["oauth:tokenCache", "oauth:tokenCacheV2"] {
        if let Some(value) = snapshot.config.get_mut(name) {
            let mut cache: serde_json::Map<String, Value> =
                serde_json::from_str(&decrypt(key, value.as_str().ok_or("accountError.format")?)?)
                    .map_err(|_| "accountError.format")?;
            let prefix = format!("acct:{account_uuid}|");
            cache.retain(|key, _| {
                if key.starts_with("acct:") {
                    key.starts_with(&prefix)
                } else {
                    key.split(':').nth(1) == Some(org)
                }
            });
            *value = Value::String(encrypt(key, &Value::Object(cache).to_string())?);
        }
    }
    Ok(())
}
pub async fn list() -> Result<Vec<Account>, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    let mut store = load_store()?;
    let dir = data_dir()?;
    let live = snapshot(&dir).ok().and_then(|s| fingerprint(&s));
    let mut accounts = store
        .values_mut()
        .map(|s| {
            s.account.active = live.is_some() && live == fingerprint(&s.snapshot);
            s.account.clone()
        })
        .collect::<Vec<_>>();
    accounts.sort_by(|a, b| a.email.cmp(&b.email).then(a.id.cmp(&b.id)));
    Ok(accounts)
}
pub async fn start_login() -> Result<LoginStart, String> {
    if super::tool_config_manager::claude_desktop_uses_third_party_mode()
        .map_err(|_| "accountError.read")?
    {
        // Reuse the existing official-model restart flow before native sign-in.
        // No fresh official login has happened yet, so there is no new session to flush.
        super::process_manager::stop_desktop_for_config("claudedesktop").await;
        if super::process_manager::desktop_tool_is_running("claudedesktop").await? {
            return Err("accountError.quitClaudeDesktop".into());
        }
        let restored = super::tool_config_manager::restore_tool_to_official("claudedesktop").await;
        if !restored.success {
            return Err("accountError.write".into());
        }
    }
    let now = chrono::Utc::now().timestamp();
    let pending = Pending {
        id: uuid::Uuid::new_v4().to_string(),
        dir: data_dir()?,
        expires: now + LOGIN_SECONDS,
        awaiting_exit: false,
    };
    let login = LoginStart {
        login_id: pending.id.clone(),
        expires_at: pending.expires,
    };
    *PENDING.lock().map_err(|_| "accountError.busy")? = Some(pending);
    if let Err(error) = super::process_manager::open_tool_for_login("claudedesktop").await {
        cancel_login(&login.login_id).await?;
        return Err(error);
    }
    Ok(login)
}
pub async fn cancel_login(id: &str) -> Result<(), String> {
    let mut guard = PENDING.lock().map_err(|_| "accountError.busy")?;
    if guard.as_ref().is_some_and(|p| p.id == id) {
        *guard = None;
    }
    Ok(())
}
pub async fn poll_login(id: &str) -> Result<LoginPoll, String> {
    let pending = {
        let mut guard = PENDING.lock().map_err(|_| "accountError.busy")?;
        let pending = guard.as_mut().ok_or("accountError.cancelled")?;
        pending.validate(id, chrono::Utc::now().timestamp())?;
        // This local hint only advances the prompt; the authenticated API below verifies identity.
        if native_config(&pending.dir)
            .ok()
            .is_some_and(|c| c["windowSizeWasSignedIn"].as_bool() == Some(true))
        {
            pending.begin_exit(chrono::Utc::now().timestamp());
        }
        pending.clone()
    };
    let waiting = || LoginPoll {
        awaiting_client_exit: pending.awaiting_exit,
        expires_at: Some(pending.expires),
        ..Default::default()
    };
    if super::process_manager::desktop_tool_is_running("claudedesktop").await? {
        return Ok(waiting());
    }
    let mut snapshot = match snapshot(&pending.dir) {
        Ok(s) => s,
        Err(e) if e == "accountError.closeClient" || e == "accountError.initializeClient" => {
            return Ok(waiting())
        }
        Err(e) => return Err(e),
    };
    if primary_cookie(&snapshot, "sessionKey").is_none() {
        return Ok(waiting());
    }
    let (session, org) = session_values(&pending.dir, &snapshot)?;
    let (account, account_uuid) = profile(&session, &org).await?;
    if snapshot
        .config
        .get("lastKnownAccountUuid")
        .and_then(Value::as_str)
        .is_some_and(|id| id != account_uuid)
    {
        return Err("accountError.authResponse".into());
    }
    if super::process_manager::desktop_tool_is_running("claudedesktop").await? {
        return Ok(waiting());
    }
    scope_token_caches(&mut snapshot, &cipher(&pending.dir)?, &account_uuid, &org)?;
    let _lock = ACCOUNT_LOCK.lock().await;
    let mut guard = PENDING.lock().map_err(|_| "accountError.busy")?;
    guard
        .as_ref()
        .ok_or("accountError.cancelled")?
        .validate(id, chrono::Utc::now().timestamp())?;
    // The client may reopen or switch accounts while the identity request was in flight.
    if fingerprint(&snapshot) != fingerprint(&self::snapshot(&pending.dir)?) {
        return Ok(waiting());
    }
    let mut store = load_store()?;
    for saved in store.values_mut() {
        saved.account.active = false;
    }
    store.insert(
        account.id.clone(),
        Saved {
            account: account.clone(),
            account_uuid,
            organization_uuid: org,
            snapshot,
        },
    );
    write(&store_path()?, &store)?;
    *guard = None;
    Ok(LoginPoll {
        account: Some(account),
        ..Default::default()
    })
}
fn restore(db: &mut Connection, config_path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let version = db
        .query_row("SELECT value FROM meta WHERE key='version'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    if (version >= 24) != (snapshot.cookie_version >= 24) {
        return Err("accountError.format".into());
    }
    let mut config: Value = read(config_path)?;
    let original = config.clone();
    let object = config.as_object_mut().ok_or("accountError.format")?;
    for name in AUTH_CONFIG {
        object.remove(*name);
    }
    for (name, value) in &snapshot.config {
        if !AUTH_CONFIG.contains(&name.as_str()) {
            return Err("accountError.format".into());
        }
        object.insert(name.clone(), value.clone());
    }
    let columns = db
        .prepare("SELECT * FROM cookies LIMIT 0")
        .map_err(|_| "accountError.read")?
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let tx = db.transaction().map_err(|_| "accountError.write")?;
    let names = AUTH_NAMES.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    tx.execute(&format!("DELETE FROM cookies WHERE name IN ({names}) AND host_key IN ('claude.ai','.claude.ai','claude.com','.claude.com')"), rusqlite::params_from_iter(AUTH_NAMES)).map_err(|_| "accountError.write")?;
    for cookie in &snapshot.cookies {
        if !is_auth_cookie(
            cookie.text("host_key").ok_or("accountError.format")?,
            cookie.text("name").ok_or("accountError.format")?,
        ) {
            return Err("accountError.format".into());
        }
        let fields = columns
            .iter()
            .filter(|k| cookie.0.contains_key(*k))
            .collect::<Vec<_>>();
        let quoted = fields
            .iter()
            .map(|k| format!("\"{}\"", k.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",");
        let placeholders = fields.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let values = fields
            .iter()
            .map(|k| cookie.0[*k].sql())
            .collect::<Vec<_>>();
        tx.execute(
            &format!("INSERT INTO cookies ({quoted}) VALUES ({placeholders})"),
            rusqlite::params_from_iter(values),
        )
        .map_err(|_| "accountError.write")?;
    }
    write(config_path, &config)?;
    if tx.commit().is_err() {
        write(config_path, &original)?;
        return Err("accountError.write".into());
    }
    Ok(())
}
pub async fn switch(id: &str) -> Result<Account, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    if super::process_manager::desktop_tool_is_running("claudedesktop").await? {
        return Err("accountError.quitClaudeDesktop".into());
    }
    let mut store = load_store()?;
    let saved = store.get(id).ok_or("accountError.invalidAccount")?;
    let dir = data_dir()?;
    let (session, org) = session_values(&dir, &saved.snapshot)?;
    let (verified, uuid) = profile(&session, &org).await?;
    if verified.id != id || uuid != saved.account_uuid || org != saved.organization_uuid {
        return Err("accountError.authResponse".into());
    }
    if super::process_manager::desktop_tool_is_running("claudedesktop").await? {
        return Err("accountError.quitClaudeDesktop".into());
    }
    super::claude_desktop_history::prepare(Some(
        dir.join("claude-code-sessions")
            .join(&saved.account_uuid)
            .join(&saved.organization_uuid),
    ))
    .await?;
    restore(
        &mut cookie_db(&dir, true)?,
        &dir.join("config.json"),
        &saved.snapshot,
    )?;
    for (key, saved) in &mut store {
        saved.account.active = key == id;
    }
    let account = store
        .get(id)
        .ok_or("accountError.invalidAccount")?
        .account
        .clone();
    write(&store_path()?, &store)?;
    Ok(account)
}
pub async fn refresh(id: &str) -> Result<Account, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    let mut store = load_store()?;
    let saved = store.get_mut(id).ok_or("accountError.invalidAccount")?;
    let (session, org) = session_values(&data_dir()?, &saved.snapshot)?;
    let (mut account, _) = profile(&session, &org).await?;
    if account.id != id {
        return Err("accountError.authResponse".into());
    }
    let quota = request(&session, &org, &format!("organizations/{org}/usage")).await?;
    set_usage(&mut account, &quota)?;
    account.active = saved.account.active;
    saved.account = account.clone();
    write(&store_path()?, &store)?;
    Ok(account)
}
pub async fn delete(id: &str) -> Result<(), String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    let mut store = load_store()?;
    store.remove(id).ok_or("accountError.invalidAccount")?;
    write(&store_path()?, &store)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(extra: &str) -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(&format!("CREATE TABLE cookies(host_key TEXT,name TEXT,path TEXT,value TEXT,encrypted_value BLOB,has_expires INTEGER,expires_utc INTEGER,last_update_utc INTEGER{extra});
            CREATE TABLE meta(key TEXT,value TEXT); INSERT INTO meta VALUES('version','24');
            INSERT INTO cookies(host_key,name,path,value,encrypted_value,has_expires,expires_utc,last_update_utc) VALUES('.claude.ai','sessionKey','/','fixture-old',X'',0,0,1),('.claude.ai','theme','/','dark',X'',0,0,1),('example.test','sessionKey','/','other-app',X'',0,0,1);" )).unwrap();
        db
    }
    #[test]
    fn restore_preserves_unrelated_settings_and_cookies_in_current_and_legacy_schemas() {
        for extra in ["", ",source_type INTEGER DEFAULT 0"] {
            let mut db = database(extra);
            let mut captured = cookies(&db).unwrap();
            assert_eq!(captured.len(), 1);
            captured[0]
                .0
                .insert("value".into(), Cell::Text("fixture-new".into()));
            let dir = std::env::temp_dir().join(format!(
                "echobird-claude-desktop-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let config_path = dir.join("config.json");
            write(&config_path, &json!({"locale":"ja","lastKnownAccountUuid":"old","oauth:tokenCache":"old-cache","oauth:tokenCacheV2":"old-v2","coworkTrustedDeviceToken:other":"keep"})).unwrap();
            let saved = Snapshot {
                cookies: captured,
                config: BTreeMap::from([
                    ("lastKnownAccountUuid".into(), json!("new")),
                    ("oauth:tokenCache".into(), json!("new-cache")),
                ]),
                cookie_version: 24,
            };
            restore(&mut db, &config_path, &saved).unwrap();
            let rows = db
                .prepare("SELECT value FROM cookies ORDER BY value")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(rows, ["dark", "fixture-new", "other-app"]);
            let config: Value = read(&config_path).unwrap();
            assert_eq!(
                config,
                json!({"locale":"ja","lastKnownAccountUuid":"new","oauth:tokenCache":"new-cache","coworkTrustedDeviceToken:other":"keep"})
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
    #[test]
    fn invalid_snapshot_rolls_back_cookie_changes_without_touching_config() {
        let mut db = database("");
        let dir = std::env::temp_dir().join(format!(
            "echobird-claude-desktop-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let config = json!({"locale":"en"});
        write(&path, &config).unwrap();
        let mut rows = cookies(&db).unwrap();
        rows[0]
            .0
            .insert("host_key".into(), Cell::Text("unrelated.test".into()));
        let mut snapshot = Snapshot {
            cookies: rows,
            config: BTreeMap::new(),
            cookie_version: 24,
        };
        assert_eq!(
            restore(&mut db, &path, &snapshot).unwrap_err(),
            "accountError.format"
        );
        assert_eq!(cookies(&db).unwrap()[0].text("value"), Some("fixture-old"));
        assert_eq!(read::<Value>(&path).unwrap(), config);
        snapshot.cookies[0]
            .0
            .insert("host_key".into(), Cell::Text(".claude.ai".into()));
        snapshot.cookie_version = 23;
        assert_eq!(
            restore(&mut db, &path, &snapshot).unwrap_err(),
            "accountError.format"
        );
        assert_eq!(cookies(&db).unwrap()[0].text("value"), Some("fixture-old"));
        assert_eq!(read::<Value>(&path).unwrap(), config);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn cookie_decryption_verifies_host_binding_and_expiry() {
        let db = database("");
        let mut cookie = cookies(&db).unwrap().remove(0);
        let mut bytes = Sha256::digest(b".claude.ai").to_vec();
        bytes.extend_from_slice(b"fixture-session");
        cookie.0.insert("value".into(), Cell::Text(String::new()));
        for key in super::super::electron_storage::test_ciphers() {
            cookie.0.insert(
                "encrypted_value".into(),
                Cell::Blob(super::super::electron_storage::encrypt_bytes(&key, &bytes).unwrap()),
            );
            assert_eq!(cookie_value(24, &cookie, &key).unwrap(), "fixture-session");
            let mut wrong_host = cookie.clone();
            wrong_host
                .0
                .insert("host_key".into(), Cell::Text(".claude.com".into()));
            assert_eq!(
                cookie_value(24, &wrong_host, &key).unwrap_err(),
                "accountError.format"
            );
            let mut legacy = cookie.clone();
            legacy.0.insert(
                "encrypted_value".into(),
                Cell::Blob(
                    super::super::electron_storage::encrypt_bytes(&key, b"legacy-session").unwrap(),
                ),
            );
            assert_eq!(cookie_value(23, &legacy, &key).unwrap(), "legacy-session");
        }
        let mut snapshot = Snapshot {
            cookies: vec![cookie.clone()],
            config: BTreeMap::new(),
            cookie_version: 24,
        };
        assert!(primary_cookie(&snapshot, "sessionKey").is_some());
        snapshot.cookies[0]
            .0
            .insert("has_expires".into(), Cell::Integer(1));
        snapshot.cookies[0]
            .0
            .insert("expires_utc".into(), Cell::Integer(1));
        assert!(primary_cookie(&snapshot, "sessionKey").is_none());
    }
    #[test]
    fn token_caches_keep_only_verified_account_and_legacy_organization_entries() {
        for key in super::super::electron_storage::test_ciphers() {
            let cache = json!({
                "acct:current|client:org:host:scope": {"token":"fixture-current"},
                "acct:other|client:org:host:scope": {"token":"fixture-other"},
                "client:org:host:scope": {"token":"fixture-legacy"},
                "client:other-org:host:scope": {"token":"fixture-wrong-org"}
            });
            let mut snapshot = Snapshot {
                cookies: vec![],
                config: ["oauth:tokenCache", "oauth:tokenCacheV2"]
                    .into_iter()
                    .map(|name| {
                        (
                            name.into(),
                            json!(encrypt(&key, &cache.to_string()).unwrap()),
                        )
                    })
                    .collect(),
                cookie_version: 24,
            };
            scope_token_caches(&mut snapshot, &key, "current", "org").unwrap();
            for value in snapshot.config.values() {
                let actual: Value =
                    serde_json::from_str(&decrypt(&key, value.as_str().unwrap()).unwrap()).unwrap();
                assert_eq!(
                    actual,
                    json!({
                        "acct:current|client:org:host:scope": {"token":"fixture-current"},
                        "client:org:host:scope": {"token":"fixture-legacy"}
                    })
                );
            }
        }
    }
    #[test]
    fn profile_uses_only_the_active_organizations_plan_and_rejects_unknown_identity() {
        let org = "12345678-1234-1234-1234-123456789abc";
        let uuid = "12345678-1234-1234-1234-123456789def";
        let mut body = json!({"account":{"uuid":uuid,"email_address":"fixture@example.test","memberships":[
            {"organization":{"uuid":"other-org","capabilities":["claude_max"]}},
            {"organization":{"uuid":org,"capabilities":["claude_pro"]}}
        ]}});
        let (account, id) = account_from_profile(&body, org).unwrap();
        assert_eq!(id, uuid);
        assert_eq!(account.plan.as_deref(), Some("Pro"));
        assert!(account.five_hour.is_none());
        assert!(account_from_profile(&body, "other").is_err());
        body["account"]["memberships"][1]["organization"]["capabilities"] = json!([]);
        assert!(account_from_profile(&body, org).unwrap().0.plan.is_none());
        body["account"]["memberships"][1]["organization"]["capabilities"] = json!(["chat"]);
        body["account"]["memberships"][1]["organization"]["billing_type"] = json!("none");
        assert_eq!(
            account_from_profile(&body, org).unwrap().0.plan.as_deref(),
            Some("Free")
        );
        body["account"]["memberships"][1]["organization"]["capabilities"] =
            json!(["chat", "claude_pro"]);
        assert_eq!(
            account_from_profile(&body, org).unwrap().0.plan.as_deref(),
            Some("Pro")
        );
        body["account"]["memberships"][1]["organization"]["capabilities"] = json!(["chat"]);
        body["account"]["memberships"][1]["organization"]["billing_type"] =
            json!("stripe_subscription");
        assert!(account_from_profile(&body, org).unwrap().0.plan.is_none());
        body["account"]["email_address"] = json!("");
        assert!(account_from_profile(&body, org).is_err());
    }
    #[test]
    fn free_empty_usage_stays_unknown_and_malformed_or_paid_empty_usage_fails() {
        let mut account = Account {
            id: "fixture".into(),
            email: "fixture@example.test".into(),
            plan: Some("Free".into()),
            five_hour: None,
            seven_day: None,
            active: false,
        };
        set_usage(&mut account, &json!({"limits":[]})).unwrap();
        assert!(account.five_hour.is_none());
        assert!(account.seven_day.is_none());
        assert!(set_usage(&mut account, &json!({})).is_err());
        set_usage(
            &mut account,
            &json!({"limits":[{"kind":"session","percent":100}]}),
        )
        .unwrap();
        assert_eq!(account.five_hour.as_ref().unwrap().remaining_percent, 0);
        account.plan = Some("Pro".into());
        assert!(set_usage(&mut account, &json!({"limits":[]})).is_err());
    }
    #[test]
    fn usage_accepts_both_schemas_without_treating_model_limits_as_account_limits() {
        let legacy = json!({"five_hour":{"utilization":20,"resets_at":"2026-10-09T12:00:00Z"}});
        let expected = usage_window(&legacy, "five_hour", "session").unwrap();
        let current = json!({"limits":[
            {"kind":"weekly","percent":90,"scope":{"model":{"display_name":"Claude Sonnet"}}},
            {"kind":"session","percent":20,"resets_at":"2026-10-09T12:00:00Z"},
            {"group":"weekly","percent":"0","resets_at":1791547200}
        ]});
        assert_eq!(
            usage_window(&current, "five_hour", "session").unwrap(),
            expected
        );
        let weekly = usage_window(&current, "seven_day", "weekly").unwrap();
        assert_eq!(weekly.remaining_percent, 100);
        assert_eq!(weekly.reset_at, Some(1791547200));
        assert!(usage_window(
            &json!({"limits":[{"kind":"session"}]}),
            "five_hour",
            "session"
        )
        .is_none());
        assert!(usage_window(
            &json!({"limits":[{"kind":"weekly","percent":10,"scope":{"model":{}}}]}),
            "seven_day",
            "weekly"
        )
        .is_none());
    }
    #[test]
    fn exit_phase_has_its_own_fixed_deadline_and_old_attempts_cannot_save() {
        let mut pending = Pending {
            id: "current".into(),
            dir: PathBuf::new(),
            expires: 60,
            awaiting_exit: false,
        };
        pending.validate("current", 50).unwrap();
        pending.begin_exit(50);
        assert_eq!(pending.expires, 110);
        pending.begin_exit(109);
        assert_eq!(pending.expires, 110);
        assert_eq!(
            pending.validate("current", 110).unwrap_err(),
            "accountError.expired"
        );
        assert_eq!(
            pending.validate("old", 55).unwrap_err(),
            "accountError.cancelled"
        );
    }
    #[test]
    fn access_time_updates_do_not_change_account_fingerprint() {
        let db = database("");
        let mut snapshot = Snapshot {
            cookies: cookies(&db).unwrap(),
            config: BTreeMap::new(),
            cookie_version: 24,
        };
        let original = fingerprint(&snapshot);
        snapshot.cookies[0]
            .0
            .insert("last_update_utc".into(), Cell::Integer(999));
        assert_eq!(fingerprint(&snapshot), original);
        snapshot.cookies[0]
            .0
            .insert("value".into(), Cell::Text("changed-session".into()));
        assert_ne!(fingerprint(&snapshot), original);
    }
}
