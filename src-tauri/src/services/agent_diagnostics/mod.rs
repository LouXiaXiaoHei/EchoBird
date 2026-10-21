mod artifact;
mod state;
mod storage;

use super::agent_output_store::{OutputRecord, OutputStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

const FILE_LIMIT: u64 = 16 * 1024 * 1024;

pub(crate) fn definitions() -> Vec<super::llm_client::ToolDef> {
    let schemas = [
        ("artifact_inspect", "Inspect a LOCAL ASAR without executing it. Omit member to page its file index; select member to read UTF-8 text, optionally search a literal. Offsets are bytes for member text and entries for the index. Results report next_offset and coverage.", json!({"path":{"type":"string"},"member":{"type":"string"},"search":{"type":"string"},"offset":{"type":"integer","minimum":0}}), vec!["path"]),
        ("storage_inspect", "Inspect a LOCAL SQLite or LevelDB using a disposable copy; never open the original database. SQLite: omit table for schema, or select table and columns for up to 20 rows. LevelDB: omit key for key names only, or supply an exact UTF-8 key (JSON escapes allow Chromium control bytes). Offset pages rows/keys. No database writes or SQL execution supplied by the model.", json!({"path":{"type":"string"},"format":{"type":"string","enum":["sqlite","leveldb"]},"table":{"type":"string"},"columns":{"type":"array","items":{"type":"string"},"maxItems":16},"key":{"type":"string"},"offset":{"type":"integer","minimum":0}}), vec!["path","format"]),
        ("diagnostic_snapshot", "Capture hashes of 1-16 explicit LOCAL files. JSON/JSONC snapshots also retain leaf hashes for changed-field paths, never raw values. Optional baseline_id compares against a snapshot from this conversation. For databases, snapshot their explicit main/WAL/log files, and use storage_inspect for selected decoded state. Nothing executes or repairs files.", json!({"paths":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":16},"baseline_id":{"type":"string"}}), vec!["paths"]),
        ("file_repair", "Repair one existing LOCAL UTF-8 text/config file with exactly one replacement. Supply path, expected_sha256, old_string and new_string. JSON/JSONC/TOML/YAML/plist syntax is checked before writing; a conversation-scoped backup and atomic replacement are used. To restore, supply only backup_id; restore refuses if the file has changed since repair. Saved backups expire with output retention. Verify the app's actual behavior afterward; this does not edit databases, ASARs, executables or model configuration managed by EchoBird.", json!({"path":{"type":"string"},"expected_sha256":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"backup_id":{"type":"string"}}), vec![]),
    ];
    schemas.into_iter().map(|(name, description, properties, required)| super::llm_client::ToolDef {
        name: name.into(), description: description.into(),
        parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }).collect()
}

pub(crate) fn handles(name: &str) -> bool {
    matches!(
        name,
        "artifact_inspect" | "storage_inspect" | "diagnostic_snapshot" | "file_repair"
    )
}

pub(crate) fn execute(
    name: &str,
    args: Value,
    scope: &str,
    store: &OutputStore,
    cancel: &CancellationToken,
) -> Result<String, String> {
    check_cancel(cancel)?;
    let result = match name {
        "artifact_inspect" => artifact::inspect(parse(args)?),
        "storage_inspect" => storage::inspect(parse(args)?),
        "diagnostic_snapshot" => state::snapshot(parse(args)?, scope, store, cancel),
        "file_repair" => state::repair(parse(args)?, scope, store, cancel),
        _ => Err("Unknown diagnostic tool".into()),
    }?;
    check_cancel(cancel)?;
    let text = serde_json::to_string(&result).map_err(|e| e.to_string())?;
    if text.len() <= 8_000 {
        return Ok(text);
    }
    let id = store.store(
        scope,
        &OutputRecord {
            command: name.into(),
            target: "local".into(),
            exit_code: None,
            success: true,
            stdout: text,
            stderr: String::new(),
        },
    )?;
    store.read(scope, &id, 0, 6_000)
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("Diagnostic cancelled; verify file state before retrying a repair".into())
    } else {
        Ok(())
    }
}
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("Invalid diagnostic arguments: {e}"))
}
fn absolute(path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err("An absolute local path is required".into());
    }
    fs::canonicalize(p).map_err(|e| format!("Cannot resolve path: {e}"))
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    if !fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
        return Err("Expected a regular file".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Expected a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("File exceeds the {limit}-byte diagnostic limit"));
    }
    Ok(bytes)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn saved<T: Serialize>(
    kind: &str,
    value: &T,
    scope: &str,
    store: &OutputStore,
) -> Result<String, String> {
    store.store(
        scope,
        &OutputRecord {
            command: String::new(),
            target: kind.into(),
            exit_code: None,
            success: true,
            stdout: serde_json::to_string(value).map_err(|e| e.to_string())?,
            stderr: String::new(),
        },
    )
}
fn load<T: for<'a> Deserialize<'a>>(
    kind: &str,
    id: &str,
    scope: &str,
    store: &OutputStore,
) -> Result<T, String> {
    let record = store.load(scope, id)?;
    if record.target != kind {
        return Err("Wrong diagnostic record type".into());
    }
    serde_json::from_str(&record.stdout).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
