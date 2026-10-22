use super::{Credentials, Edition};
use crate::services::{
    electron_storage,
    native_account_files::{DirectoryLock, Transaction},
    process_manager, tool_manager,
};
use std::path::{Path, PathBuf};

pub(super) struct Update {
    transaction: Transaction,
    _lock: DirectoryLock,
    _apply: tokio::sync::OwnedMutexGuard<()>,
}
impl Update {
    pub(super) fn commit(self) {
        self.transaction.commit();
    }
}

pub(super) fn data_dir(edition: Edition) -> Result<PathBuf, String> {
    let root = if cfg!(target_os = "macos") {
        dirs::home_dir()
            .ok_or("accountError.home")?
            .join("Library/Application Support")
    } else {
        dirs::config_dir().ok_or("accountError.home")?
    };
    Ok(root.join(edition.data_name()))
}
pub(super) fn machine_id(edition: Edition) -> Result<String, String> {
    let path = dirs::home_dir()
        .ok_or("accountError.home")?
        .join(format!(".{}/machine-id", edition.tool()));
    let value = std::fs::read_to_string(path).map_err(|_| "accountError.initializeClient")?;
    let value = value.trim();
    uuid::Uuid::parse_str(value).map_err(|_| "accountError.initializeClient")?;
    Ok(value.to_owned())
}
fn cipher(directory: &Path, edition: Edition) -> Result<electron_storage::Cipher, String> {
    #[cfg(target_os = "linux")]
    if std::fs::read(directory.join("auth-v2.dat")).is_ok_and(|bytes| bytes.starts_with(b"v10")) {
        let mut key = [0u8; 16];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(b"peanuts", b"saltysalt", 1, &mut key);
        return Ok(electron_storage::Cipher::Cbc {
            key,
            prefix: *b"v10",
            linux: true,
        });
    }
    electron_storage::cipher_with_name(directory, edition.data_name())
}
fn write_at(
    c: &Credentials,
    directory: &Path,
    cipher: &electron_storage::Cipher,
) -> Result<(Transaction, DirectoryLock), String> {
    c.validate()?;
    if !directory.is_dir() {
        return Err("accountError.initializeClient".into());
    }
    let lock = DirectoryLock::acquire(&directory.join("auth-v2.dat"))?;
    let bytes = serde_json::to_vec(&c.auth).map_err(|_| "accountError.format")?;
    let encrypted = electron_storage::encrypt_bytes(cipher, &bytes)?;
    let mut transaction = Transaction::new();
    transaction.replace(&directory.join("auth-v2.dat"), &encrypted)?;
    transaction.remove(&directory.join("auth-v2.dat.json"))?;
    Ok((transaction, lock))
}
pub(super) async fn apply(c: &Credentials) -> Result<Update, String> {
    // Keep the native transaction exclusive through the saved-account commit.
    static LOCK: std::sync::OnceLock<std::sync::Arc<tokio::sync::Mutex<()>>> =
        std::sync::OnceLock::new();
    let apply = LOCK
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
        .lock_owned()
        .await;
    let executable =
        tool_manager::get_tool_exe_path(c.edition.tool()).ok_or("accountError.unavailable")?;
    let metadata = tool_manager::packaged_desktop_metadata(Path::new(&executable))
        .ok_or("accountError.unavailable")?;
    if metadata["name"] != c.edition.package() {
        return Err("accountError.invalidAccount".into());
    }
    let directory = data_dir(c.edition)?;
    if !directory.is_dir() {
        return Err("accountError.initializeClient".into());
    }
    let cipher = cipher(&directory, c.edition)?;
    process_manager::stop_desktop_for_account(c.edition.tool()).await?;
    let (transaction, lock) = write_at(c, &directory, &cipher)?;
    Ok(Update {
        transaction,
        _lock: lock,
        _apply: apply,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_replacement_preserves_models_and_history_and_rolls_back_both_auth_formats() {
        for edition in [Edition::QwenWork, Edition::QwenWorkCn] {
            for cipher in electron_storage::test_ciphers() {
                let root =
                    std::env::temp_dir().join(format!("qwenwork-native-{}", uuid::Uuid::new_v4()));
                std::fs::create_dir_all(root.join("data")).unwrap();
                std::fs::write(root.join("data/agents.db"), b"model-and-session-fixture").unwrap();
                std::fs::write(root.join("auth-v2.dat"), b"original-encrypted-auth").unwrap();
                std::fs::write(root.join("auth-v2.dat.json"), b"original-fallback").unwrap();
                let c = super::super::tests::fixture(edition);
                let (transaction, lock) = write_at(&c, &root, &cipher).unwrap();
                let encrypted = std::fs::read(root.join("auth-v2.dat")).unwrap();
                let value: serde_json::Value = serde_json::from_slice(
                    &electron_storage::decrypt_bytes(&cipher, &encrypted).unwrap(),
                )
                .unwrap();
                assert_eq!(value, c.auth);
                assert!(!root.join("auth-v2.dat.json").exists());
                assert!(write_at(&c, &root, &cipher).is_err());
                drop(transaction);
                drop(lock);
                assert_eq!(
                    std::fs::read(root.join("auth-v2.dat")).unwrap(),
                    b"original-encrypted-auth"
                );
                assert_eq!(
                    std::fs::read(root.join("auth-v2.dat.json")).unwrap(),
                    b"original-fallback"
                );
                let (transaction, lock) = write_at(&c, &root, &cipher).unwrap();
                transaction.commit();
                drop(lock);
                assert_eq!(
                    std::fs::read(root.join("data/agents.db")).unwrap(),
                    b"model-and-session-fixture"
                );
                assert!(!root.join("auth-v2.dat.json").exists());
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }
}
