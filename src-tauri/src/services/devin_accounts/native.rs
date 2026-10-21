use super::{Account, Credentials, SERVER};
use crate::services::{devin_protocol as wire, electron_storage, native_account_files};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const SESSION: &str =
    "secret://{\"extensionId\":\"codeium.windsurf\",\"key\":\"windsurf_auth.sessions\"}";
const SERVER_KEY: &str =
    "secret://{\"extensionId\":\"codeium.windsurf\",\"key\":\"windsurf_auth.apiServerUrl\"}";
const STATUS: &str = "windsurfAuthStatus";

pub(super) fn desktop_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    let root = dirs::home_dir().map(|d| d.join("Library/Application Support"));
    #[cfg(not(target_os = "macos"))]
    let root = dirs::config_dir();
    Ok(root.ok_or("accountError.home")?.join("Devin"))
}
pub(super) fn cli_file() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let root = dirs::config_dir();
    #[cfg(not(windows))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_local_dir);
    Ok(root
        .ok_or("accountError.home")?
        .join("devin/credentials.toml"))
}
fn database(dir: &Path, writable: bool) -> Result<Connection, String> {
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(dir.join("User/globalStorage/state.vscdb"), flags)
        .map_err(|_| "accountError.initializeClient")?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|_| "accountError.read")?;
    Ok(db)
}
fn get(db: &Connection, key: &str) -> Result<Option<String>, String> {
    db.query_row("SELECT value FROM ItemTable WHERE key=?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .map_err(|_| "accountError.read".into())
}
fn secret_bytes(value: &str) -> Result<Vec<u8>, String> {
    if let Ok(value) = serde_json::from_str::<Value>(value) {
        if value["type"] == "Buffer" {
            return value["data"]
                .as_array()
                .ok_or("accountError.format")?
                .iter()
                .map(|n| {
                    n.as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or("accountError.format".into())
                })
                .collect();
        }
    }
    STANDARD
        .decode(value)
        .map_err(|_| "accountError.format".into())
}
fn decode_secret(dir: &Path, value: &str) -> Result<String, String> {
    let cipher = electron_storage::cipher_with_name(dir, "Devin")?;
    let bytes = electron_storage::decrypt_bytes(&cipher, &secret_bytes(value)?)?;
    String::from_utf8(bytes).map_err(|_| "accountError.format".into())
}
fn encode_secret(cipher: &electron_storage::Cipher, value: &str) -> Result<String, String> {
    Ok(
        json!({"type":"Buffer","data":electron_storage::encrypt_bytes(cipher, value.as_bytes())?})
            .to_string(),
    )
}
fn cached_user(cache: &Value) -> Result<Value, String> {
    if let Some(raw) = cache["userStatusJson"].as_str() {
        return serde_json::from_str(raw).map_err(|_| "accountError.format".into());
    }
    let bytes = STANDARD
        .decode(
            cache["userStatusProtoBinaryBase64"]
                .as_str()
                .ok_or("accountError.format")?,
        )
        .map_err(|_| "accountError.format")?;
    let f = wire::fields(&bytes).map_err(|_| "accountError.format")?;
    Ok(
        json!({"userId":wire::string(&f,36),"email":wire::string(&f,7),"name":wire::string(&f,3),"teamsTier":wire::number(&f,10)}),
    )
}
pub(super) fn read_desktop() -> Result<Option<(Credentials, Account)>, String> {
    read_desktop_at(&desktop_dir()?)
}
fn read_desktop_at(dir: &Path) -> Result<Option<(Credentials, Account)>, String> {
    if !dir.join("User/globalStorage/state.vscdb").exists() {
        return Ok(None);
    }
    let db = database(dir, false)?;
    let Some(raw) = get(&db, SESSION)? else {
        return Ok(None);
    };
    let sessions: Value =
        serde_json::from_str(&decode_secret(dir, &raw)?).map_err(|_| "accountError.format")?;
    let session = sessions.as_array().ok_or("accountError.format")?.first();
    let Some(session) = session else {
        return Ok(None);
    };
    let token = session["accessToken"]
        .as_str()
        .ok_or("accountError.format")?;
    let cache: Value = get(&db, STATUS)?
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| "accountError.format")?
        .unwrap_or(json!({}));
    if cache["apiKey"].as_str() != Some(token) {
        return Err("accountError.invalidAccount".into());
    }
    let server = get(&db, SERVER_KEY)?
        .map(|s| decode_secret(dir, &s))
        .transpose()?
        .unwrap_or_else(|| SERVER.into());
    super::from_user(token, &server, &cached_user(&cache)?, false).map(Some)
}
pub(super) fn cli_token() -> Result<Option<(String, String)>, String> {
    let path = cli_file()?;
    let Some(bytes) = native_account_files::read_optional(&path)? else {
        return Ok(None);
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| "accountError.format")?;
    let doc: toml_edit::DocumentMut = text.parse().map_err(|_| "accountError.format")?;
    let Some(token) = doc
        .get("windsurf_api_key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    Ok(Some((
        token.into(),
        doc.get("api_server_url")
            .and_then(|v| v.as_str())
            .unwrap_or(SERVER)
            .into(),
    )))
}
pub(super) fn apply_cli(c: &Credentials) -> Result<(), String> {
    apply_cli_at(&cli_file()?, c)
}
fn apply_cli_at(path: &Path, c: &Credentials) -> Result<(), String> {
    super::validate(&c.token, &c.server)?;
    let previous = native_account_files::read_optional(path)?;
    let mut doc: toml_edit::DocumentMut = match previous.as_ref() {
        Some(bytes) => std::str::from_utf8(bytes)
            .map_err(|_| "accountError.format")?
            .parse()
            .map_err(|_| "accountError.format")?,
        None => toml_edit::DocumentMut::new(),
    };
    for (key, value) in [
        ("windsurf_api_key", c.token.as_str()),
        ("api_server_url", c.server.as_str()),
        ("devin_webapp_host", "app.devin.ai"),
        ("devin_api_url", "https://api.devin.ai"),
    ] {
        doc[key] = toml_edit::value(value);
    }
    native_account_files::write_private_file(path, doc.to_string().as_bytes())
}
pub(super) fn apply_desktop(c: &Credentials) -> Result<(), String> {
    apply_desktop_at(&desktop_dir()?, c)
}
fn apply_desktop_at(dir: &Path, c: &Credentials) -> Result<(), String> {
    super::validate(&c.token, &c.server)?;
    let cipher = electron_storage::cipher_with_name(dir, "Devin")?;
    let sessions = json!([{"id":uuid::Uuid::new_v4().to_string(),"accessToken":c.token,"account":{"id":c.user_id,"label":c.email},"scopes":[]}]);
    let encrypted = encode_secret(&cipher, &sessions.to_string())?;
    let server = encode_secret(&cipher, &c.server)?;
    let status = json!({"apiKey":c.token,"userStatusJson":c.user.to_string()});
    let mut db = database(dir, true)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| "accountError.write")?;
    for (key, value) in [
        (SESSION, encrypted),
        (SERVER_KEY, server),
        (STATUS, status.to_string()),
    ] {
        tx.execute(
            "INSERT OR REPLACE INTO ItemTable (key,value) VALUES (?1,?2)",
            [key, &value],
        )
        .map_err(|_| "accountError.write")?;
    }
    tx.commit().map_err(|_| "accountError.write".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    fn fixture_state(root: &Path) {
        use windows::Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::Cryptography::{
                CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
            },
        };
        let key = [37_u8; 32];
        let input = CRYPT_INTEGER_BLOB {
            cbData: 32,
            pbData: key.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let wrapped = unsafe {
            CryptProtectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .unwrap();
            assert!(!output.pbData.is_null());
            let mut bytes = b"DPAPI".to_vec();
            bytes.extend(std::slice::from_raw_parts(
                output.pbData,
                output.cbData as usize,
            ));
            let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
            bytes
        };
        std::fs::write(
            root.join("Local State"),
            json!({"os_crypt":{"encrypted_key":STANDARD.encode(wrapped)}}).to_string(),
        )
        .unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn desktop_switch_roundtrip_preserves_unrelated_sqlite_records_and_cli() {
        let root =
            std::env::temp_dir().join(format!("echobird-devin-fixture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("User/globalStorage")).unwrap();
        fixture_state(&root);
        let db = Connection::open(root.join("User/globalStorage/state.vscdb")).unwrap();
        db.execute_batch("CREATE TABLE ItemTable(key TEXT PRIMARY KEY,value TEXT); INSERT INTO ItemTable VALUES('workspace','retained');").unwrap();
        let cli = root.join("credentials.toml");
        std::fs::write(&cli, "windsurf_api_key='cli-original'").unwrap();
        let user = json!({"userId":"fixture-user","email":"fixture@example.test","teamsTier":17});
        let (c, _) =
            super::super::from_user("devin-session-token$fixture", SERVER, &user, false).unwrap();
        apply_desktop_at(&root, &c).unwrap();
        let (actual, a) = read_desktop_at(&root).unwrap().unwrap();
        assert!(actual.token == c.token && actual.user_id == c.user_id);
        assert_eq!(a.plan.as_deref(), Some("Max"));
        assert_eq!(get(&db, "workspace").unwrap().as_deref(), Some("retained"));
        assert_eq!(
            std::fs::read_to_string(&cli).unwrap(),
            "windsurf_api_key='cli-original'"
        );
        let before = get(&db, SESSION).unwrap();
        let mut invalid = c.clone();
        invalid.token = "invalid".into();
        assert!(apply_desktop_at(&root, &invalid).is_err());
        assert_eq!(get(&db, SESSION).unwrap(), before);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn desktop_cached_proto_identity_and_json_are_equivalent() {
        let proto = wire::Message::default()
            .bytes(36, "id")
            .bytes(7, "test@example.test")
            .bytes(3, "Test")
            .num(10, 19)
            .0;
        let u =
            cached_user(&json!({"userStatusProtoBinaryBase64":STANDARD.encode(proto)})).unwrap();
        assert_eq!(u["userId"], "id");
        assert_eq!(u["teamsTier"], 19);
        assert_eq!(
            cached_user(&json!({"userStatusJson":u.to_string()})).unwrap(),
            u
        );
        assert!(cached_user(&json!({"userStatusProtoBinaryBase64":"invalid"})).is_err());
    }
    #[test]
    #[ignore = "Requires an installed, signed-in Devin desktop; reads only native credentials"]
    fn installed_desktop_read_only_positive_control() {
        let (c, a) = read_desktop()
            .expect("native credential decode")
            .expect("signed-in desktop");
        assert!(c.token.starts_with("devin-session-token$"));
        assert!(!c.user_id.is_empty() && !a.email.is_empty());
    }
    #[tokio::test]
    #[ignore = "Explicit read-only network verification with an installed Devin account"]
    async fn installed_account_status_and_models_read_only() {
        let (c, a) = read_desktop()
            .expect("native credential decode")
            .expect("signed-in desktop");
        let user = super::super::user_status(&c.token, &c.server)
            .await
            .expect("user status");
        let (_, fresh) =
            super::super::from_user(&c.token, &c.server, &user, true).expect("verified identity");
        assert!(fresh.id == a.id);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let models = crate::services::account_proxy::list_models(
            crate::models::model::AccountProvider::Devin,
            json!({"accessToken":c.token,"baseUrl":c.server}),
            &client,
        )
        .await
        .expect("model directory");
        assert!(!models.is_empty());
    }
    #[test]
    fn cli_switch_preserves_other_keys_and_rejects_corrupt_toml() {
        let root = std::env::temp_dir().join(format!("devin-native-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("credentials.toml");
        std::fs::write(
            &path,
            "# retained\nother = true\nwindsurf_api_key = 'old'\n",
        )
        .unwrap();
        let c = Credentials {
            token: "devin-session-token$fixture".into(),
            server: SERVER.into(),
            user_id: "id".into(),
            email: "fixture@example.test".into(),
            user: json!({}),
        };
        apply_cli_at(&path, &c).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# retained"));
        assert!(text.contains("other = true"));
        assert!(text.contains(&c.token));
        std::fs::write(&path, "[broken").unwrap();
        assert!(apply_cli_at(&path, &c).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[broken");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    #[ignore = "Explicit minimal inference smoke test with an installed account and an available model"]
    async fn live_proxy_available_model_stream() {
        use crate::{models::model::AccountProvider, services::account_proxy};
        let (c, _) = read_desktop()
            .expect("native credential decode")
            .expect("signed-in desktop");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let source = "devin-live-protocol-check";
        let credentials =
            json!({"accessToken":c.token,"baseUrl":c.server,"_echobird_saved_account_id":source});
        let models =
            account_proxy::list_models(AccountProvider::Devin, credentials.clone(), &client)
                .await
                .expect("model directory");
        let model = models.first().expect("available model");
        let response=account_proxy::route(AccountProvider::Devin,credentials,model,json!({"stream":true,"max_tokens":64,"messages":[{"role":"user","content":"Reply with OK."}]}),&client).await.expect("inference request");
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("stream read");
        let text = std::str::from_utf8(&bytes).expect("stream encoding");
        assert!(!text.contains("\"error\""), "upstream rejected inference");
        assert!(
            text.contains("\"finish_reason\":\"stop\"")
                || text.contains("\"finish_reason\":\"length\"")
        );
        assert!(text.ends_with("data: [DONE]\n\n"));
    }
}
