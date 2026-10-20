// Native tool patcher — replaces Node.js patch-*.cjs scripts
// Pure Rust file operations: find install dir → backup → inject marker code → write back

use std::fs;
use std::path::{Path, PathBuf};

const ECHOBIRD_BACKUP_EXT: &str = ".echobird-backup";

// ─── Patch Markers ───

const OPENCLAW_MARKER: &str = "/* [Echobird-Patched] */";

// ─── Injection Code Strings ───

// OpenClaw injection: ESM import style
// Compatible with OpenClaw v2026.3.11+: removed deprecated auth/authHeader fields,
// added required input/reasoning fields to model objects.
const OPENCLAW_INJECT: &str = r#"
/* [Echobird-Patched] */
import { readFileSync as _wc_readFileSync, writeFileSync as _wc_writeFileSync, existsSync as _wc_existsSync, mkdirSync as _wc_mkdirSync } from "node:fs";
import { join as _wc_join } from "node:path";
import { homedir as _wc_homedir } from "node:os";
(function _Echobird_inject() {
  try {
    const wcConfigPath = _wc_join(_wc_homedir(), ".echobird", "openclaw.json");
    if (!_wc_existsSync(wcConfigPath)) return;
    const wcConfig = JSON.parse(_wc_readFileSync(wcConfigPath, "utf-8"));
    if (!wcConfig.modelId || !wcConfig.apiKey) return;

    const ocDir = _wc_join(_wc_homedir(), ".openclaw");
    const ocConfigPath = _wc_join(ocDir, "openclaw.json");
    if (!_wc_existsSync(ocDir)) _wc_mkdirSync(ocDir, { recursive: true });

    let ocConfig = {};
    if (_wc_existsSync(ocConfigPath)) {
      try { ocConfig = JSON.parse(_wc_readFileSync(ocConfigPath, "utf-8")); } catch {}
    }

    if (!ocConfig.models) ocConfig.models = { providers: {} };
    if (!ocConfig.models.providers) ocConfig.models.providers = {};
    if (!ocConfig.agents) ocConfig.agents = {};
    if (!ocConfig.agents.defaults) ocConfig.agents.defaults = {};
    if (!ocConfig.agents.defaults.model) ocConfig.agents.defaults.model = {};

    for (const key of Object.keys(ocConfig.models.providers)) {
      if (key.startsWith("eb_")) {
        delete ocConfig.models.providers[key];
      }
    }

    const protocol = wcConfig.protocol || "openai";
    const isAnthropic = protocol === "anthropic" || wcConfig.modelId?.toLowerCase().includes("claude") || wcConfig.baseUrl?.toLowerCase().includes("anthropic");
    const apiType = isAnthropic ? "anthropic-messages" : "openai-completions";

    let providerTag = "custom";
    try {
      const hostname = new URL(wcConfig.baseUrl || "").hostname;
      if (hostname === "localhost" || hostname.startsWith("127.") || hostname.startsWith("192.168.")) {
        providerTag = "local";
      } else {
        const parts = hostname.split(".");
        providerTag = parts.length >= 2 ? parts[parts.length - 2] : hostname;
      }
    } catch {}

    const ebProviderName = "eb_" + providerTag;
    let baseUrl = wcConfig.baseUrl || "https://api.openai.com/v1";
    if (baseUrl.endsWith("/")) baseUrl = baseUrl.slice(0, -1);

    ocConfig.models.providers[ebProviderName] = {
      baseUrl: baseUrl,
      apiKey: wcConfig.apiKey,
      api: apiType,
      models: [{
        id: wcConfig.modelId,
        name: wcConfig.modelName || wcConfig.modelId,
        contextWindow: 128000,
        maxTokens: 8192,
        input: ["text"],
        reasoning: false,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }
      }]
    };
    ocConfig.agents.defaults.model.primary = ebProviderName + "/" + wcConfig.modelId;
    console.log("[EchoBird] Injected " + apiType + " model: " + ebProviderName + "/" + wcConfig.modelId);

    _wc_writeFileSync(ocConfigPath, JSON.stringify(ocConfig, null, 2), "utf-8");
  } catch (err) {
    console.warn("[EchoBird] Config injection failed:", err.message);
  }
})();

