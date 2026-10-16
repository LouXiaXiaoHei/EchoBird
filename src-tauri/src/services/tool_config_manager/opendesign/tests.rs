use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn fixture() -> (Snapshot, NativePaths) {
    let root = Snapshot(
        std::env::temp_dir().join(format!("echobird-od-model-test-{}", uuid::Uuid::new_v4())),
    );
    let paths = namespace_paths(&root.0, "release-fixture").unwrap();
    files::private_dir(&paths.storage).unwrap();
    let mut db = rusty_leveldb::DB::open(&paths.storage, Default::default()).unwrap();
    let config = json!({
        "mode":"daemon", "agentId":"amr", "theme":"dark", "customInstructions":"keep",
        "apiProtocolConfigs":{"google":{"apiKey":"keep-google"},"openai":{"apiVersion":"keep-version"}},
        "mediaProviders":{"image":{"apiKey":"keep-media"}}
    });
    db.put(KEY, &encode(&config).unwrap()).unwrap();
    db.put(b"_od://app\0\x01unrelated", b"\x01keep").unwrap();
    db.flush().unwrap();
    files::write(
        &paths.preferences,
        br#"{"agentId":"amr","installationId":"keep-id","telemetry":{"metrics":false}}"#,
    )
    .unwrap();
    (root, paths)
}

fn model(protocol: &str) -> ModelInfo {
    serde_json::from_value(json!({
        "model":"fixture-model", "protocol":protocol, "baseUrl":"https://api.example/v1/",
        "anthropicUrl":"https://api.example/anthropic/", "apiKey":"fixture-secret"
    }))
    .unwrap()
}

fn fingerprints(path: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(path)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|entry| entry.file_name() != "LOCK")
        })
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                Sha256::digest(fs::read(entry.path()).unwrap()).to_vec(),
            )
        })
        .collect()
}

#[test]
fn both_protocols_select_api_mode_and_keep_other_profiles_and_preferences() {
    for protocol in ["openai", "anthropic"] {
        let (_root, paths) = fixture();
        let prefs = fs::read(&paths.preferences).unwrap();
        apply_at(&paths, Some(&model(protocol))).unwrap();
        let config = read_at(&paths.storage).unwrap();
        assert_eq!(config["mode"], "api");
        assert_eq!(config["apiProtocol"], protocol);
        assert_eq!(config["model"], "fixture-model");
        assert_eq!(config["apiKey"], "fixture-secret");
        assert_eq!(
            config["baseUrl"],
            if protocol == "openai" {
                "https://api.example/v1"
            } else {
                "https://api.example/anthropic"
            }
        );
        assert_eq!(
            config["apiProtocolConfigs"][protocol]["model"],
            "fixture-model"
        );
        assert_eq!(
            config["apiProtocolConfigs"]["google"]["apiKey"],
            "keep-google"
        );
        assert_eq!(
            config["apiProtocolConfigs"]["openai"]["apiVersion"],
            "keep-version"
        );
        assert_eq!(config["theme"], "dark");
        assert_eq!(config["mediaProviders"]["image"]["apiKey"], "keep-media");
        assert_eq!(fs::read(&paths.preferences).unwrap(), prefs);
        let mut db = open_storage(&paths.storage).unwrap();
        assert_eq!(
            db.get(b"_od://app\0\x01unrelated").unwrap().as_ref(),
            b"\x01keep"
        );
    }
}

#[test]
fn official_restore_updates_both_agent_selections_and_retains_api_configuration() {
    let (_root, paths) = fixture();
    apply_at(&paths, Some(&model("openai"))).unwrap();
    let api_profile = read_at(&paths.storage).unwrap()["apiProtocolConfigs"].clone();
    let mut prefs = files::object(&paths.preferences).unwrap();
    prefs["agentId"] = json!("codex");
    files::write(&paths.preferences, &files::json(&prefs).unwrap()).unwrap();
    apply_at(&paths, None).unwrap();
    let config = read_at(&paths.storage).unwrap();
    assert_eq!(config["mode"], "daemon");
    assert_eq!(config["agentId"], "amr");
    assert_eq!(config["apiProtocolConfigs"], api_profile);
    let prefs = files::object(&paths.preferences).unwrap();
    assert_eq!(prefs["agentId"], "amr");
    assert_eq!(prefs["installationId"], "keep-id");
    assert_eq!(prefs["telemetry"]["metrics"], false);
}

