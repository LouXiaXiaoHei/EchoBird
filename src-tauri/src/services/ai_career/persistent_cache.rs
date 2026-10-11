use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct FileStamp {
    pub mtime: SystemTime,
    pub size: u64,
    #[serde(default)]
    created: Option<SystemTime>,
    #[cfg(unix)]
    #[serde(default)]
    identity: Option<(u64, u64)>,
}

impl FileStamp {
    pub(super) fn read(metadata: &std::fs::Metadata) -> std::io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            mtime: metadata.modified()?,
            size: metadata.len(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: Some((metadata.dev(), metadata.ino())),
        })
    }
}

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Snapshot<T> {
    version: u32,
    entries: T,
}

pub(super) fn read<T: DeserializeOwned>(path: &Path) -> HashMap<PathBuf, T> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Snapshot<HashMap<PathBuf, T>>>(&bytes).ok())
        .filter(|snapshot| snapshot.version == VERSION)
        .map(|snapshot| snapshot.entries)
        .unwrap_or_default()
}

// The caller holds its cache mutex through serialization and replacement.
pub(super) fn write<T: Serialize>(path: &Path, entries: &HashMap<PathBuf, T>) -> io::Result<()> {
    let bytes = serde_json::to_vec(&Snapshot {
        version: VERSION,
        entries,
    })?;
    replace(path, &bytes)
}

pub(super) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cache has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".ai-career-{}.tmp", uuid::Uuid::new_v4()));
    let result = std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_cache_replaces_atomically_and_recovers_from_invalid_data() {
        let dir = std::env::temp_dir().join(format!("echobird-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");
        assert!(read::<u64>(&path).is_empty());
        let entries = HashMap::from([(dir.join("fixture.jsonl"), 10u64)]);
        write(&path, &entries).unwrap();
        assert_eq!(read::<u64>(&path), entries);
        write(&path, &HashMap::<PathBuf, u64>::new()).unwrap();
        assert!(read::<u64>(&path).is_empty());
        std::fs::write(&path, b"{partial").unwrap();
        assert!(read::<u64>(&path).is_empty());
        std::fs::write(&path, r#"{"version":0,"entries":{}}"#).unwrap();
        assert!(read::<u64>(&path).is_empty());
        write(&path, &entries).unwrap();
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"old cache").unwrap();
        assert!(write(&blocked.join("cache.json"), &entries).is_err());
        assert_eq!(std::fs::read(&blocked).unwrap(), b"old cache");
        assert_eq!(read::<u64>(&path), entries);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
