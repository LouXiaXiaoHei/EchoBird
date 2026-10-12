//! Shared ZCode desktop storage paths.

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) struct ZCodePaths {
    pub config_dir: PathBuf,
    pub settings_file: PathBuf,
}

pub(crate) fn resolve() -> Result<ZCodePaths, String> {
    let home = dirs::home_dir().ok_or("accountError.home")?;
    resolve_at(&home, std::env::var("ZCODE_DATA_BASE_DIR").ok().as_deref())
}

pub(crate) fn resolve_at(home: &Path, env_base: Option<&str>) -> Result<ZCodePaths, String> {
    let settings_file = home.join(".zcode/v2/setting.json");
    let settings: Value = match fs::read_to_string(&settings_file) {
        Ok(raw) => serde_json::from_str::<Value>(&raw)
            .ok()
            .filter(Value::is_object)
            .ok_or("accountError.format")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(_) => return Err("accountError.read".into()),
    };
    let base = settings
        .get("dataBaseDir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| env_base.map(str::trim).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.to_path_buf());
    Ok(ZCodePaths {
        config_dir: base.join(".zcode/v2"),
        settings_file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn zcode_data_paths_follow_bootstrap_then_environment_then_home_without_writes() {
        let root =
            std::env::temp_dir().join(format!("echobird-zcode-paths-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let custom = root.join("custom data");
        let environment = root.join("environment");
        let settings_file = home.join(".zcode/v2/setting.json");
        let environment_value = format!("  {}  ", environment.display());
        let paths = resolve_at(&home, None).unwrap();
        assert_eq!(paths.config_dir, home.join(".zcode/v2"));
        assert_eq!(paths.settings_file, settings_file);
        assert!(!root.exists());
        assert_eq!(
            resolve_at(&home, Some(&environment_value))
                .unwrap()
                .config_dir,
            environment.join(".zcode/v2")
        );
        assert_eq!(
            resolve_at(&home, Some("  ")).unwrap().config_dir,
            home.join(".zcode/v2")
        );
        fs::create_dir_all(settings_file.parent().unwrap()).unwrap();
        for value in [Value::Null, json!(123), json!("  ")] {
            fs::write(&settings_file, json!({"dataBaseDir":value}).to_string()).unwrap();
            assert_eq!(
                resolve_at(&home, Some(&environment_value))
                    .unwrap()
                    .config_dir,
                environment.join(".zcode/v2")
            );
        }
        fs::write(
            &settings_file,
            json!({"dataBaseDir":format!("  {}  ", custom.display()),"theme":"dark"}).to_string(),
        )
        .unwrap();
        let before = fs::read(&settings_file).unwrap();
        let paths = resolve_at(&home, Some(&environment_value)).unwrap();
        assert_eq!(paths.config_dir, custom.join(".zcode/v2"));
        assert_eq!(paths.settings_file, settings_file);
        assert_eq!(fs::read(&settings_file).unwrap(), before);
        assert!(!custom.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zcode_corrupt_bootstrap_does_not_redirect_operations_to_home() {
        let root =
            std::env::temp_dir().join(format!("echobird-zcode-paths-{}", uuid::Uuid::new_v4()));
        let settings_file = root.join(".zcode/v2/setting.json");
        fs::create_dir_all(settings_file.parent().unwrap()).unwrap();
        for invalid in ["broken", "null", "[]"] {
            fs::write(&settings_file, invalid).unwrap();
            assert!(resolve_at(&root, None).is_err());
            assert_eq!(fs::read_to_string(&settings_file).unwrap(), invalid);
        }
        assert!(!root.join(".zcode/v2/provider_config.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