#[test]
fn passive_read_preserves_native_files_and_locked_storage_rejects_model_writes() {
    let (_root, paths) = fixture();
    let mut owner = open_storage(&paths.storage).unwrap();
    let before = fingerprints(&paths.storage);
    let prefs = fs::read(&paths.preferences).unwrap();
    assert_eq!(read_at(&paths.storage).unwrap()["mode"], "daemon");
    assert_eq!(fingerprints(&paths.storage), before);
    assert_eq!(fs::read(&paths.preferences).unwrap(), prefs);
    assert!(apply_at(&paths, Some(&model("openai"))).is_err());
    // An explicit failed open may rotate LevelDB's diagnostic LOG; settings must stay intact.
    assert_eq!(decode(&owner.get(KEY).unwrap()).unwrap()["mode"], "daemon");
    assert_eq!(fs::read(&paths.preferences).unwrap(), prefs);
}

#[test]
fn invalid_inputs_missing_key_or_malformed_preferences_do_not_replace_settings() {
    let (_root, paths) = fixture();
    let before = fingerprints(&paths.storage);
    assert!(apply_at(&paths, Some(&model("other"))).is_err());
    let mut invalid = model("openai");
    invalid.base_url = Some("https://user:secret@example.test/v1".into());
    assert!(apply_at(&paths, Some(&invalid)).is_err());
    assert_eq!(fingerprints(&paths.storage), before);
    files::write(&paths.preferences, b"not-json").unwrap();
    assert!(apply_at(&paths, None).is_err());
    assert_eq!(read_at(&paths.storage).unwrap()["mode"], "daemon");
    let mut db = open_storage(&paths.storage).unwrap();
    db.delete(KEY).unwrap();
    db.flush().unwrap();
    drop(db);
    assert!(apply_at(&paths, Some(&model("openai"))).is_err());
    assert!(read_at(&paths.storage).is_err());
    let missing = paths.storage.join("not-created");
    assert!(read_at(&missing).is_err());
    assert!(!missing.exists());
}

#[test]
fn chromium_encodings_and_native_package_paths_match_the_desktop_contract() {
    assert_eq!(
        decode(b"\x01{\"mode\":\"daemon\"}").unwrap()["mode"],
        "daemon"
    );
    let unicode = json!({"customInstructions":"保留中文 🎨", "mode":"api"});
    assert_eq!(decode(&encode(&unicode).unwrap()).unwrap(), unicode);
    assert!(decode(b"\x00x").is_err());
    assert!(decode(b"\x01[]").is_err());
    assert_eq!(
        packaged_config_path(Path::new("/program/Open Design.exe"), false).unwrap(),
        Path::new("/program/resources/open-design-config.json")
    );
    assert_eq!(
        packaged_config_path(
            Path::new("/Applications/Open Design.app/Contents/MacOS/Open Design"),
            true
        )
        .unwrap(),
        Path::new("/Applications/Open Design.app/Contents/Resources/open-design-config.json")
    );
    let linux = namespace_paths(
        Path::new("/home/test/.config/Open Design/namespaces"),
        "source-desktop",
    )
    .unwrap();
    assert_eq!(linux.storage, Path::new("/home/test/.config/Open Design/namespaces/source-desktop/user-data/session/Local Storage/leveldb"));
    assert!(namespace_paths(Path::new("/native"), "../other").is_err());
}

#[test]
#[ignore = "machine-specific: requires an initialized Open Design desktop profile; read-only snapshot"]
fn real_opendesign_model_settings_are_read_without_native_writes() {
    let paths = native_paths().unwrap().expect("Open Design native profile");
    let before = fingerprints(&paths.storage);
    let config = read_at(&paths.storage).unwrap();
    assert!(config["mode"] == "api" || config["mode"] == "daemon");
    assert_eq!(fingerprints(&paths.storage), before);
}
