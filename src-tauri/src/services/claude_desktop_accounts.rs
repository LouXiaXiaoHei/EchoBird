//! Official Claude Desktop sessions; the CLI OAuth store is intentionally separate.
use super::account_login_browser::Login;
pub use super::claude_code_accounts::ClaudeCodeAccount as Account;
use super::cursor_auth::{read, write};
mod browser;
use super::electron_storage::{cipher, decrypt_bytes, Cipher};
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
static ACCOUNT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PENDING: std::sync::Mutex<Option<Login>> = std::sync::Mutex::new(None);

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
    load_store_at(&store_path()?)
}
fn load_store_at(path: &Path) -> Result<Store, String> {
    if path.exists() {
        read(path)
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
    let db = Connection::open_with_flags(cookie_path(dir)?, flags).map_err(|_| {
        if writable {
            "accountError.write"
        } else {
            "accountError.read"
        }
    })?;
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
    if version == browser::COOKIE_VERSION {
        return browser::value(cookie);
    }
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
    let session_cookie =
        primary_cookie(snapshot, "sessionKey").ok_or("accountError.loginRequired")?;
    let org_cookie =
        primary_cookie(snapshot, "lastActiveOrg").ok_or("accountError.loginRequired")?;
    let browser = snapshot.cookie_version == browser::COOKIE_VERSION;
    let plain = [session_cookie, org_cookie]
        .iter()
        .all(|cookie| cookie.text("value").is_some_and(|value| !value.is_empty()));
    let key = if browser || plain {
        None
    } else {
        Some(cipher(dir)?)
    };
    let value = |cookie: &Cookie| match &key {
        Some(key) => cookie_value(snapshot.cookie_version, cookie, key),
        None if browser => browser::value(cookie),
        None => cookie
            .text("value")
            .map(str::to_owned)
            .ok_or_else(|| "accountError.format".into()),
    };
    let session = value(session_cookie)?;
    let org = value(org_cookie)?;
    uuid::Uuid::parse_str(&org).map_err(|_| "accountError.authResponse")?;
    if session.is_empty() || session.contains(['\r', '\n', ';']) {
        return Err("accountError.format".into());
    }
    Ok((session, org))
}
fn account_id(uuid: &str, org: &str) -> String {
    format!("{:x}", Sha256::digest(format!("{uuid}\n{org}").as_bytes()))
}
fn validate_saved_identity(saved: &Saved, id: &str, org: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(&saved.account_uuid).is_err()
        || saved.account.id != id
        || account_id(&saved.account_uuid, org) != id
        || org != saved.organization_uuid
        || saved
            .snapshot
            .config
            .get("lastKnownAccountUuid")
            .and_then(Value::as_str)
            .is_some_and(|uuid| uuid != saved.account_uuid)
        || saved
            .snapshot
            .config
            .keys()
            .any(|name| !AUTH_CONFIG.contains(&name.as_str()))
        || saved.snapshot.cookies.iter().any(|cookie| {
            !is_auth_cookie(
                cookie.text("host_key").unwrap_or_default(),
                cookie.text("name").unwrap_or_default(),
            )
        })
    {
        return Err("accountError.authResponse".into());
    }
    Ok(())
}

// Unit tests must not contact the public Claude service. This also makes an
// accidental live verification in cached account application fail immediately.
#[cfg(test)]
async fn request(_session: &str, _org: &str, _endpoint: &str) -> Result<Value, String> {
    Err("accountError.network".into())
}
#[cfg(not(test))]
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
            id: account_id(uuid, org),
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
pub async fn start_login(app: tauri::AppHandle) -> Result<LoginStart, String> {
    let login = Login::start(
        &app,
        "Claude",
        "https://claude.ai/login",
        &[
            "claude.ai",
            "claude.com",
            "anthropic.com",
            "google.com",
            "googleusercontent.com",
            "apple.com",
        ],
    )?;
    let old = PENDING
        .lock()
        .map_err(|_| "accountError.busy")?
        .replace(login.clone());
    if let Some(old) = old {
        old.cancel(&app);
    }
    Ok(LoginStart {
        login_id: login.id,
        expires_at: login.expires,
    })
}
fn pending_login(id: &str) -> Result<Login, String> {
    let login = PENDING
        .lock()
        .map_err(|_| "accountError.busy")?
        .as_ref()
        .filter(|login| login.id == id)
        .cloned()
        .ok_or("accountError.cancelled")?;
    login.check()?;
    Ok(login)
}
pub async fn cancel_login(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let login = {
        let mut pending = PENDING.lock().map_err(|_| "accountError.busy")?;
        if pending.as_ref().is_some_and(|login| login.id == id) {
            pending.take()
        } else {
            None
        }
    };
    if let Some(login) = login {
        login.cancel(app);
    }
    Ok(())
}
pub async fn poll_login(app: &tauri::AppHandle, id: &str) -> Result<LoginPoll, String> {
    let login = pending_login(id)?;
    let Some(mut snapshot) = browser::capture(&login.cookies(app)?)? else {
        return Ok(LoginPoll::default());
    };
    let (session, org) = session_values(&data_dir()?, &snapshot)?;
    let (mut account, account_uuid) = match profile(&session, &org).await {
        Err(error) if error == "accountError.loginRequired" => return Ok(LoginPoll::default()),
        result => result?,
    };
    account.active = false;
    snapshot
        .config
        .insert("lastKnownAccountUuid".into(), json!(account_uuid));
    let _lock = ACCOUNT_LOCK.lock().await;
    let mut pending = PENDING.lock().map_err(|_| "accountError.busy")?;
    pending
        .as_ref()
        .filter(|current| current.id == id)
        .ok_or("accountError.cancelled")?;
    let result = login.complete(app, || {
        let mut store = load_store()?;
        if let Some(previous) = store.get(&account.id) {
            account.five_hour = previous.account.five_hour.clone();
            account.seven_day = previous.account.seven_day.clone();
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
        Ok(LoginPoll {
            account: Some(account),
            ..Default::default()
        })
    })?;
    *pending = None;
    Ok(result)
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
    switch_at(
        id,
        &data_dir()?,
        &store_path()?,
        super::process_manager::stop_desktop_for_account("claudedesktop"),
        |target| super::claude_desktop_history::prepare(Some(target)),
    )
    .await
}
async fn switch_at<S, H, HF>(
    id: &str,
    dir: &Path,
    store_path: &Path,
    stop: S,
    prepare_history: H,
) -> Result<Account, String>
where
    S: std::future::Future<Output = Result<(), String>>,
    H: FnOnce(PathBuf) -> HF,
    HF: std::future::Future<Output = Result<(), String>>,
{
    let mut store = load_store_at(store_path)?;
    let saved = store.get(id).ok_or("accountError.invalidAccount")?;
    let (_session, org) = session_values(dir, &saved.snapshot)?;
    validate_saved_identity(saved, id, &org)?;
    stop.await?;
    prepare_history(
        dir.join("claude-code-sessions")
            .join(&saved.account_uuid)
            .join(&saved.organization_uuid),
    )
    .await?;
    let mut db = cookie_db(dir, true)?;
    let snapshot = if saved.snapshot.cookie_version == browser::COOKIE_VERSION {
        browser::native_snapshot(&db, &saved.snapshot, &cipher(dir)?)?
    } else {
        saved.snapshot.clone()
    };
    restore(&mut db, &dir.join("config.json"), &snapshot)?;
    store
        .get_mut(id)
        .ok_or("accountError.invalidAccount")?
        .snapshot = snapshot;
    for (key, saved) in &mut store {
        saved.account.active = key == id;
    }
    let account = store
        .get(id)
        .ok_or("accountError.invalidAccount")?
        .account
        .clone();
    write(store_path, &store)?;
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

    struct ApplyFixture {
        dir: PathBuf,
        id: String,
    }
    impl ApplyFixture {
        fn new() -> Self {
            let dir = std::env::temp_dir()
                .join(format!("echobird-claude-apply-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).unwrap();
            let db = database("");
            db.execute("VACUUM INTO ?1", [dir.join("Cookies").to_str().unwrap()])
                .unwrap();
            let account_uuid = "11111111-1111-4111-8111-111111111111";
            let org = "22222222-2222-4222-8222-222222222222";
            let id = account_id(account_uuid, org);
            let mut session = cookies(&db).unwrap().remove(0);
            session
                .0
                .insert("value".into(), Cell::Text("offline-fixture-session".into()));
            let mut organization = session.clone();
            organization
                .0
                .insert("name".into(), Cell::Text("lastActiveOrg".into()));
            organization
                .0
                .insert("value".into(), Cell::Text(org.into()));
            write(
                &dir.join("accounts.json"),
                &Store::from([(
                    id.clone(),
                    Saved {
                        account: Account {
                            id: id.clone(),
                            email: "fixture@example.test".into(),
                            plan: Some("Pro".into()),
                            five_hour: None,
                            seven_day: None,
                            active: false,
                        },
                        account_uuid: account_uuid.into(),
                        organization_uuid: org.into(),
                        snapshot: Snapshot {
                            cookies: vec![session, organization],
                            config: BTreeMap::from([(
                                "lastKnownAccountUuid".into(),
                                json!(account_uuid),
                            )]),
                            cookie_version: 24,
                        },
                    },
                )]),
            )
            .unwrap();
            write(
                &dir.join("config.json"),
                &json!({"locale":"ja","lastKnownAccountUuid":"old"}),
            )
            .unwrap();
            Self { dir, id }
        }
        fn state(&self) -> Vec<Vec<u8>> {
            ["accounts.json", "config.json", "Cookies"]
                .iter()
                .map(|name| std::fs::read(self.dir.join(name)).unwrap())
                .collect()
        }
    }
    impl Drop for ApplyFixture {
        fn drop(&mut self) {
            for name in ["accounts.json", "config.json", "Cookies"] {
                std::fs::remove_file(self.dir.join(name)).unwrap();
            }
            std::fs::remove_dir(&self.dir).unwrap();
        }
    }

    #[tokio::test]
    async fn cached_apply_works_offline_and_stops_before_history_and_native_writes() {
        let fixture = ApplyFixture::new();
        let path = fixture.dir.join("accounts.json");
        let store = load_store_at(&path).unwrap();
        let saved = &store[&fixture.id];
        let (session, org) = session_values(&fixture.dir, &saved.snapshot).unwrap();
        // Positive control: online identity verification is unavailable in this test.
        assert!(
            matches!(profile(&session, &org).await, Err(error) if error == "accountError.network")
        );
        let stage = std::cell::Cell::new(0);
        let before = fixture.state();
        let account = switch_at(
            &fixture.id,
            &fixture.dir,
            &path,
            async {
                assert_eq!(stage.replace(1), 0);
                assert_eq!(fixture.state(), before);
                Ok(())
            },
            |target| {
                assert_eq!(stage.replace(2), 1);
                assert_eq!(fixture.state(), before);
                assert_eq!(
                    target,
                    fixture
                        .dir
                        .join("claude-code-sessions")
                        .join(&saved.account_uuid)
                        .join(&saved.organization_uuid)
                );
                async { Ok(()) }
            },
        )
        .await
        .unwrap();
        assert_eq!(stage.get(), 2);
        assert_eq!(account.id, fixture.id);
        assert!(account.active);
        assert!(load_store_at(&path).unwrap()[&fixture.id].account.active);
        let native = snapshot(&fixture.dir).unwrap();
        assert_eq!(
            session_values(&fixture.dir, &native).unwrap(),
            (session, org)
        );
        assert_eq!(
            native.config["lastKnownAccountUuid"],
            json!(saved.account_uuid)
        );
        assert_eq!(native_config(&fixture.dir).unwrap()["locale"], json!("ja"));
        let db = cookie_db(&fixture.dir, false).unwrap();
        let unrelated: Vec<String> = db.prepare("SELECT value FROM cookies WHERE name='theme' OR host_key='example.test' ORDER BY value")
            .unwrap().query_map([], |row| row.get(0)).unwrap()
            .collect::<Result<_, _>>().unwrap();
        assert_eq!(unrelated, ["dark", "other-app"]);
    }

    #[tokio::test]
    async fn invalid_cached_identity_does_not_stop_or_write() {
        for field in [
            "account_id",
            "account_uuid",
            "organization_uuid",
            "config_uuid",
        ] {
            let fixture = ApplyFixture::new();
            let path = fixture.dir.join("accounts.json");
            let mut store = load_store_at(&path).unwrap();
            let saved = store.get_mut(&fixture.id).unwrap();
            match field {
                "account_id" => saved.account.id = "wrong".into(),
                "account_uuid" => {
                    saved.account_uuid = "33333333-3333-4333-8333-333333333333".into()
                }
                "organization_uuid" => {
                    saved.organization_uuid = "33333333-3333-4333-8333-333333333333".into()
                }
                "config_uuid" => {
                    saved
                        .snapshot
                        .config
                        .insert("lastKnownAccountUuid".into(), json!("other-account"));
                }
                _ => unreachable!(),
            }
            write(&path, &store).unwrap();
            let before = fixture.state();
            let result = switch_at(
                &fixture.id,
                &fixture.dir,
                &path,
                async { panic!("invalid identity must not stop the client") },
                |_| async { panic!("invalid identity must not prepare history") },
            )
            .await;
            assert!(matches!(result, Err(error) if error == "accountError.authResponse"));
            assert_eq!(fixture.state(), before);
        }
    }

    #[tokio::test]
    async fn failed_stop_does_not_prepare_history_or_write_credentials() {
        let fixture = ApplyFixture::new();
        let before = fixture.state();
        let result = switch_at(
            &fixture.id,
            &fixture.dir,
            &fixture.dir.join("accounts.json"),
            async { Err("accountError.stopClient".into()) },
            |_| async { panic!("failed stop must not prepare history") },
        )
        .await;
        assert!(matches!(result, Err(error) if error == "accountError.stopClient"));
        assert_eq!(fixture.state(), before);
    }

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
