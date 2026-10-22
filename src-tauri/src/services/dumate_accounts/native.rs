use super::Credentials;
use crate::services::{
    native_account_files::{self, DirectoryLock, Transaction},
    process_manager,
};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn data_dir() -> Result<PathBuf, String> {
    let root = if cfg!(target_os = "macos") {
        dirs::home_dir()
            .ok_or("accountError.home")?
            .join("Library/Application Support")
    } else {
        dirs::config_dir().ok_or("accountError.home")?
    };
    Ok(root.join("qianfan-desktop-app"))
}
fn key(root: &Path) -> Result<Vec<u8>, String> {
    let key =
        std::fs::read(root.join(".cookie-key")).map_err(|_| "accountError.initializeClient")?;
    if key.len() != 32 {
        return Err("accountError.initializeClient".into());
    }
    Ok(key)
}
fn decrypt(value: &str, key: &[u8]) -> Result<Vec<Value>, String> {
    let raw = STANDARD.decode(value).map_err(|_| "accountError.format")?;
    if raw.len() < 28 {
        return Err("accountError.format".into());
    }
    let mut payload = raw[28..].to_vec();
    payload.extend_from_slice(&raw[12..28]);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "accountError.keychain")?;
    let plain = cipher
        .decrypt(Nonce::from_slice(&raw[..12]), payload.as_slice())
        .map_err(|_| "accountError.keychain")?;
    serde_json::from_slice(&plain).map_err(|_| "accountError.format".into())
}
fn encrypt(cookies: &[Value], key: &[u8]) -> Result<String, String> {
    let mut iv = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut iv);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "accountError.keychain")?;
    let raw = cipher
        .encrypt(
            Nonce::from_slice(&iv),
            serde_json::to_vec(cookies)
                .map_err(|_| "accountError.format")?
                .as_slice(),
        )
        .map_err(|_| "accountError.keychain")?;
    let at = raw.len() - 16;
    let mut out = iv.to_vec();
    out.extend_from_slice(&raw[at..]);
    out.extend_from_slice(&raw[..at]);
    Ok(STANDARD.encode(out))
}
pub(super) fn current() -> Result<Option<Credentials>, String> {
    super::api::installed()?;
    read_at(&data_dir()?)
}
fn read_at(root: &Path) -> Result<Option<Credentials>, String> {
    let value = native_account_files::object(&root.join("auth.json"))?;
    let profile = value["accountProfiles"]
        .as_array()
        .and_then(|profiles| {
            profiles
                .iter()
                .find(|profile| profile["profileId"] == value["activeProfileId"])
        })
        .cloned();
    let Some(profile) = profile else {
        return Ok(None);
    };
    let encrypted = value["cookies"]
        .as_str()
        .or_else(|| profile["encryptedCookies"].as_str())
        .ok_or("accountError.loginRequired")?;
    let cookies = decrypt(encrypted, &key(root)?)?;
    let c = Credentials {
        profile,
        cookies,
        gateway_token: None,
    };
    c.validate()?;
    Ok(Some(c))
}
pub(super) struct Update {
    transaction: Transaction,
    _lock: DirectoryLock,
    _apply: tokio::sync::OwnedMutexGuard<()>,
}
impl Update {
    pub(super) fn commit(self) {
        self.transaction.commit();
    }
}
fn write_at(c: &Credentials, root: &Path) -> Result<(Transaction, DirectoryLock), String> {
    c.validate()?;
    let key = key(root)?;
    let path = root.join("auth.json");
    let lock = DirectoryLock::acquire_existing(&path)?;
    let mut auth = native_account_files::object(&path)?;
    let mut profiles = auth["accountProfiles"]
        .as_array()
        .cloned()
        .ok_or("accountError.initializeClient")?;
    if let Some(old) = profiles
        .iter_mut()
        .find(|p| p["profileId"] == auth["activeProfileId"])
    {
        if auth["cookies"].is_string() {
            old["encryptedCookies"] = auth["cookies"].clone();
        }
    }
    let mut profile = c.profile.clone();
    let identity = |p: &&Value| {
        p["loginType"] == profile["loginType"]
            && p["bceUserId"] == profile["bceUserId"]
            && (if c.enterprise() {
                p["enterpriseApiBaseUrl"] == profile["enterpriseApiBaseUrl"]
            } else {
                p["bceAccountId"] == profile["bceAccountId"]
            })
    };
    let index = profiles.iter().position(|p| identity(&p));
    if let Some(index) = index {
        // Native metadata may have changed since the account was saved in
        // EchoBird. Replace authentication, while retaining its current values.
        let target = profile.as_object_mut().ok_or("accountError.format")?;
        for (field, value) in profiles[index].as_object().ok_or("accountError.format")? {
            if !matches!(
                field.as_str(),
                "profileId"
                    | "loginType"
                    | "bceUserId"
                    | "bceAccountId"
                    | "encryptedCookies"
                    | "enterpriseApiBaseUrl"
                    | "vpcSvcUrl"
            ) {
                target.insert(field.clone(), value.clone());
            }
        }
    }
    profile["profileId"] = index
        .map(|i| profiles[i]["profileId"].clone())
        .unwrap_or_else(|| Value::String(uuid::Uuid::new_v4().to_string()));
    let encrypted = encrypt(&c.cookies, &key)?;
    profile["encryptedCookies"] = Value::String(encrypted.clone());
    profile["lastLogin"] = serde_json::json!(chrono::Utc::now().timestamp_millis());
    auth["activeProfileId"] = profile["profileId"].clone();
    auth["activeProviderType"] = profile["loginType"].clone();
    auth["cookies"] = Value::String(encrypted);
    for field in ["enterpriseApiBaseUrl", "vpcSvcUrl"] {
        if c.enterprise() && !profile[field].is_null() {
            auth[field] = profile[field].clone();
        } else {
            auth.as_object_mut()
                .ok_or("accountError.format")?
                .remove(field);
        }
    }
    auth.as_object_mut()
        .ok_or("accountError.format")?
        .remove("pendingLoginAt");
    if let Some(index) = index {
        profiles[index] = profile;
    } else {
        profiles.push(profile);
    }
    auth["accountProfiles"] = Value::Array(profiles);
    let mut transaction = Transaction::new();
    transaction.replace_private_file(&path, &native_account_files::json(&auth)?)?;
    Ok((transaction, lock))
}
pub(super) async fn apply(c: &Credentials) -> Result<Update, String> {
    static LOCK: std::sync::OnceLock<std::sync::Arc<tokio::sync::Mutex<()>>> =
        std::sync::OnceLock::new();
    let apply = LOCK
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
        .lock_owned()
        .await;
    super::api::installed()?;
    let root = data_dir()?;
    key(&root)?;
    process_manager::stop_desktop_for_account("dumate").await?;
    let (transaction, lock) = write_at(c, &root)?;
    Ok(Update {
        transaction,
        _lock: lock,
        _apply: apply,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Explicit read-only probe of local DuMate login and live quota; never claims or writes"]
    async fn installed_dumate_account_probe() {
        assert_eq!(
            std::env::var("ECHOBIRD_DUMATE_NATIVE_PROBE").as_deref(),
            Ok("1")
        );
        let c = read_at(&data_dir().unwrap())
            .unwrap()
            .expect("native signed-in profile");
        c.validate().unwrap();
        assert!(!c.enterprise());
        let client =
            super::super::api::Client::new("https://console.bce.baidu.com".into()).unwrap();
        client.verify(&c).await.unwrap();
        let mut account = c.summary();
        client.quota(&c).await.unwrap().apply(&mut account);
        let daily = client.daily(&c).await.unwrap();
        println!("DuMate native cookies decoded; identity verified; plan={:?}, remaining={:?}, buckets={}, daily={}",
            account.plan,account.remaining,account.credit_buckets.len(),daily.status);
    }
    #[test]
    fn cookie_cipher_matches_electron_layout_and_rejects_tampering() {
        let c = super::super::tests::fixture();
        let key = [7u8; 32];
        let value = encrypt(&c.cookies, &key).unwrap();
        assert_eq!(decrypt(&value, &key).unwrap(), c.cookies);
        assert!(decrypt(&value, &[8u8; 32]).is_err());
        assert!(decrypt("bad", &key).is_err());
    }
    #[test]
    fn native_switch_preserves_profiles_history_and_rolls_back() {
        let root = std::env::temp_dir().join(format!("dumate-native-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".cookie-key"), [7u8; 32]).unwrap();
        std::fs::write(root.join("tasks.db"), b"history").unwrap();
        let original = serde_json::json!({"unknown":42,"activeProfileId":"old","cookies":"old-cookie",
            "enterpriseApiBaseUrl":"https://old.test","accountProfiles":[{"profileId":"old","bceUserId":"other","loginType":"bce"}]});
        std::fs::write(
            root.join("auth.json"),
            serde_json::to_vec(&original).unwrap(),
        )
        .unwrap();
        let c = super::super::tests::fixture();
        let (t, l) = write_at(&c, &root).unwrap();
        assert!(write_at(&c, &root).is_err());
        drop(t);
        drop(l);
        assert_eq!(
            native_account_files::object(&root.join("auth.json")).unwrap(),
            original
        );
        let (t, l) = write_at(&c, &root).unwrap();
        t.commit();
        drop(l);
        let auth = native_account_files::object(&root.join("auth.json")).unwrap();
        assert_eq!(auth["unknown"], 42);
        assert_eq!(auth["accountProfiles"].as_array().unwrap().len(), 2);
        assert_eq!(auth["accountProfiles"][0]["encryptedCookies"], "old-cookie");
        assert!(auth.get("enterpriseApiBaseUrl").is_none());
        assert_eq!(read_at(&root).unwrap().unwrap().id(), c.id());
        assert_eq!(std::fs::read(root.join("tasks.db")).unwrap(), b"history");
        let mut updated = auth.clone();
        let profile_id = updated["activeProfileId"].clone();
        let target = updated["accountProfiles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|profile| profile["profileId"] == profile_id)
            .unwrap();
        target["clientMetadata"] = serde_json::json!({"revision":2});
        target["nativeOnly"] = serde_json::json!(true);
        std::fs::write(
            root.join("auth.json"),
            serde_json::to_vec(&updated).unwrap(),
        )
        .unwrap();
        let mut stale = c.clone();
        stale.profile["clientMetadata"] = serde_json::json!({"revision":1});
        let (t, l) = write_at(&stale, &root).unwrap();
        t.commit();
        drop(l);
        let merged = native_account_files::object(&root.join("auth.json")).unwrap();
        assert_eq!(merged["accountProfiles"].as_array().unwrap().len(), 2);
        let target = merged["accountProfiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["profileId"] == profile_id)
            .unwrap();
        assert_eq!(target["clientMetadata"]["revision"], 2);
        assert_eq!(target["nativeOnly"], true);
        assert_eq!(read_at(&root).unwrap().unwrap().cookies, c.cookies);
        std::fs::remove_file(root.join(".cookie-key")).unwrap();
        assert!(write_at(&c, &root).is_err());
        assert!(!root.join(".cookie-key").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