"#;

// ─── Installation Directories ───

/// Find npm global module install directory
fn find_npm_global_module(package_name: &str) -> Option<PathBuf> {
    // Try `npm root -g` first
    if let Ok(output) = crate::utils::process::command("npm")
        .args(["root", "-g"])
        .output()
    {
        if output.status.success() {
            let npm_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let candidate = PathBuf::from(&npm_root).join(package_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // Try known paths
    let home = dirs::home_dir()?;
    let candidates = vec![
        // Windows
        PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
            .join("npm")
            .join("node_modules")
            .join(package_name),
        home.join(".npm-global")
            .join("lib")
            .join("node_modules")
            .join(package_name),
        // macOS/Linux
        PathBuf::from("/usr/local/lib/node_modules").join(package_name),
        PathBuf::from("/usr/lib/node_modules").join(package_name),
    ];

    candidates.into_iter().find(|p| p.exists())
}

// ─── Generic Patcher ───

struct PatchConfig {
    marker: &'static str,
    inject_code: &'static str,
    /// Search patterns to find injection point (tried in order)
    search_patterns: Vec<&'static str>,
    /// true = inject AFTER pattern, false = inject BEFORE
    inject_after: bool,
}

/// Generic patch function: find entry file, backup, inject code
fn patch_entry_file(entry_path: &Path, config: &PatchConfig) -> bool {
    if !entry_path.exists() {
        log::warn!("[Patcher] Entry file not found: {:?}", entry_path);
        return false;
    }

    let backup_path = PathBuf::from(format!("{}{}", entry_path.display(), ECHOBIRD_BACKUP_EXT));

    let mut content = match fs::read_to_string(entry_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[Patcher] Failed to read {:?}: {}", entry_path, e);
            return false;
        }
    };

    // Already patched? Restore from backup first
    if content.contains(config.marker) {
        if backup_path.exists() {
            content = match fs::read_to_string(&backup_path) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("[Patcher] Failed to read backup: {}", e);
                    return false;
                }
            };
        } else {
            log::warn!("[Patcher] Already patched but no backup found");
            return false;
        }
    } else {
        // First patch — create backup
        if let Err(e) = fs::copy(entry_path, &backup_path) {
            log::warn!("[Patcher] Failed to create backup: {}", e);
            return false;
        }
    }

    // Find injection point
    let mut inject_pos = None;
    for pattern in &config.search_patterns {
        if let Some(idx) = content.find(pattern) {
            inject_pos = Some(if config.inject_after {
                idx + pattern.len()
            } else {
                idx
            });
            break;
        }
    }

    let pos = match inject_pos {
        Some(p) => p,
        None => {
            log::warn!("[Patcher] No injection point found in {:?}", entry_path);
            return false;
        }
    };

    let patched = format!(
        "{}\n{}{}",
        &content[..pos],
        config.inject_code,
        &content[pos..]
    );

    if let Err(e) = fs::write(entry_path, &patched) {
        log::warn!("[Patcher] Failed to write patched file: {}", e);
        return false;
    }

    log::info!("[Patcher] Successfully patched: {:?}", entry_path);
    true
}

// ─── Tool-Specific Patchers ───

/// Patch OpenClaw CLI tool
pub fn patch_openclaw() {
    let install_dir = match find_npm_global_module("openclaw") {
        Some(d) => d,
        None => {
            log::info!("[Patcher] OpenClaw not found, skipping");
            return;
        }
    };

    let entry = install_dir.join("openclaw.mjs");
    patch_entry_file(
        &entry,
        &PatchConfig {
            marker: OPENCLAW_MARKER,
            inject_code: OPENCLAW_INJECT,
            search_patterns: vec![
                "await installProcessWarningFilter();",
                "if (await tryImport(",
            ],
            inject_after: true,
        },
    );
}

// ─── OpenCode v1 Patcher (legacy npm compatibility) ───

const OPENCODE_MARKER: &str = "/* [Echobird-OpenCode-Patched] */";

