//! Model configuration for workbuddy.

mod selection;

use super::{extract_domain_name, ApplyResult, ModelInfo};
use crate::services::{
    native_account_files as files, process_manager, tool_manager, workbuddy_accounts::Edition,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// ════════════════════════════════════════════════════════════════
//  WorkBuddy (Tencent CodeBuddy 办公版) → ~/.workbuddy/models.json
//  WorkBuddy AI → ~/.workbuddy-ai/models.json
//  Desktop UI shape: [{ id, name, vendor, url, apiKey, ... }].
//  Also read the legacy { models: [...], availableModels: [...] } shape
//  documented by DeepSeek for WorkBuddy/CodeBuddy.
//
//  Notes:
//   • Write to each desktop edition's own directory. The documented
//     ~/.codebuddy compatibility path is not the desktop UI's native file.
//   • Standard models use the full /chat/completions endpoint; existing
//     custom-protocol models retain their exact endpoint.
//   • File must be UTF-8 WITHOUT BOM — some desktop builds fail to parse a
//     BOM'd models.json. write_json_file writes raw UTF-8 (no BOM), so this
//     is satisfied for free.
//   • Preserve other models and existing capability/reasoning settings.
//     New-task selection is stored separately from model registration.
// ════════════════════════════════════════════════════════════════

fn config_path(home: &Path, tool_id: &str) -> PathBuf {
    home.join(if tool_id == "workbuddyai" {
        ".workbuddy-ai"
    } else {
        ".workbuddy"
    })
    .join("models.json")
}

fn completion_url(base_url: &str) -> String {
    let url = base_url.trim_end_matches('/');
    if url.ends_with("/chat/completions") {
        url.to_string()
    } else {
        format!("{url}/chat/completions")
    }
}

fn read_models(path: &Path) -> Result<Vec<Value>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Failed to read {}: {e}", path.display())),
    };
    let content = content.trim_start_matches('\u{feff}').trim();
    if content.is_empty() {
        return Ok(Vec::new());
    }
    let mut config: Value =
        serde_json::from_str(content).map_err(|e| format!("Invalid WorkBuddy models.json: {e}"))?;
    if config.is_object() {
        config = config["models"].take();
    }
    match config {
        Value::Array(models) if models.iter().all(Value::is_object) => Ok(models),
        _ => Err("Invalid WorkBuddy models.json: expected a model array".to_string()),
    }
}

fn edition(tool_id: &str) -> Edition {
    if tool_id == "workbuddyai" {
        Edition::Ai
    } else {
        Edition::Cn
    }
}

fn active_user_id(path: &Path) -> Result<Option<String>, String> {
    let mut marker = path.as_os_str().to_os_string();
    marker.push(".logged-out");
    if Path::new(&marker).exists() {
        return Ok(None);
    }
    let Some(bytes) =
        files::read_optional(path).map_err(|_| "Cannot read WorkBuddy account selection")?
    else {
        return Ok(None);
    };
    let session: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid WorkBuddy account selection")?;
    Ok(session["account"]["uid"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(String::from))
}

fn storage_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap()
        .join("app/session/Local Storage/leveldb")
}

pub(super) async fn apply_workbuddy(tool_id: &str, model_info: &ModelInfo) -> ApplyResult {
    let result = async {
        let home = dirs::home_dir().ok_or("Cannot find the home directory")?;
        let path = config_path(&home, tool_id);
        prepare_models(&path, model_info)?;
        if tool_manager::get_tool_exe_path(tool_id).is_none() {
            return Err("WorkBuddy desktop app is not installed".to_string());
        }
        if !storage_path(&path).join("CURRENT").is_file() {
            return Err("Open WorkBuddy once before configuring its model".to_string());
        }
        let auth_path = edition(tool_id).auth_path()?;
        active_user_id(&auth_path)?;
        process_manager::stop_desktop_for_config(tool_id).await;
        if process_manager::desktop_tool_is_running(tool_id).await? {
            return Err("Fully quit WorkBuddy before configuring its model".to_string());
        }
        let user_id = active_user_id(&auth_path)?;
        Ok(apply_workbuddy_at(&path, user_id.as_deref(), model_info))
    }
    .await;
    match result {
        Ok(result) => result,
        Err(message) => ApplyResult {
            success: false,
            message,
        },
    }
}

