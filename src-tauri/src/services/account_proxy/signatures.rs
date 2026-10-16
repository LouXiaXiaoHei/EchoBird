use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Deserialize, Serialize)]
struct Signature {
    id: String,
    signature: String,
}

fn reference(value: &Signature) -> String {
    let bytes = serde_json::to_vec(value).expect("string fields serialize");
    hex::encode(Sha256::digest(bytes))[..32].to_string()
}

fn directory() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        static DIRECTORY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        Ok(DIRECTORY
            .get_or_init(|| {
                std::env::temp_dir().join(format!("echobird-signatures-{}", uuid::Uuid::new_v4()))
            })
            .clone())
    }
    #[cfg(not(test))]
    dirs::home_dir()
        .map(|home| home.join(".echobird/cache/code-assist-signatures"))
        .ok_or_else(|| "Could not locate the thought signature cache".into())
}

pub(super) fn encode(id: &str, signature: &str) -> Result<String, String> {
    if signature.is_empty() {
        return Ok(id.to_string());
    }
    encode_at(&directory()?, id, signature)
}

fn encode_at(root: &Path, id: &str, signature: &str) -> Result<String, String> {
    let value = Signature {
        id: id.into(),
        signature: signature.into(),
    };
    let key = reference(&value);
    let path = root.join(format!("{key}.json"));
    crate::services::native_account_files::write(
        &path,
        &serde_json::to_vec(&value).map_err(|_| "Could not encode a thought signature")?,
    )
    .map_err(|_| "Could not save the thought signature for this tool call")?;
    prune(root);
    Ok(format!("call_eb_{key}"))
}

pub(super) fn decode(id: &str) -> Option<(String, String)> {
    decode_at(&directory().ok()?, id)
}

fn decode_at(root: &Path, id: &str) -> Option<(String, String)> {
    let key = id.strip_prefix("call_eb_")?;
    if key.len() != 32 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let path = root.join(format!("{key}.json"));
    let value: Signature = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
    if reference(&value) != key {
        return None;
    }
    if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
    Some((value.id, value.signature))
}

fn prune(root: &Path) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let Ok(mut last) = LAST.lock() else { return };
    if last.is_some_and(|last| last.elapsed() < Duration::from_secs(3600)) {
        return;
    }
    *last = Some(Instant::now());
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(key) = name.strip_suffix(".json") else {
            continue;
        };
        if key.len() != 32 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let expired = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(90 * 86_400));
        if expired {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_reference_survives_a_fresh_read_and_rejects_corruption() {
        let root =
            std::env::temp_dir().join(format!("echobird-signature-test-{}", uuid::Uuid::new_v4()));
        let signature = "thought".repeat(1024);
        let id = encode_at(&root, "original-call", &signature).unwrap();
        assert!(id.len() <= 64);
        assert_eq!(
            decode_at(&root, &id),
            Some(("original-call".into(), signature))
        );
        let file = root.join(format!("{}.json", id.strip_prefix("call_eb_").unwrap()));
        fs::write(&file, br#"{"id":"different","signature":"tampered"}"#).unwrap();
        assert!(decode_at(&root, &id).is_none());
        fs::remove_file(&file).unwrap();
        assert!(decode_at(&root, &id).is_none());
        fs::write(&file, b"occupied").unwrap();
        assert!(encode_at(&file, "call", "signature").is_err());
        fs::remove_file(file).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