// OpenCode injection: CJS style, injected BEFORE `run()` is called.
// Reads ~/.echobird/opencode.json for API key and base URL,
// sets process.env so the spawned binary inherits them.
// Writes ~/.config/opencode/opencode.json with provider + model config.
const OPENCODE_INJECT: &str = r#"
/* [Echobird-OpenCode-Patched] */
;(function() { try {
  var _eb_p = path.join(os.homedir(), ".echobird", "opencode.json");
  if (fs.existsSync(_eb_p)) {
    var _eb_c = JSON.parse(fs.readFileSync(_eb_p, "utf-8"));
    if (_eb_c.apiKey) process.env.OPENAI_API_KEY = _eb_c.apiKey;
    if (_eb_c.baseUrl) process.env.OPENAI_BASE_URL = _eb_c.baseUrl;
    // Write provider + model config for OpenCode to pick up
    var _eb_cfgDir = path.join(os.homedir(), ".config", "opencode");
    if (!fs.existsSync(_eb_cfgDir)) fs.mkdirSync(_eb_cfgDir, {recursive:true});
    var _eb_cfgPath = path.join(_eb_cfgDir, "opencode.json");
    var _eb_provId = "echobird";
    var _eb_cfg = {"$schema":"https://opencode.ai/config.json","provider":{}};
    try { if (fs.existsSync(_eb_cfgPath)) _eb_cfg = JSON.parse(fs.readFileSync(_eb_cfgPath,"utf-8")); } catch {}
    if (!_eb_cfg.provider) _eb_cfg.provider = {};
    _eb_cfg.provider[_eb_provId] = {
      npm: "@ai-sdk/openai-compatible",
      name: _eb_c.providerName || "Echobird",
      options: { baseURL: _eb_c.baseUrl || "", apiKey: _eb_c.apiKey || "" },
      models: {}
    };
    if (_eb_c.modelId) {
      _eb_cfg.provider[_eb_provId].models[_eb_c.modelId] = {name: _eb_c.modelName || _eb_c.modelId};
      // Set model at top level — this is how OpenCode selects the default model
      _eb_cfg.model = _eb_provId + "/" + _eb_c.modelId;
      _eb_cfg.small_model = _eb_provId + "/" + _eb_c.modelId;
    }
    fs.writeFileSync(_eb_cfgPath, JSON.stringify(_eb_cfg, null, 2), "utf-8");
    // Also set OPENCODE_CONFIG_CONTENT for runtime override (highest priority)
    if (_eb_c.modelId) {
      process.env.OPENCODE_CONFIG_CONTENT = JSON.stringify({model: _eb_provId + "/" + _eb_c.modelId, small_model: _eb_provId + "/" + _eb_c.modelId});
    }
    console.log("[EchoBird] OpenCode configured: model=" + _eb_provId + "/" + (_eb_c.modelId || "default"));
  }
} catch(_e) { console.warn("[EchoBird] OpenCode inject error:", _e.message); } })();
"#;

/// Patch the legacy `opencode-ai` launcher. OpenCode v2 ships a native binary
/// and reads the config written by `tool_config_manager` directly.
pub fn patch_opencode() {
    let install_dir = match find_npm_global_module("opencode-ai") {
        Some(d) => d,
        None => {
            log::info!("[Patcher] OpenCode not found, skipping");
            return;
        }
    };

    let entry = install_dir.join("bin").join("opencode");

    patch_entry_file(
        &entry,
        &PatchConfig {
            marker: OPENCODE_MARKER,
            inject_code: OPENCODE_INJECT,
            search_patterns: vec![
                "const envPath = process.env.OPENCODE_BIN_PATH",
                "function run(target) {",
            ],
            inject_after: false, // inject BEFORE these patterns
        },
    );
}

/// Dispatch patch by tool ID
pub fn patch_tool(tool_id: &str) {
    match tool_id {
        "openclaw" => patch_openclaw(),
        "opencode" => patch_opencode(),
        _ => log::debug!("[Patcher] No patch needed for tool: {}", tool_id),
    }
}
