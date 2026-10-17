use super::*;
use std::collections::BTreeMap;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("echobird-codearts-config-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn desktop() -> Self {
        let fixture = Self::new();
        fs::write(fixture.0.join("app-settings.json"),json!({"fontScale":"large","customModels":[{"uid":"user-model","apiKey":"user-key","enabled":true}],"unknown":{"preserve":true}}).to_string()).unwrap();
        let mut db = rusty_leveldb::DB::open(storage(&fixture.0), Default::default()).unwrap();
        db.put(SELECTION, &encode_selection("official-fixture"))
            .unwrap();
        db.put(b"unrelated", b"preserve").unwrap();
        db.flush().unwrap();
        drop(db);
        fixture
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn model(protocol: &str) -> ModelInfo {
    serde_json::from_value(json!({"name":"Fixture","model":"vendor/fixture-model","baseUrl":"https://example.test/v1","anthropicUrl":"https://anthropic.example.test","apiKey":"fixture-secret","protocol":protocol})).unwrap()
}
fn encrypt_fixture(_: &Path, key: &str) -> Result<Vec<u8>, String> {
    Ok(format!("encrypted:{key}").into_bytes())
}
fn decrypt_fixture(_: &Path, bytes: &[u8]) -> Result<String, String> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .strip_prefix("encrypted:")
        .map(str::to_owned)
        .ok_or_else(|| "fixture encryption missing".into())
}

#[test]
fn desktop_round_trip_keeps_secrets_encrypted_and_reading_does_not_touch_native_leveldb() {
    let fixture = Fixture::desktop();
    configure_desktop_with(&fixture.0, Some(&model("anthropic")), encrypt_fixture)
        .unwrap()
        .commit();
    let config = files::object(&fixture.0.join("app-settings.json")).unwrap();
    assert_eq!(config["fontScale"], "large");
    assert_eq!(config["unknown"]["preserve"], true);
    assert_eq!(config["customModels"][0]["apiKey"], "user-key");
    let own = config["customModels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["uid"] == UID)
        .unwrap();
    assert_eq!(own["apiKey"], format!("${{input:chat.lm.secret.{UID}}}"));
    assert!(!fs::read_to_string(fixture.0.join("app-settings.json"))
        .unwrap()
        .contains("fixture-secret"));
    let before = fs::read_dir(storage(&fixture.0))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect::<BTreeMap<_, _>>();
    let info = read_desktop_with(&fixture.0, decrypt_fixture).unwrap();
    assert_eq!(info.model.as_deref(), Some("vendor/fixture-model"));
    assert_eq!(info.api_key.as_deref(), Some("fixture-secret"));
    assert_eq!(info.protocol.as_deref(), Some("anthropic"));
    let after = fs::read_dir(storage(&fixture.0))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(before, after);
    configure_desktop_with(&fixture.0, None, encrypt_fixture)
        .unwrap()
        .commit();
    let config = files::object(&fixture.0.join("app-settings.json")).unwrap();
    assert_eq!(config["customModels"].as_array().unwrap().len(), 1);
    assert!(!secret_path(&fixture.0).exists());
    assert!(desktop_selection(&fixture.0).unwrap().is_none());
    let mut db = open_storage(&storage(&fixture.0)).unwrap();
    assert_eq!(db.get(b"unrelated").unwrap().as_ref(), b"preserve");
}

#[test]
fn desktop_account_failure_restores_settings_secret_and_selection() {
    let fixture = Fixture::desktop();
    configure_desktop_with(&fixture.0, Some(&model("openai")), encrypt_fixture)
        .unwrap()
        .commit();
    let settings = fixture.0.join("app-settings.json");
    let old_settings = fs::read(&settings).unwrap();
    let old_secret = fs::read(secret_path(&fixture.0)).unwrap();
    let guard = configure_desktop_with(&fixture.0, None, encrypt_fixture).unwrap();
    assert!(!secret_path(&fixture.0).exists());
    drop(guard);
    assert_eq!(fs::read(settings).unwrap(), old_settings);
    assert_eq!(fs::read(secret_path(&fixture.0)).unwrap(), old_secret);
    assert_eq!(
        desktop_selection(&fixture.0).unwrap(),
        Some(format!("custom:{UID}"))
    );
}

#[test]
fn desktop_busy_storage_or_encryption_failure_does_not_write_settings_or_keys() {
    let fixture = Fixture::desktop();
    let settings = fixture.0.join("app-settings.json");
    let original = fs::read(&settings).unwrap();
    let busy = open_storage(&storage(&fixture.0)).unwrap();
    assert!(configure_desktop_with(&fixture.0, Some(&model("openai")), encrypt_fixture).is_err());
    drop(busy);
    assert_eq!(fs::read(&settings).unwrap(), original);
    assert!(!secret_path(&fixture.0).exists());
    fn unavailable(_: &Path, _: &str) -> Result<Vec<u8>, String> {
        Err("fixture-keychain-unavailable".into())
    }
    assert!(configure_desktop_with(&fixture.0, Some(&model("openai")), unavailable).is_err());
    assert_eq!(fs::read(&settings).unwrap(), original);
    assert!(!secret_path(&fixture.0).exists());
    assert_eq!(
        desktop_selection(&fixture.0).unwrap(),
        Some("official-fixture".into())
    );
}

#[test]
fn restoring_a_user_selected_model_preserves_it() {
    let fixture = Fixture::desktop();
    let settings = fixture.0.join("app-settings.json");
    let original = fs::read(&settings).unwrap();
    configure_desktop_with(&fixture.0, None, encrypt_fixture)
        .unwrap()
        .commit();
    assert_eq!(fs::read(settings).unwrap(), original);
    assert_eq!(
        desktop_selection(&fixture.0).unwrap(),
        Some("official-fixture".into())
    );
}

#[test]
fn local_ipv6_models_accept_an_empty_key_without_allowing_remote_endpoints() {
    let mut info = model("openai");
    info.api_key = Some(String::new());
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        info.base_url = Some(format!("http://{host}:8787/v1"));
        assert!(parameters(&info).is_ok());
    }
    info.base_url = Some("http://[2001:db8::1]:8787/v1".into());
    assert!(parameters(&info).is_err());
}

#[test]
fn known_anonymous_remote_models_round_trip_without_a_key() {
    let fixture = Fixture::desktop();
    let mut info = model("openai");
    info.base_url = Some("https://api.kilo.ai/api/gateway".into());
    info.model = Some("kilo-auto/free".into());
    info.api_key = None;
    assert!(crate::services::free_model_directory::is_keyless_model(
        info.base_url.as_deref().unwrap(),
        info.model.as_deref().unwrap()
    ));
    configure_desktop_with(&fixture.0, Some(&info), encrypt_fixture)
        .unwrap()
        .commit();
    let loaded = read_desktop_with(&fixture.0, decrypt_fixture).unwrap();
    assert_eq!(loaded.model, info.model);
    assert_eq!(loaded.base_url, info.base_url);
    assert_eq!(loaded.api_key.as_deref(), Some(""));
    assert!(!secret_path(&fixture.0).exists());
    for (base, model) in [
        ("https://api.kilo.ai/api/gateway", "not-a-free-model"),
        ("https://example.test/v1", "kilo-auto/free"),
        ("https://api.kilo.ai/other-path", "kilo-auto/free"),
        ("https://opencode.ai/inference/v1", "big-pickle"),
    ] {
        info.base_url = Some(base.into());
        info.model = Some(model.into());
        assert!(parameters(&info).is_err());
    }
}
