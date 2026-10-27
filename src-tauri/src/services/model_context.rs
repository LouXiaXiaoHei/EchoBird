//! One source of truth for a model's context window (in tokens).
//!
//! Two readers depend on this:
//!  * the Auto Router's `/v1/models` response, so an external client can size
//!    its budget instead of falling back to its own minimum tier;
//!  * the per-tool config writers (Codex `model_context_window`, OpenCode
//!    `limit.context`, ...) that must not over-claim a window.
//!
//! Resolution order: the vendor catalog assets already shipped for Codex
//! (documented per-model values), then a small family registry for the
//! partially documented vendors, then `DEFAULT_CONTEXT_WINDOW`.
//!
//! `DEFAULT_CONTEXT_WINDOW` mirrors the historic Codex default: an
//! unregistered model keeps the 1M assumption EchoBird has always written
//! rather than silently dropping to a client-side minimum.

use crate::services::codex_catalog;

/// Assumed window for a model neither the vendor catalogs nor the family
/// registry describes: the historic 1M assumption EchoBird has always written.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 1_000_000;

/// Context window in tokens for `model_id` served from `base_url`.
///
/// `base_url` may be empty when only the model id is known; the vendor
/// catalog lookup is then skipped and only the family registry applies.
pub fn context_window_for(base_url: &str, model_id: &str) -> u64 {
    if let Some(window) = catalog_context_window(base_url, model_id) {
        return window;
    }
    family_context_window(model_id).unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// Documented `context_window` from the bundled Codex vendor catalogs.
fn catalog_context_window(base_url: &str, model_id: &str) -> Option<u64> {
    if base_url.trim().is_empty() || model_id.trim().is_empty() {
        return None;
    }
    let value = codex_catalog::template_for_model(base_url, model_id)?;
    entry_context_window(&value)
}

fn entry_context_window(value: &serde_json::Value) -> Option<u64> {
    // `template_for_model` returns either one model entry or a whole
    // `{ "models": [...] }` template, depending on the lookup branch.
    if let Some(window) = value.get("context_window").and_then(|v| v.as_u64()) {
        return Some(window);
    }
    value
        .get("models")?
        .as_array()?
        .first()?
        .get("context_window")?
        .as_u64()
}

/// Windows for model families whose exact per-release value is not in the
/// vendor catalog assets. Prefixes are matched longest-first so
/// `deepseek-v4-flash` cannot be shadowed by a broader `deepseek` entry.
fn family_context_window(model_id: &str) -> Option<u64> {
    let id = model_id.trim().to_ascii_lowercase();
    if id.is_empty() {
        return None;
    }
    for (prefix, window) in FAMILY_WINDOWS {
        if id.starts_with(prefix) {
            return Some(*window);
        }
    }
    None
}

/// Longest prefix first.
const FAMILY_WINDOWS: &[(&str, u64)] = &[
    // DeepSeek's own endpoints serve 1M. A reseller's smaller documented
    // window (163,840 on the Qwen plan catalog) is resolved by the vendor
    // catalog lookup above, which knows the base URL.
    ("deepseek-", 1_048_576),
    // Kimi K2 code models are documented at 256K.
    ("kimi-k2", 262_144),
    ("kimi-", 262_144),
    // GLM: 5.1/5 are 200K, 5.2+ are 1M (see catalog assets for exact slugs).
    ("glm-5.1", 202_752),
    ("glm-5-turbo", 204_800),
    ("glm-", 1_000_000),
    // Qwen: 983,616 for max-tier, 1M for the newer plan models.
    ("qwen3.8", 983_616),
    ("qwen3.", 1_000_000),
    ("qwen", 1_000_000),
    // MiniMax M2.x is 200K, M3.x is 512K.
    ("minimax-m2", 204_800),
    ("minimax-m3.1-flash", 524_288),
    ("minimax-m3", 1_000_000),
    // Anthropic Claude: 200K, or 1M with the long-context beta.
    ("claude-", 200_000),
    // MiMo (Xiaomi) ships 1M across the current line.
    ("mimo-", 1_048_576),
    // StepFun plan models.
    ("step-3.", 256_000),
    ("step-5", 1_000_000),
    // OpenAI: 400K for the Codex line, 272K for gpt-5.x, 128K for gpt-4o.
    ("gpt-5.1-codex", 400_000),
    ("gpt-5", 272_000),
    ("gpt-4o", 128_000),
    ("gpt-4.1", 1_047_576),
    // Google Gemini: 1M.
    ("gemini-", 1_048_576),
    // Grok.
    ("grok-", 256_000),
    // OpenCode Zen free routing id.
    ("auto", 1_048_576),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_catalog_wins_over_family_registry() {
        // glm-5.2 is documented at 1M by the Qwen catalog but only 200K in the
        // GLM family fallback: the documented value must win.
        assert_eq!(
            context_window_for("https://maas.qianwenaiapi.com/v1", "glm-5.2"),
            1_000_000
        );
        assert_eq!(context_window_for("", "glm-5.2"), 1_000_000);
        assert_eq!(context_window_for("", "glm-5.1"), 202_752);
    }

    #[test]
    fn documented_small_windows_are_not_raised_to_the_default() {
        // A reseller's catalog value differs from the vendor's own window:
        // both must resolve from their own documented source.
        assert_eq!(
            context_window_for("https://maas.qianwenaiapi.com/v1", "qwen3.8-max"),
            983_616
        );
        assert_eq!(
            context_window_for("https://api.deepseek.com", "deepseek-v4-pro"),
            1_048_576
        );
        assert_eq!(context_window_for("", "deepseek-flash"), 1_048_576);
        assert_eq!(context_window_for("", "MiniMax-M2.7"), 204_800);
        assert_eq!(context_window_for("", "MiniMax-M3"), 1_000_000);
        assert_eq!(context_window_for("", "kimi-k2.7-code"), 262_144);
        assert_eq!(context_window_for("", "claude-sonnet-4-5"), 200_000);
    }

    #[test]
    fn plan_catalog_resolves_the_selected_slug_not_a_neighbour() {
        // A plan catalog lists many vendors' models; the lookup has to pick the
        // entry whose slug matches, not the first entry of the template.
        let base = "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1";
        assert_eq!(
            context_window_for(base, "deepseek-v4.1-flash-1017"),
            1_000_000
        );
        assert_eq!(context_window_for(base, "deepseek-v4-pro"), 163_840);
        assert_eq!(context_window_for(base, "glm-5.1"), 202_752);
        assert_eq!(context_window_for(base, "kimi-k2.7-code"), 262_144);
    }

    #[test]
    fn unknown_models_keep_the_historic_one_million_default() {
        assert_eq!(context_window_for("", "totally-unknown-model"), 1_000_000);
        assert_eq!(context_window_for("", ""), DEFAULT_CONTEXT_WINDOW);
        assert_eq!(
            context_window_for("https://unknown.example/v1", "some-model"),
            1_000_000
        );
    }

    #[test]
    fn family_prefixes_match_longest_first() {
        // `minimax-m3.1-flash` (512K) must not be shadowed by `minimax-m3`.
        assert_eq!(family_context_window("minimax-m3.1-flash"), Some(524_288));
        assert_eq!(family_context_window("minimax-m3"), Some(1_000_000));
        assert_eq!(family_context_window("glm-5-turbo"), Some(204_800));
        assert_eq!(family_context_window("unknown-model"), None);
    }
}
