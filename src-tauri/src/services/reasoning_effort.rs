//! The composer's five levels are passed through using the selected protocol.
//! An omitted effort leaves the caller's payload unchanged.
use super::llm_client::{LlmConfig, LlmProvider};
use serde_json::{json, Value};

pub fn apply(body: &mut Value, config: &LlmConfig) -> Result<(), String> {
    let Some(effort) = config.reasoning_effort.as_deref() else {
        return Ok(());
    };
    if !["low", "medium", "high", "xhigh", "max"].contains(&effort) {
        return Err(format!("Invalid reasoning effort '{}'", effort));
    }
    match config.provider {
        LlmProvider::OpenAI => body["reasoning_effort"] = json!(effort),
        LlmProvider::Anthropic => body["output_config"]["effort"] = json!(effort),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(model: &str, provider: LlmProvider, effort: Option<&str>) -> LlmConfig {
        LlmConfig {
            model: model.into(),
            provider,
            base_url: "https://api.example.test/v1".into(),
            api_key: "fixture".into(),
            reasoning_effort: effort.map(str::to_owned),
        }
    }

    #[test]
    fn default_preserves_payload_for_known_and_unknown_models() {
        for model in ["deepseek-flash", "claude-opus-5", "unknown"] {
            for provider in [LlmProvider::OpenAI, LlmProvider::Anthropic] {
                let mut body = json!({ "max_tokens": 4096, "messages": [] });
                let original = body.clone();
                apply(&mut body, &config(model, provider, None)).unwrap();
                assert_eq!(body, original);
            }
        }
    }

    #[test]
    fn invalid_values_fail_without_changing_payload() {
        for provider in [LlmProvider::OpenAI, LlmProvider::Anthropic] {
            for effort in ["none", "ultra", "invalid", ""] {
                let mut body = json!({ "messages": [] });
                assert!(apply(&mut body, &config("custom-model", provider, Some(effort))).is_err());
                assert_eq!(body, json!({ "messages": [] }));
            }
        }
    }

    #[test]
    fn every_model_passes_all_five_levels_using_only_the_protocol_field() {
        for provider in [LlmProvider::OpenAI, LlmProvider::Anthropic] {
            for model in [
                "gpt-5.2-pro",
                "claude-opus-4-5",
                "deepseek-flash",
                "custom-model",
            ] {
                for effort in ["low", "medium", "high", "xhigh", "max"] {
                    let mut cfg = config(model, provider, Some(effort));
                    cfg.base_url = "https://openrouter.ai/api/v1".into();
                    let mut body = json!({"max_tokens":4096,"messages":[],
                        "output_config":{"format":{"type":"text"}}});
                    let mut expected = body.clone();
                    match provider {
                        LlmProvider::OpenAI => expected["reasoning_effort"] = json!(effort),
                        LlmProvider::Anthropic => {
                            expected["output_config"]["effort"] = json!(effort)
                        }
                    }
                    apply(&mut body, &cfg).unwrap();
                    assert_eq!(body, expected, "{model} {provider:?} {effort}");
                }
            }
        }
    }
}
