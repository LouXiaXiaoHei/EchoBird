use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

type Secret = Result<Option<Vec<u8>>, String>;

struct Read {
    result: Mutex<Option<Secret>>,
    ready: Condvar,
    started: Instant,
    stamp: Option<(u64, u128)>,
}

#[derive(Default)]
pub(super) struct ReadCache(Mutex<HashMap<PathBuf, Arc<Read>>>);

impl ReadCache {
    pub(super) fn read(
        &self,
        key: &Path,
        stamp: Option<(u64, u128)>,
        wait: Duration,
        fetch: impl FnOnce() -> Secret + Send + 'static,
    ) -> Secret {
        let (read, start) = {
            let mut reads = self.0.lock().map_err(|_| "accountError.keychain")?;
            let reusable = reads
                .get(key)
                .filter(|read| {
                    let Ok(result) = read.result.lock() else {
                        return false;
                    };
                    match result.as_ref() {
                        None => true,
                        Some(Ok(Some(_))) => {
                            read.stamp == stamp
                                && (stamp.is_some()
                                    || read.started.elapsed() < Duration::from_secs(60))
                        }
                        Some(Ok(None)) => read.started.elapsed() < Duration::from_secs(10),
                        Some(Err(_)) => read.started.elapsed() < Duration::from_secs(600),
                    }
                })
                .cloned();
            match reusable {
                Some(read) => (read, false),
                None => {
                    let read = Arc::new(Read {
                        result: Mutex::new(None),
                        ready: Condvar::new(),
                        started: Instant::now(),
                        stamp,
                    });
                    reads.insert(key.to_path_buf(), read.clone());
                    (read, true)
                }
            }
        };
        if start {
            let pending = read.clone();
            std::thread::spawn(move || {
                let result = fetch();
                if let Ok(mut value) = pending.result.lock() {
                    *value = Some(result);
                    pending.ready.notify_all();
                }
            });
        }
        let value = read.result.lock().map_err(|_| "accountError.keychain")?;
        let remaining = wait.saturating_sub(read.started.elapsed());
        let (value, _) = read
            .ready
            .wait_timeout_while(value, remaining, |value| value.is_none())
            .map_err(|_| "accountError.keychain")?;
        value.clone().unwrap_or_else(|| {
            Err("accountError.keychain|Waiting for macOS keychain approval".into())
        })
    }

    pub(super) fn retry_failure(&self, key: &Path) {
        if let Ok(mut reads) = self.0.lock() {
            let failed = reads.get(key).is_some_and(|read| {
                read.result
                    .lock()
                    .is_ok_and(|value| matches!(value.as_ref(), Some(Err(_))))
            });
            if failed {
                reads.remove(key);
            }
        }
    }

    #[cfg(all(target_os = "macos", not(test)))]
    pub(super) fn forget(&self, key: &Path) {
        if let Ok(mut reads) = self.0.lock() {
            reads.remove(key);
        }
    }
}