fn prepare_models(config_path: &Path, model_info: &ModelInfo) -> Result<Vec<Value>, String> {
    let model_id = model_info
        .model
        .as_deref()
        .or(model_info.name.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or("echobird-model");
    let display_name = model_info
        .name
        .as_deref()
        .or(model_info.model.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or(model_id);

    let base_url = model_info.base_url.as_deref().unwrap_or("");
    let api_key = model_info.api_key.as_deref().unwrap_or("");
    if base_url.is_empty() || api_key.is_empty() {
        return Err(
            "WorkBuddy needs both a base URL and an API key — pick a model first.".to_string(),
        );
    }

    let url = completion_url(base_url);

    let mut models = read_models(config_path)?;
    // The same model ID can be served by different providers. Only update
    // the matching endpoint, leaving the other provider's entry untouched.
    let existing = models.iter().position(|m| {
        m["id"].as_str() == Some(model_id)
            && m["url"].as_str().is_some_and(|existing_url| {
                if m["useCustomProtocol"] == true {
                    existing_url == base_url || existing_url == url
                } else {
                    completion_url(existing_url) == url
                }
            })
    });
    let mut model = existing.map(|i| models.remove(i)).unwrap_or_else(|| {
        json!({
            "id": model_id,
            "vendor": extract_domain_name(base_url),
            "maxInputTokens": 200000,
            "maxOutputTokens": 8192,
            "supportsToolCall": true,
            "supportsImages": true,
        })
    });
    model["name"] = json!(display_name);
    if model["useCustomProtocol"] != true {
        model["url"] = json!(url);
        model["useCustomProtocol"] = json!(false);
    }
    model["apiKey"] = json!(api_key);
    model["disabled"] = json!(false);
    for flag in ["supportsReasoning", "onlyReasoning"] {
        if !model[flag].is_boolean() {
            model[flag] = json!(false);
        }
    }
    // The native catalog merges duplicate IDs in order, with the last entry winning.
    models.push(model);
    Ok(models)
}

fn apply_workbuddy_at(
    config_path: &Path,
    user_id: Option<&str>,
    model_info: &ModelInfo,
) -> ApplyResult {
    let result = (|| -> Result<String, String> {
        let _lock = files::DirectoryLock::acquire(config_path)?;
        let models = prepare_models(config_path, model_info)?;
        let model = models.last().unwrap();
        let mut transaction = files::Transaction::new();
        transaction.replace_private_file(config_path, &files::json(&json!(models))?)?;
        let selection = selection::select(
            &storage_path(config_path),
            user_id,
            model["id"].as_str().unwrap(),
            model["onlyReasoning"] == true,
        )?;
        let name = model["name"].as_str().unwrap().to_string();
        selection.commit();
        transaction.commit();
        Ok(name)
    })();
    match result {
        Ok(display_name) => ApplyResult {
            success: true,
            message: format!(
                "Model \"{}\" selected for new WorkBuddy tasks.",
                display_name
            ),
        },
        Err(e) => ApplyResult {
            success: false,
            message: format!("WorkBuddy error: {}", e),
        },
    }
}

pub(super) fn read_workbuddy(tool_id: &str) -> Option<ModelInfo> {
    tool_manager::get_tool_exe_path(tool_id)?;
    let user_id = active_user_id(&edition(tool_id).auth_path().ok()?).ok()?;
    read_workbuddy_at(
        &config_path(&dirs::home_dir()?, tool_id),
        user_id.as_deref(),
    )
}

fn read_workbuddy_at(path: &Path, user_id: Option<&str>) -> Option<ModelInfo> {
    let models = read_models(path).ok()?;
    let selected = selection::read(&storage_path(path), user_id).ok()??;
    let m = models.iter().rev().find(|m| {
        m["id"]
            .as_str()
            .is_some_and(|id| selection::native_id(id) == selected)
    })?;
    if m["disabled"] == true {
        return None;
    }
    let model = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if model.is_empty() {
        return None;
    }
    let url = m.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let base_url = if m["useCustomProtocol"] == true {
        url.to_string()
    } else {
        url.trim_end_matches('/')
            .trim_end_matches("/chat/completions")
            .trim_end_matches('/')
            .to_string()
    };
    Some(ModelInfo {
        name: m.get("name").and_then(|v| v.as_str()).map(String::from),
        model: Some(model.to_string()),
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url)
        },
        api_key: m.get("apiKey").and_then(|v| v.as_str()).map(String::from),
        anthropic_url: None,
        protocol: None,
        display_model: None,
        relay_mode: None,
        web_search: None,
        image_input: None,
        one_m_context: None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::write_json_file;
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("workbuddy-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self, tool_id: &str) -> PathBuf {
            let path = config_path(&self.0, tool_id);
            let storage = storage_path(&path);
            std::fs::create_dir_all(&storage).unwrap();
            let mut db = rusty_leveldb::DB::open(&storage, Default::default()).unwrap();
            db.flush().unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn model() -> ModelInfo {
        serde_json::from_value(json!({
            "model": "test-model",
            "name": "Test model",
            "baseUrl": "https://example.test/v1/",
            "apiKey": "test-key"
        }))
        .unwrap()
    }

    #[test]
    fn writes_native_arrays_and_keeps_editions_separate() {
        let fixture = Fixture::new();
        let cn = fixture.path("workbuddy");
        let ai = fixture.path("workbuddyai");
        assert_eq!(cn, fixture.0.join(".workbuddy/models.json"));
        assert_eq!(ai, fixture.0.join(".workbuddy-ai/models.json"));
        for path in [&cn, &ai] {
            assert!(apply_workbuddy_at(path, Some("fixture-user"), &model()).success);
            let bytes = std::fs::read(path).unwrap();
            assert_eq!(bytes[0], b'[');
            let entries = read_models(path).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(
                entries[0]["url"],
                "https://example.test/v1/chat/completions"
            );
            let actual = read_workbuddy_at(path, Some("fixture-user")).unwrap();
            assert_eq!(actual.model, model().model);
            assert_eq!(actual.api_key, model().api_key);
            assert_eq!(actual.base_url.as_deref(), Some("https://example.test/v1"));
        }
        let cn_bytes = std::fs::read(&cn).unwrap();
        let mut changed = model();
        changed.api_key = Some("ai-only-key".into());
        assert!(apply_workbuddy_at(&ai, Some("fixture-user"), &changed).success);
        assert_eq!(std::fs::read(&cn).unwrap(), cn_bytes);
    }

    #[test]
    fn updates_matching_model_preserving_other_providers_and_reasoning() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        let other = json!({
            "id": "test-model", "url": "https://other.test/v1/chat/completions",
            "apiKey": "other-key", "name": "Other provider"
        });
        let existing = json!({
            "id": "test-model", "url": "https://example.test/v1/chat/completions",
            "apiKey": "old-key", "name": "Old name", "vendor": "Custom",
            "supportsImages": false, "supportsToolCall": true,
            "supportsReasoning": true, "onlyReasoning": false,
            "reasoning": {"effort": "high"}, "useCustomProtocol": false,
            "maxInputTokens": 128000, "maxOutputTokens": 16384
        });
        write_json_file(&path, &json!([other, existing])).unwrap();
        let mut info = model();
        info.base_url = Some("https://example.test/v1/chat/completions".into());
        assert!(apply_workbuddy_at(&path, Some("fixture-user"), &info).success);
        assert!(apply_workbuddy_at(&path, Some("fixture-user"), &info).success);
        let entries = read_models(&path).unwrap();
        let mut expected = existing;
        expected["name"] = json!("Test model");
        expected["apiKey"] = json!("test-key");
        expected["disabled"] = json!(false);
        assert_eq!(entries, vec![other, expected]);
    }

    #[test]
    fn reads_legacy_models_and_migrates_without_losing_entries() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        let legacy = json!({"id": "legacy-model", "apiKey": "legacy-key"});
        write_json_file(
            &path,
            &json!({
                "models": [legacy], "availableModels": ["legacy-model"]
            }),
        )
        .unwrap();
        assert!(read_workbuddy_at(&path, Some("fixture-user")).is_none());
        selection::select(
            &storage_path(&path),
            Some("fixture-user"),
            "legacy-model",
            false,
        )
        .unwrap()
        .commit();
        assert_eq!(
            read_workbuddy_at(&path, Some("fixture-user"))
                .unwrap()
                .model
                .as_deref(),
            Some("legacy-model")
        );
        assert!(apply_workbuddy_at(&path, Some("fixture-user"), &model()).success);
        assert_eq!(std::fs::read(&path).unwrap()[0], b'[');
        assert_eq!(read_models(&path).unwrap()[0], legacy);
    }

    #[test]
    fn custom_protocol_endpoints_round_trip_without_losing_settings() {
        for url in [
            "https://example.test/gateway/invoke/",
            "https://example.test/v1/chat/completions",
        ] {
            let fixture = Fixture::new();
            let path = fixture.path("workbuddy");
            let existing = json!({
                "id": "test-model", "url": url, "apiKey": "old-key",
                "useCustomProtocol": true, "supportsReasoning": true,
                "maxInputTokens": 262144, "maxOutputTokens": 65536
            });
            write_json_file(&path, &json!([existing])).unwrap();
            // The existing configuration is registered but has not been selected yet.
            assert!(read_workbuddy_at(&path, Some("fixture-user")).is_none());
            selection::select(
                &storage_path(&path),
                Some("fixture-user"),
                "test-model",
                false,
            )
            .unwrap()
            .commit();
            let mut info = read_workbuddy_at(&path, Some("fixture-user")).unwrap();
            assert_eq!(info.base_url.as_deref(), Some(url));
            info.api_key = Some("new-key".into());
            assert!(apply_workbuddy_at(&path, Some("fixture-user"), &info).success);
            let mut expected = existing;
            expected["name"] = json!("test-model");
            expected["apiKey"] = json!("new-key");
            expected["disabled"] = json!(false);
            expected["onlyReasoning"] = json!(false);
            assert_eq!(read_models(&path).unwrap(), vec![expected]);
        }
    }

    #[test]
    fn standard_base_urls_match_the_completed_endpoint() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        write_json_file(
            &path,
            &json!([{
                "id": "test-model", "url": "https://example.test/v1/",
                "maxOutputTokens": 65536
            }]),
        )
        .unwrap();
        assert!(apply_workbuddy_at(&path, Some("fixture-user"), &model()).success);
        let models = read_models(&path).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["maxOutputTokens"], 65536);
        assert_eq!(models[0]["url"], "https://example.test/v1/chat/completions");
    }

    #[test]
    fn accepts_empty_files_and_utf8_bom() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddyai");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for content in ["", " \n", "\u{feff}[]"] {
            std::fs::write(&path, content).unwrap();
            assert!(apply_workbuddy_at(&path, Some("fixture-user"), &model()).success);
            assert_eq!(std::fs::read(&path).unwrap()[0], b'[');
        }
    }

    #[test]
    fn malformed_or_unknown_configs_are_not_overwritten() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for content in ["{broken", "{}", "null", "[null]", r#"{"models":{}}"#] {
            std::fs::write(&path, content).unwrap();
            assert!(!apply_workbuddy_at(&path, Some("fixture-user"), &model()).success);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        }
    }

    #[test]
    fn missing_credentials_leave_existing_config_untouched() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        write_json_file(&path, &json!([])).unwrap();
        for info in [
            ModelInfo {
                api_key: None,
                ..model()
            },
            ModelInfo {
                base_url: None,
                ..model()
            },
        ] {
            assert!(!apply_workbuddy_at(&path, Some("fixture-user"), &info).success);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]");
        }
    }

    #[test]
    fn reads_the_native_selection_instead_of_the_first_registered_model() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        write_json_file(
            &path,
            &json!([
                {"id":"first", "url":"https://first.test/v1/chat/completions", "apiKey":"one"},
                {"id":"selected", "url":"https://selected.test/v1/chat/completions", "apiKey":"two"}
            ]),
        )
        .unwrap();
        assert!(read_workbuddy_at(&path, Some("fixture-user")).is_none());
        selection::select(
            &storage_path(&path),
            Some("fixture-user"),
            "selected",
            false,
        )
        .unwrap()
        .commit();
        let selected = read_workbuddy_at(&path, Some("fixture-user")).unwrap();
        assert_eq!(selected.model.as_deref(), Some("selected"));
        assert_eq!(
            selected.base_url.as_deref(),
            Some("https://selected.test/v1")
        );
        assert!(read_workbuddy_at(&path, Some("other-user")).is_none());
        std::fs::remove_file(&path).unwrap();
        assert!(read_workbuddy_at(&path, Some("fixture-user")).is_none());
    }

    #[test]
    fn explicit_apply_enables_only_the_target_and_honors_required_thinking() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        let other = json!({"id":"other", "disabled":true});
        write_json_file(&path, &json!([
            other,
            {"id":"test-model", "url":"https://example.test/v1", "disabled":true, "onlyReasoning":true, "supportsReasoning":true}
        ])).unwrap();
        assert!(apply_workbuddy_at(&path, Some("fixture-user"), &model()).success);
        let rows = read_models(&path).unwrap();
        assert_eq!(rows[1]["disabled"], false);
        assert_eq!(rows[0], other);
        let mut db = rusty_leveldb::DB::open(storage_path(&path), Default::default()).unwrap();
        let value = db
            .get(b"_file://\0\x01cb-newtask:model:fixture-user")
            .unwrap();
        let selected: Value = serde_json::from_slice(&value[1..]).unwrap();
        assert_eq!(
            selected,
            json!({"id":"custom-local:test-model", "isThinking":true})
        );
    }

    #[test]
    fn native_selection_failure_rolls_back_registration_without_false_success() {
        for failure in ["locked", "malformed", "missing"] {
            let fixture = Fixture::new();
            let path = fixture.path("workbuddy");
            let original = b"[{\"id\":\"existing\",\"unknown\":true}]";
            std::fs::write(&path, original).unwrap();
            let storage = storage_path(&path);
            let mut held = rusty_leveldb::DB::open(&storage, Default::default()).unwrap();
            if failure == "malformed" {
                held.put(
                    b"_file://\0\x01cb-newtask:model:fixture-user",
                    b"\x01not-json",
                )
                .unwrap();
                held.flush().unwrap();
            }
            if failure != "locked" {
                drop(held);
                if failure == "missing" {
                    std::fs::remove_file(storage.join("CURRENT")).unwrap();
                }
                assert!(!apply_workbuddy_at(&path, Some("fixture-user"), &model()).success);
            } else {
                assert!(!apply_workbuddy_at(&path, Some("fixture-user"), &model()).success);
                assert!(held
                    .get(b"_file://\0\x01cb-newtask:model:fixture-user")
                    .is_none());
            }
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
    }

    #[test]
    fn current_account_uid_and_logged_out_marker_scope_the_new_task_default() {
        let fixture = Fixture::new();
        let auth = fixture.0.join("workbuddy-desktop.info");
        assert_eq!(active_user_id(&auth).unwrap(), None);
        write_json_file(&auth, &json!({"account":{"uid":"current-user"}})).unwrap();
        assert_eq!(
            active_user_id(&auth).unwrap().as_deref(),
            Some("current-user")
        );
        let marker = fixture.0.join("workbuddy-desktop.info.logged-out");
        std::fs::write(&marker, b"logged-out").unwrap();
        assert_eq!(active_user_id(&auth).unwrap(), None);
        std::fs::remove_file(marker).unwrap();
        std::fs::write(&auth, b"{broken").unwrap();
        assert!(active_user_id(&auth).is_err());
    }

    #[test]
    fn duplicate_ids_use_the_requested_endpoint_in_the_native_last_wins_merge() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        let other = json!({"id":"test-model", "url":"https://other.test/invoke", "apiKey":"other-key", "disabled":true, "useCustomProtocol":true, "onlyReasoning":true, "supportsReasoning":true});
        let selected = json!({"id":"test-model", "url":"https://example.test/v1/chat/completions", "apiKey":"old-key"});
        write_json_file(&path, &json!([selected, other])).unwrap();
        assert!(apply_workbuddy_at(&path, Some("fixture-user"), &model()).success);
        let mut effective = serde_json::Map::new();
        let mut rows = read_models(&path).unwrap();
        for row in &rows {
            effective.extend(row.as_object().unwrap().clone());
        }
        assert_eq!(effective["url"], "https://example.test/v1/chat/completions");
        assert_eq!(effective["apiKey"], "test-key");
        for flag in [
            "disabled",
            "useCustomProtocol",
            "onlyReasoning",
            "supportsReasoning",
        ] {
            assert_eq!(effective[flag], false);
        }
        assert_eq!(
            read_workbuddy_at(&path, Some("fixture-user"))
                .unwrap()
                .api_key
                .as_deref(),
            Some("test-key")
        );
        rows.last_mut().unwrap()["disabled"] = json!(true);
        write_json_file(&path, &json!(rows)).unwrap();
        assert!(read_workbuddy_at(&path, Some("fixture-user")).is_none());
    }

    #[test]
    fn a_model_id_with_the_native_prefix_can_be_selected_and_read_back() {
        let fixture = Fixture::new();
        let path = fixture.path("workbuddy");
        let info = ModelInfo {
            model: Some("custom-local:prefixed".into()),
            ..model()
        };
        assert!(apply_workbuddy_at(&path, Some("fixture-user"), &info).success);
        assert_eq!(
            read_workbuddy_at(&path, Some("fixture-user"))
                .unwrap()
                .model,
            info.model
        );
        assert_eq!(
            selection::read(&storage_path(&path), Some("fixture-user"))
                .unwrap()
                .as_deref(),
            Some("custom-local:prefixed")
        );
    }

    #[test]
    fn international_and_domestic_users_keep_independent_new_task_defaults() {
        let fixture = Fixture::new();
        let cn = fixture.path("workbuddy");
        let ai = fixture.path("workbuddyai");
        let domestic = ModelInfo {
            model: Some("domestic-model".into()),
            ..model()
        };
        let international = ModelInfo {
            model: Some("international-model".into()),
            base_url: Some("https://international.test/v1".into()),
            api_key: Some("international-key".into()),
            ..model()
        };
        assert!(apply_workbuddy_at(&cn, Some("domestic-user"), &domestic).success);
        let cn_bytes = std::fs::read(&cn).unwrap();
        selection::select(
            &storage_path(&ai),
            Some("other-international-user"),
            "other-model",
            false,
        )
        .unwrap()
        .commit();
        selection::select(
            &storage_path(&ai),
            Some("international-user:template"),
            "template-model",
            true,
        )
        .unwrap()
        .commit();
        assert!(apply_workbuddy_at(&ai, Some("international-user"), &international).success);
        assert_eq!(std::fs::read(&cn).unwrap(), cn_bytes);
        assert_eq!(
            read_workbuddy_at(&cn, Some("domestic-user")).unwrap().model,
            domestic.model
        );
        let selected = read_workbuddy_at(&ai, Some("international-user")).unwrap();
        assert_eq!(selected.model, international.model);
        assert_eq!(selected.api_key, international.api_key);
        assert_eq!(
            selected.base_url.as_deref(),
            Some("https://international.test/v1")
        );
        assert!(read_workbuddy_at(&ai, Some("domestic-user")).is_none());
        assert!(read_workbuddy_at(&cn, Some("international-user")).is_none());
        assert_eq!(
            selection::read(&storage_path(&ai), Some("other-international-user"))
                .unwrap()
                .as_deref(),
            Some("custom-local:other-model")
        );
        assert_eq!(
            selection::read(&storage_path(&ai), Some("international-user:template"))
                .unwrap()
                .as_deref(),
            Some("custom-local:template-model")
        );
        assert_eq!(
            edition("workbuddyai")
                .auth_path()
                .unwrap()
                .file_name()
                .unwrap(),
            "workbuddy-desktop-ai.info"
        );
        assert_eq!(
            edition("workbuddy")
                .auth_path()
                .unwrap()
                .file_name()
                .unwrap(),
            "workbuddy-desktop.info"
        );
    }
}
