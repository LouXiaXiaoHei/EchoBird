use super::ModelInfo;
use serde_json::{json, Value};
use std::path::PathBuf;

const ONE_M: u64 = 1_000_000;

fn positive(value: &Value) -> Option<u64> {
    value.as_u64().filter(|value| *value > 0)
}

fn normalized_base(url: &str) -> &str {
    url.trim_end_matches('/').trim_end_matches("/v1")
}

pub(super) fn same_endpoint(provider: &Value, info: &ModelInfo) -> bool {
    provider["options"]["baseURL"].as_str().map(normalized_base)
        == Some(normalized_base(
            info.base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1"),
        ))
}

fn local_catalog() -> Option<Value> {
    let path = if let Some(path) = std::env::var_os("MIMOCODE_MODELS_PATH") {
        PathBuf::from(path)
    } else {
        let root = if let Some(home) = std::env::var_os("MIMOCODE_HOME").filter(|v| !v.is_empty()) {
            PathBuf::from(home).join("cache")
        } else {
            std::env::var_os("XDG_CACHE_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or(dirs::home_dir()?.join(".cache"))
                .join("mimocode")
        };
        root.join("models.json")
    };
    if !path.is_absolute() {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn catalog_output(catalog: &Value, info: &ModelInfo) -> Option<u64> {
    let model = info.model.as_deref()?;
    let base = info.base_url.as_deref().map(normalized_base);
    let mut matching_endpoint = Vec::new();
    let mut matching_id = Vec::new();
    for provider in catalog.as_object()?.values() {
        let Some(output) = provider["models"]
            .get(model)
            .and_then(|m| positive(&m["limit"]["output"]))
        else {
            continue;
        };
        matching_id.push(output);
        if base.is_some() && provider["api"].as_str().map(normalized_base) == base {
            matching_endpoint.push(output);
        }
    }
    let outputs = if matching_endpoint.is_empty() {
        matching_id
    } else {
        matching_endpoint
    };
    let Some(first) = outputs.first().copied() else {
        let (provider, id) = model.split_once('/')?;
        return catalog[provider]["models"]
            .get(id)
            .and_then(|m| positive(&m["limit"]["output"]));
    };
    outputs.iter().all(|value| *value == first).then_some(first)
}

pub(super) fn model_entry(info: &ModelInfo, previous: &Value) -> Value {
    let catalog = (info.one_m_context == Some(true)
        && positive(&previous["limit"]["output"]).is_none())
    .then(local_catalog)
    .flatten();
    configured_entry(info, previous, catalog.as_ref())
}

fn configured_entry(info: &ModelInfo, previous: &Value, catalog: Option<&Value>) -> Value {
    let mut entry = previous
        .as_object()
        .map(|_| previous.clone())
        .unwrap_or(json!({}));
    entry["name"] = json!(info.name.as_deref().or(info.model.as_deref()).unwrap_or(""));
    if let Some(image) = info.image_input {
        let mut modalities = entry["modalities"].as_object().cloned().unwrap_or_default();
        let mut inputs = modalities
            .get("input")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        inputs.retain(|value| value != "image");
        if !inputs.contains(&json!("text")) {
            inputs.insert(0, json!("text"));
        }
        if image {
            inputs.push(json!("image"));
        }
        modalities.insert("input".into(), json!(inputs));
        modalities.entry("output").or_insert(json!(["text"]));
        entry["modalities"] = json!(modalities);
    }
    match info.one_m_context {
        Some(true) => {
            if let Some(output) = positive(&entry["limit"]["output"])
                .or_else(|| catalog.and_then(|catalog| catalog_output(catalog, info)))
            {
                let mut limit = entry["limit"].as_object().cloned().unwrap_or_default();
                limit.insert("context".into(), json!(ONE_M));
                limit.insert("output".into(), json!(output));
                entry["limit"] = json!(limit);
            }
        }
        Some(false) if entry["limit"]["context"].as_u64() == Some(ONE_M) => {
            entry.as_object_mut().unwrap().remove("limit");
        }
        _ => {}
    }
    entry
}

pub(super) fn read_capabilities(info: &mut ModelInfo, entry: &Value) {
    info.image_input = entry["modalities"]["input"]
        .as_array()
        .map(|inputs| inputs.contains(&json!("image")));
    info.one_m_context = entry["limit"]["context"]
        .as_u64()
        .map(|context| context == ONE_M);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> ModelInfo {
        serde_json::from_value(json!({"model":"any-model","name":"Example","baseUrl":"https://api.example.test/v1","imageInput":true,"oneMContext":true})).unwrap()
    }

    #[test]
    fn local_catalog_supplies_output_without_model_specific_rules() {
        let catalog = json!({"vendor":{"api":"https://api.example.test","models":{"any-model":{"limit":{"output":64000}}}}});
        let entry = configured_entry(&info(), &json!({}), Some(&catalog));
        assert_eq!(entry["limit"], json!({"context":ONE_M,"output":64000}));
        let mut prefixed = info();
        prefixed.model = Some("vendor/any-model".into());
        assert_eq!(catalog_output(&catalog, &prefixed), Some(64000));
        assert_eq!(
            configured_entry(&info(), &json!({}), None).get("limit"),
            None
        );
    }

    #[test]
    fn unknown_or_ambiguous_catalog_entries_keep_native_defaults() {
        let catalog = json!({"a":{"models":{"any-model":{"limit":{"output":4000}}}},"b":{"models":{"any-model":{"limit":{"output":8000}}}}});
        assert!(configured_entry(&info(), &json!({}), Some(&catalog))
            .get("limit")
            .is_none());
        let mut unknown = info();
        unknown.model = Some("unlisted".into());
        assert_eq!(catalog_output(&catalog, &unknown), None);
    }

    #[test]
    fn endpoint_match_wins_over_other_providers_without_guessing() {
        let catalog = json!({"a":{"api":"https://api.example.test","models":{"any-model":{"limit":{"output":4000}}}},"b":{"api":"https://other.test/v1","models":{"any-model":{"limit":{"output":8000}}}}});
        assert_eq!(catalog_output(&catalog, &info()), Some(4000));
        let gateway = json!({"vendor":{"models":{"any-model":{"limit":{"output":64000}}}},"gateway":{"api":"https://api.example.test/v1","models":{"vendor/any-model":{"limit":{"output":16000}}}}});
        let mut prefixed = info();
        prefixed.model = Some("vendor/any-model".into());
        assert_eq!(catalog_output(&gateway, &prefixed), Some(16000));
    }

    #[test]
    fn explicit_off_resets_one_m_but_preserves_unrelated_limits_and_modalities() {
        let previous = json!({"limit":{"context":ONE_M,"output":128000},"modalities":{"input":["text","image","audio","pdf"],"output":["audio"]},"custom":true});
        let mut off = info();
        off.image_input = Some(false);
        off.one_m_context = Some(false);
        let entry = configured_entry(&off, &previous, None);
        assert!(entry.get("limit").is_none());
        assert_eq!(
            entry["modalities"],
            json!({"input":["text","audio","pdf"],"output":["audio"]})
        );
        assert_eq!(entry["custom"], true);
        let existing = json!({"limit":{"context":512000,"output":64000}});
        assert_eq!(
            configured_entry(&off, &existing, None)["limit"],
            existing["limit"]
        );
        let mut unchanged = info();
        unchanged.image_input = None;
        unchanged.one_m_context = None;
        let unchanged = configured_entry(&unchanged, &previous, None);
        assert_eq!(unchanged["limit"], previous["limit"]);
        assert_eq!(unchanged["modalities"], previous["modalities"]);
    }
}
