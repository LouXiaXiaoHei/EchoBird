use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("accountError.home".into());
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            component => result.push(component.as_os_str()),
        }
    }
    Ok(result)
}

pub(crate) fn private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| "accountError.write")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "accountError.write")?;
    }
    Ok(())
}

pub(crate) struct DirectoryLock(PathBuf);
impl DirectoryLock {
    pub(crate) fn acquire(anchor: &Path) -> Result<Self, String> {
        private_dir(anchor.parent().ok_or("accountError.home")?)?;
        let lock = PathBuf::from(format!("{}.lock", anchor.display()));
        fs::create_dir(&lock).map_err(|_| "accountError.busy")?;
        Ok(Self(lock))
    }
}
impl Drop for DirectoryLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("accountError.read".into()),
    }
}
pub(crate) fn object(path: &Path) -> Result<Value, String> {
    let value: Value = match read_optional(path)? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| "accountError.format")?,
        None => serde_json::json!({}),
    };
    value
        .is_object()
        .then_some(value)
        .ok_or_else(|| "accountError.format".into())
}
pub(crate) fn json(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(value).map_err(|_| "accountError.format".into())
}
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    private_dir(path.parent().ok_or("accountError.home")?)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|_| "accountError.write")?;
        file.write_all(bytes).map_err(|_| "accountError.write")?;
        file.sync_all().map_err(|_| "accountError.write")?;
        drop(file);
        fs::rename(&temporary, path).map_err(|_| "accountError.write".to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub(crate) struct Transaction {
    original: Vec<(PathBuf, Option<Vec<u8>>)>,
    committed: bool,
}
impl Transaction {
    pub(crate) fn new() -> Self {
        Self {
            original: Vec::new(),
            committed: false,
        }
    }
    pub(crate) fn replace(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        let old = read_optional(path)?;
        write(path, bytes)?;
        self.original.push((path.to_path_buf(), old));
        Ok(())
    }
    pub(crate) fn commit(mut self) {
        self.committed = true;
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for (path, original) in self.original.iter().rev() {
            let restored = match original {
                Some(bytes) => write(path, bytes),
                None => fs::remove_file(path).map_err(|_| "accountError.write".into()),
            };
            if restored.is_err() {
                log::error!("Failed to restore native account configuration");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_paths_normalize_dot_segments_without_resolving_symbolic_links() {
        let home = std::env::temp_dir();
        assert_eq!(
            absolute_path(&home.join("data/../account/./auth")).unwrap(),
            home.join("account/auth")
        );
        assert!(absolute_path(Path::new("relative/accounts")).is_err());
        #[cfg(unix)]
        {
            assert_eq!(
                absolute_path(Path::new("/../../accounts")).unwrap(),
                PathBuf::from("/accounts")
            );
        }
    }
    #[test]
    fn failed_multi_file_update_restores_prior_account_and_releases_lock() {
        let root = std::env::temp_dir().join(format!(
            "echobird-native-transaction-{}",
            uuid::Uuid::new_v4()
        ));
        let first = root.join("user");
        write(&first, b"original").unwrap();
        let guard = DirectoryLock::acquire(&root.join("auth.lock")).unwrap();
        assert!(DirectoryLock::acquire(&root.join("auth.lock")).is_err());
        {
            let mut transaction = Transaction::new();
            transaction.replace(&first, b"new").unwrap();
            transaction.replace(&root.join("new-user"), b"new").unwrap();
            assert!(transaction.replace(&first.join("invalid"), b"new").is_err());
        }
        assert_eq!(fs::read(&first).unwrap(), b"original");
        assert!(!root.join("new-user").exists());
        drop(guard);
        let _guard = DirectoryLock::acquire(&root.join("auth.lock")).unwrap();
        drop(_guard);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn committed_multi_file_update_is_private() {
        let root =
            std::env::temp_dir().join(format!("echobird-native-commit-{}", uuid::Uuid::new_v4()));
        let file = root.join("user");
        let mut transaction = Transaction::new();
        transaction.replace(&file, b"saved").unwrap();
        transaction.commit();
        assert_eq!(fs::read(&file).unwrap(), b"saved");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
