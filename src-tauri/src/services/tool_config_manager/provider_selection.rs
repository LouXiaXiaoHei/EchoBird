//! Reconcile OpenCode-style provider filters on explicit model application.

use serde_json::{json, Value};

pub(super) fn enable_provider(config: &mut Value, provider_id: &str) {
    if let Some(disabled) = config
        .get_mut("disabled_providers")
        .and_then(Value::as_array_mut)
    {
        disabled.retain(|id| id.as_str() != Some(provider_id));
    }
    if let Some(enabled) = config
        .get_mut("enabled_providers")
        .and_then(Value::as_array_mut)
    {
        if !enabled.iter().any(|id| id.as_str() == Some(provider_id)) {
            enabled.push(json!(provider_id));
        }
    }
}

// V2 evaluates the last matching policy. Keep other providers and actions intact.
pub(super) fn allow_provider_policy(config: &mut Value, provider_id: &str) {
    // Do not add v2-only fields to legacy clients' configurations.
    let path = if config.get("policies").is_some() {
        "/policies"
    } else {
        "/experimental/policies"
    };
    if let Some(policies) = config.pointer_mut(path).and_then(Value::as_array_mut) {
        policies.retain(|policy| {
            !(policy.get("action").and_then(Value::as_str) == Some("provider.use")
                && policy.get("resource").and_then(Value::as_str) == Some(provider_id))
        });
        policies.push(json!({
            "action": "provider.use", "resource": provider_id, "effect": "allow"
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_selection_resolves_filters_without_enabling_other_providers() {
        let mut config = json!({
            "disabled_providers": ["other", "echobird", "echobird"],
            "enabled_providers": ["existing"],
            "provider": {"existing": {"name": "Keep me"}},
            "theme": "unchanged"
        });
        enable_provider(&mut config, "echobird");
        assert_eq!(config["disabled_providers"], json!(["other"]));
        assert_eq!(config["enabled_providers"], json!(["existing", "echobird"]));
        assert_eq!(config["provider"]["existing"]["name"], "Keep me");
        assert_eq!(config["theme"], "unchanged");
        let applied = config.clone();
        enable_provider(&mut config, "echobird");
        assert_eq!(config, applied);
    }

    #[test]
    fn empty_allowlist_becomes_selected_provider_only() {
        let mut config = json!({"enabled_providers": []});
        enable_provider(&mut config, "echobird");
        assert_eq!(config["enabled_providers"], json!(["echobird"]));
    }

    #[test]
    fn unrestricted_configuration_stays_unrestricted() {
        let mut config = json!({});
        enable_provider(&mut config, "echobird");
        allow_provider_policy(&mut config, "echobird");
        assert_eq!(config, json!({}));
    }

    #[test]
    fn policy_override_is_specific_and_idempotent() {
        let unrelated = json!([
            {"action": "provider.use", "resource": "*", "effect": "deny"},
            {"action": "tool.use", "resource": "*", "effect": "deny"}
        ]);
        let mut config = json!({"policies": unrelated});
        config["policies"].as_array_mut().unwrap().push(json!({
            "action": "provider.use", "resource": "echobird", "effect": "deny"
        }));
        allow_provider_policy(&mut config, "echobird");
        assert_eq!(
            &config["policies"].as_array().unwrap()[..2],
            unrelated.as_array().unwrap()
        );
        assert_eq!(
            config["policies"][2],
            json!({
                "action": "provider.use", "resource": "echobird", "effect": "allow"
            })
        );
        let applied = config.clone();
        allow_provider_policy(&mut config, "echobird");
        assert_eq!(config, applied);
    }

    #[test]
    fn experimental_policies_are_reconciled_without_adding_native_fields() {
        let mut config = json!({"experimental": {"policies": [
            {"action": "provider.use", "resource": "*", "effect": "deny"}
        ]}});
        allow_provider_policy(&mut config, "echobird");
        assert!(config.get("policies").is_none());
        assert_eq!(config["experimental"]["policies"][0]["effect"], "deny");
        assert_eq!(
            config["experimental"]["policies"][1],
            json!({
                "action": "provider.use", "resource": "echobird", "effect": "allow"
            })
        );
    }
}
