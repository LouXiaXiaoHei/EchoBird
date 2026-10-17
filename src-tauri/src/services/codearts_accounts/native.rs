use super::*;
use std::path::Path;

pub(crate) fn desktop_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    let root = dirs::home_dir().map(|path| path.join("Library/Application Support"));
    #[cfg(not(target_os = "macos"))]
    let root = dirs::config_dir();
    Ok(root.ok_or("accountError.home")?.join("codearts"))
}

pub(crate) fn encrypt_desktop_secret(root: &Path, value: &str) -> Result<Vec<u8>, String> {
    let key = super::super::electron_storage::cipher_with_name(root, "codearts")?;
    STANDARD
        .decode(super::super::electron_storage::encrypt(&key, value)?)
        .map_err(|_| "accountError.keychain".into())
}

pub(crate) fn decrypt_desktop_secret(root: &Path, bytes: &[u8]) -> Result<String, String> {
    let key = super::super::electron_storage::cipher_with_name(root, "codearts")?;
    super::super::electron_storage::decrypt(&key, &STANDARD.encode(bytes))
}

fn desktop_path(root: &Path) -> PathBuf {
    root.join("secretStorage").join(format!(
        "{:x}.enc",
        Sha256::digest(b"authSession:cn-north-4")
    ))
}

pub(super) fn read() -> Result<Option<Value>, String> {
    let root = desktop_data_dir()?;
    let path = desktop_path(&root);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path).map_err(|_| "accountError.read")?;
    let bytes = STANDARD
        .decode(raw.trim())
        .map_err(|_| "accountError.format")?;
    let raw = decrypt_desktop_secret(&root, &bytes)?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|_| "accountError.format".into())
}

pub(super) fn apply(
    saved: &Saved,
) -> Result<super::super::native_account_files::Transaction, String> {
    apply_at(&desktop_data_dir()?, saved)
}

fn apply_at(
    root: &Path,
    saved: &Saved,
) -> Result<super::super::native_account_files::Transaction, String> {
    credentials(saved)?;
    let raw = serde_json::to_string(&saved.native).map_err(|_| "accountError.format")?;
    let encrypted = STANDARD.encode(encrypt_desktop_secret(root, &raw)?);
    let mut transaction = super::super::native_account_files::Transaction::new();
    transaction.replace(&desktop_path(root), encrypted.as_bytes())?;
    Ok(transaction)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn wrap_windows_key(bytes: &[u8]) -> Result<Vec<u8>, String> {
        use windows::Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::Cryptography::{
                CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
            },
        };
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes
                .len()
                .try_into()
                .map_err(|_| "accountError.keychain")?,
            pbData: bytes.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|_| "accountError.keychain")?;
            let value = if output.pbData.is_null() {
                Err("accountError.keychain".into())
            } else {
                Ok(std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec())
            };
            let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
            value
        }
    }

    #[test]
    fn desktop_windows_secret_uses_local_state_gcm_and_rejects_wrong_keys() {
        let root = std::env::temp_dir().join(format!(
            "echobird-codearts-gcm-fixture-{}",
            uuid::Uuid::new_v4()
        ));
        let state = root.join("Local State");
        let make_state = |key: &[u8]| {
            let mut wrapped = b"DPAPI".to_vec();
            wrapped.extend(wrap_windows_key(key).unwrap());
            super::super::super::cursor_auth::write(
                &state,
                &json!({"os_crypt":{"encrypted_key":STANDARD.encode(wrapped)}}),
            )
            .unwrap();
        };
        make_state(&[7; 32]);
        let bytes = encrypt_desktop_secret(&root, "fixture-token-中文").unwrap();
        assert_eq!(
            decrypt_desktop_secret(&root, &bytes).unwrap(),
            "fixture-token-中文"
        );
        assert!(bytes.starts_with(b"v10"));
        assert!(decrypt_desktop_secret(&root, b"invalid").is_err());
        let settings = root.join("app-settings.json");
        let model_secret = root.join("secretStorage/api-model.enc");
        let selection = root.join("Local Storage/leveldb/fixture-selection");
        let api_settings = br#"{"customModels":[{"uid":"echobird-managed-model","modelId":"fixture-api"}],"unknown":true}"#;
        std::fs::create_dir_all(selection.parent().unwrap()).unwrap();
        std::fs::create_dir_all(model_secret.parent().unwrap()).unwrap();
        std::fs::write(&settings, api_settings).unwrap();
        std::fs::write(&model_secret, b"keep-model-key").unwrap();
        std::fs::write(&selection, b"keep-api-selection").unwrap();
        let native = json!({"session":{"clientId":"codearts","userId":"fixture-user","domainId":"fixture-domain","userName":"Fixture","accessKey":"ak","secretKey":"sk","securityToken":"token","expiresAt":4102444800000_i64}});
        let saved = Saved {
            account: account(&native).unwrap(),
            native: native.clone(),
        };
        apply_at(&root, &saved).unwrap().commit();
        let encoded = std::fs::read_to_string(desktop_path(&root)).unwrap();
        let decoded = decrypt_desktop_secret(&root, &STANDARD.decode(encoded).unwrap()).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&decoded).unwrap(), native);
        assert_eq!(std::fs::read(settings).unwrap(), api_settings);
        assert_eq!(std::fs::read(model_secret).unwrap(), b"keep-model-key");
        assert_eq!(std::fs::read(selection).unwrap(), b"keep-api-selection");
        make_state(&[8; 32]);
        assert!(decrypt_desktop_secret(&root, &bytes).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
