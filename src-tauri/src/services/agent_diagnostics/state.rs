use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SnapshotArgs {
    paths: Vec<String>,
    baseline_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct FileState {
    path: PathBuf,
    sha256: Option<String>,
    bytes: Option<usize>,
    fields: BTreeMap<String, String>,
    fields_complete: bool,
}
fn fields(value: &Value, path: &str, out: &mut BTreeMap<String, String>) -> Result<(), String> {
    if out.len() >= 2048 {
        return Err("JSON field index exceeds 2048 leaves".into());
    }
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, value) in map {
                fields(
                    value,
                    &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                    out,
                )?;
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (i, value) in items.iter().enumerate() {
                fields(value, &format!("{path}/{i}"), out)?;
            }
        }
        value => {
            out.insert(
                path.into(),
                hash(&serde_json::to_vec(value).map_err(|e| e.to_string())?),
            );
        }
    }
    Ok(())
}
pub(super) fn snapshot(
    args: SnapshotArgs,
    scope: &str,
    store: &OutputStore,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    if args.paths.is_empty() || args.paths.len() > 16 {
        return Err("Select 1-16 explicit files".into());
    }
    let previous: Vec<FileState> = if let Some(id) = args.baseline_id {
        load("diagnostic_snapshot", &id, scope, store)?
    } else {
        Vec::new()
    };
    let mut states = Vec::new();
    for raw in args.paths {
        check_cancel(cancel)?;
        let requested = Path::new(&raw);
        if !requested.is_absolute() {
            return Err("Snapshot paths must be absolute".into());
        }
        let path = if requested.exists() {
            absolute(&raw)?
        } else {
            fs::canonicalize(requested.parent().ok_or("Missing parent")?)
                .map_err(|e| e.to_string())?
                .join(requested.file_name().ok_or("Missing filename")?)
        };
        if states.iter().any(|state: &FileState| state.path == path) {
            continue;
        }
        let bytes = if path.exists() {
            Some(read(&path, FILE_LIMIT)?)
        } else {
            None
        };
        let mut state = FileState {
            path,
            sha256: bytes.as_deref().map(hash),
            bytes: bytes.as_ref().map(Vec::len),
            fields: BTreeMap::new(),
            fields_complete: false,
        };
        if let Some(bytes) = bytes {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                if let Ok(value) =
                    jsonc_parser::parse_to_serde_value::<Value>(text, &Default::default())
                {
                    state.fields_complete = fields(&value, "", &mut state.fields).is_ok();
                    if !state.fields_complete {
                        state.fields.clear();
                    }
                }
            }
        }
        states.push(state);
    }
    check_cancel(cancel)?;
    let backups: Vec<_> = store
        .ids(scope)?
        .into_iter()
        .filter_map(|id| {
            load::<Backup>("diagnostic_repair", &id, scope, store)
                .ok()
                .map(|backup| (id, backup))
        })
        .collect();
    let id = saved("diagnostic_snapshot", &states, scope, store)?;
    let changes: Vec<_>=states.iter().map(|state| {
        let old=previous.iter().find(|old|old.path==state.path);
        let changed=old.map(|old|old.sha256!=state.sha256);
        let mut paths=BTreeSet::new();
        if let Some(old)=old.filter(|old|old.fields_complete && state.fields_complete) {
            for key in old.fields.keys().chain(state.fields.keys()) {if old.fields.get(key)!=state.fields.get(key) {paths.insert(key);}}
        }
        let matching: Vec<_> = backups.iter().filter(|(_,backup)|backup.path==state.path && state.sha256.as_deref()==Some(&backup.after_sha256)).map(|(id,backup)|json!({"backup_id":id,"before_sha256":hash(backup.before.as_bytes())})).collect();
        json!({"path":state.path,"sha256":state.sha256,"bytes":state.bytes,"changed":changed,"changed_json_paths":paths,"json_index_complete":state.fields_complete,"matching_restore_backups":matching})
    }).collect();
    Ok(
        json!({"snapshot_id":id,"files":changes,"compared":!previous.is_empty(),"coverage":"Only requested files; hashes and JSON field paths do not prove runtime state"}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RepairArgs {
    path: Option<String>,
    expected_sha256: Option<String>,
    old_string: Option<String>,
    new_string: Option<String>,
    backup_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Backup {
    path: PathBuf,
    before: String,
    after_sha256: String,
}
fn validate(path: &Path, text: &str) -> Result<(), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => {
            serde_json::from_str::<Value>(text).map_err(|e| e.to_string())?;
        }
        "jsonc" => {
            jsonc_parser::parse_to_serde_value::<Value>(text, &Default::default())
                .map_err(|e| e.to_string())?;
        }
        "toml" => {
            text.parse::<toml_edit::DocumentMut>()
                .map_err(|e| e.to_string())?;
        }
        "yaml" | "yml" => {
            serde_yaml_ng::from_str::<Value>(text).map_err(|e| e.to_string())?;
        }
        "plist" => {
            plist::Value::from_reader(std::io::Cursor::new(text.as_bytes()))
                .map_err(|e| e.to_string())?;
        }
        "exe" | "dll" | "so" | "dylib" | "asar" | "db" | "sqlite" | "sqlite3" | "sst" | "ldb"
        | "wal" => {
            return Err("Repair only supports text/config files, not programs or databases".into())
        }
        _ => {}
    }
    Ok(())
}
pub(super) fn repair(
    args: RepairArgs,
    scope: &str,
    store: &OutputStore,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let (path, expected, old, new, restore) = if let Some(id) = args.backup_id {
        if args.path.is_some()
            || args.expected_sha256.is_some()
            || args.old_string.is_some()
            || args.new_string.is_some()
        {
            return Err("Restore accepts only backup_id".into());
        }
        let backup: Backup = load("diagnostic_repair", &id, scope, store)?;
        (backup.path, backup.after_sha256, None, backup.before, true)
    } else {
        let path = absolute(&args.path.ok_or("path required")?)?;
        (
            path,
            args.expected_sha256.ok_or("expected_sha256 required")?,
            Some(args.old_string.ok_or("old_string required")?),
            args.new_string.ok_or("new_string required")?,
            false,
        )
    };
    if path.components().any(|s| {
        s.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".echobird")
    }) {
        return Err("EchoBird internal files cannot be repaired by this tool".into());
    }
    if fs::symlink_metadata(&path)
        .map_err(|e| e.to_string())?
        .is_symlink()
    {
        return Err(
            "File was replaced by a symbolic link; inspect the resolved target first".into(),
        );
    }
    let _lock = super::super::native_account_files::DirectoryLock::acquire_existing(&path)?;
    let bytes = read(&path, 512 * 1024)?;
    if hash(&bytes) != expected {
        return Err("File changed since inspection; re-read it before repair or restore".into());
    }
    let before = String::from_utf8(bytes).map_err(|_| "Repair requires UTF-8 text")?;
    if before.contains('\0') {
        return Err("Repair refuses binary text containing NUL bytes".into());
    }
    let updated = if let Some(old) = old {
        if old.is_empty() || before.matches(&old).count() != 1 {
            return Err("old_string must match exactly once".into());
        }
        before.replacen(&old, &new, 1)
    } else {
        new
    };
    if updated.len() > 512 * 1024 {
        return Err("Repaired text exceeds 512 KiB".into());
    }
    if !restore {
        validate(&path, &updated)
            .map_err(|e| format!("Format check failed; original file preserved: {e}"))?;
    }
    let after_sha256 = hash(updated.as_bytes());
    check_cancel(cancel)?;
    let backup_id = saved(
        "diagnostic_repair",
        &Backup {
            path: path.clone(),
            before: before.clone(),
            after_sha256: after_sha256.clone(),
        },
        scope,
        store,
    )?;
    if hash(&read(&path, 512 * 1024)?) != expected {
        return Err("File changed before write; nothing was repaired".into());
    }
    check_cancel(cancel)?;
    let permissions = fs::metadata(&path)
        .map_err(|e| e.to_string())?
        .permissions();
    let mut tx = super::super::native_account_files::Transaction::new();
    tx.replace_private_file(&path, updated.as_bytes())?;
    let verify = (|| {
        fs::set_permissions(&path, permissions.clone()).map_err(|e| e.to_string())?;
        if hash(&read(&path, 512 * 1024)?) != after_sha256 {
            return Err("Read-back verification failed; repair rolled back".into());
        }
        check_cancel(cancel)
    })();
    if let Err(error) = verify {
        drop(tx);
        fs::set_permissions(&path, permissions)
            .map_err(|e| format!("{error}; restoring file permissions failed: {e}"))?;
        return Err(error);
    }
    tx.commit();
    Ok(
        json!({"path":path,"sha256":after_sha256,"backup_id":backup_id,"restored":restore,"file_verified":true,"application_verified":false}),
    )
}