fn secret_output(code: Option<i32>, output: Vec<u8>) -> Secret {
    match code {
        Some(0) => Ok((!output.is_empty()).then_some(output)),
        Some(44) => Ok(None),
        _ => Err("accountError.keychain|macOS keychain access was refused or failed".into()),
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn cache() -> &'static ReadCache {
    static CACHE: std::sync::OnceLock<ReadCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(ReadCache::default)
}

#[cfg(all(target_os = "macos", not(test)))]
fn key(base: &Path) -> PathBuf {
    std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf())
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn retry(base: &Path) {
    cache().retry_failure(&key(base));
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn forget(base: &Path) {
    cache().forget(&key(base));
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn read(base: &Path, account: String) -> Secret {
    use std::hash::{Hash, Hasher};
    let stamp = dirs::home_dir().and_then(|home| {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        let mut found = false;
        let mut size = 0;
        for name in ["login.keychain-db", "login.keychain-db-wal"] {
            let file = std::fs::metadata(home.join("Library/Keychains").join(name))
                .ok()
                .and_then(|metadata| Some((metadata.len(), metadata.modified().ok()?)));
            if let Some((len, _)) = file {
                size += len;
                found = true;
            }
            file.hash(&mut hash);
        }
        found.then_some((size, u128::from(hash.finish())))
    });
    cache().read(&key(base), stamp, Duration::from_secs(5), move || {
        tauri::async_runtime::block_on(async move {
            let command = crate::utils::process::command("/usr/bin/security");
            let mut command = tokio::process::Command::from(command);
            command
                .args([
                    "find-generic-password",
                    "-s",
                    "Codex Auth",
                    "-a",
                    &account,
                    "-w",
                ])
                .kill_on_drop(true);
            match tokio::time::timeout(Duration::from_secs(120), command.output()).await {
                Ok(Ok(output)) => secret_output(output.status.code(), output.stdout),
                Ok(Err(_)) => Err("accountError.keychain|Could not read the macOS keychain".into()),
                Err(_) => Err("accountError.keychain|macOS keychain approval timed out".into()),
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn missing_and_denied_keychain_items_are_distinct() {
        assert_eq!(secret_output(Some(44), vec![]).unwrap(), None);
        assert!(secret_output(Some(51), vec![]).is_err());
        assert_eq!(
            secret_output(Some(0), b"secret".to_vec()).unwrap(),
            Some(b"secret".to_vec())
        );
    }

    #[test]
    fn pending_keychain_read_is_shared_and_late_approval_is_kept() {
        let cache = ReadCache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let (release, held) = std::sync::mpsc::channel();
        let counter = calls.clone();
        let key = Path::new("fixture");
        assert!(cache
            .read(key, None, Duration::from_millis(10), move || {
                counter.fetch_add(1, Ordering::SeqCst);
                held.recv().unwrap();
                Ok(Some(b"approved".to_vec()))
            })
            .is_err());
        assert!(cache
            .read(key, None, Duration::from_millis(10), || panic!(
                "must share the pending read"
            ))
            .is_err());
        release.send(()).unwrap();
        for _ in 0..100 {
            if let Ok(value) = cache.read(key, None, Duration::from_millis(10), || {
                panic!("must keep approval")
            }) {
                assert_eq!(value, Some(b"approved".to_vec()));
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("late approval was not retained");
    }

    #[test]
    fn refusal_is_not_repeated_until_explicit_retry() {
        let cache = ReadCache::default();
        let key = Path::new("fixture");
        assert!(cache
            .read(key, None, Duration::from_secs(1), || secret_output(
                Some(51),
                vec![]
            ))
            .is_err());
        assert!(cache
            .read(key, None, Duration::from_secs(1), || panic!(
                "refusal must be cached"
            ))
            .is_err());
        cache.retry_failure(key);
        assert_eq!(
            cache
                .read(key, None, Duration::from_secs(1), || Ok(Some(
                    b"retry".to_vec()
                )))
                .unwrap(),
            Some(b"retry".to_vec())
        );
        cache.retry_failure(key);
        assert_eq!(
            cache
                .read(key, None, Duration::from_secs(1), || panic!(
                    "success must survive retry"
                ))
                .unwrap(),
            Some(b"retry".to_vec())
        );
    }

    #[test]
    fn a_changed_keychain_stamp_reads_the_new_item() {
        let cache = ReadCache::default();
        let key = Path::new("fixture");
        assert_eq!(
            cache
                .read(key, Some((1, 1)), Duration::from_secs(1), || Ok(Some(
                    b"first".to_vec()
                )))
                .unwrap(),
            Some(b"first".to_vec())
        );
        assert_eq!(
            cache
                .read(key, Some((1, 1)), Duration::from_secs(1), || panic!(
                    "unchanged item must be cached"
                ))
                .unwrap(),
            Some(b"first".to_vec())
        );
        assert_eq!(
            cache
                .read(key, Some((1, 2)), Duration::from_secs(1), || Ok(Some(
                    b"second".to_vec()
                )))
                .unwrap(),
            Some(b"second".to_vec())
        );
    }
}
