use super::*;
use rusqlite::Connection;
use rusty_leveldb::{Options, DB};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("mother-core-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn file(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
    fn store(&self) -> OutputStore {
        OutputStore::new(self.0.join("saved"))
    }
    fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        serde_json::from_str(&execute(
            name,
            args,
            "test",
            &self.store(),
            &CancellationToken::new(),
        )?)
        .map_err(|e| e.to_string())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn cancelled_diagnostics_cannot_read_or_write_late_state() {
    let f = Fixture::new();
    let path = f.file("settings.json", b"{\"port\":1}");
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(execute("file_repair",json!({"path":path,"expected_sha256":hash(b"{\"port\":1}"),"old_string":"1","new_string":"2"}),"test",&f.store(),&cancel).is_err());
    assert_eq!(fs::read(path).unwrap(), b"{\"port\":1}");
    assert!(!f.0.join("saved").exists());
}
fn archive(f: &Fixture, content: &str, offset: &str) -> PathBuf {
    let tree=serde_json::to_vec(&json!({"files":{"main.js":{"offset":offset,"size":content.len()},"alias":{"link":"../outside"}}})).unwrap();
    let block = (tree.len() + 11) & !3;
    let mut bytes = vec![0; block + 8];
    bytes[..4].copy_from_slice(&4u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&(block as u32).to_le_bytes());
    bytes[8..12].copy_from_slice(&((block - 4) as u32).to_le_bytes());
    bytes[12..16].copy_from_slice(&(tree.len() as u32).to_le_bytes());
    bytes[16..16 + tree.len()].copy_from_slice(&tree);
    bytes.extend(content.as_bytes());
    f.file("app.asar", bytes)
}
#[test]
fn asar_index_selected_text_search_and_unicode_pages_never_execute_or_modify() {
    let f = Fixture::new();
    let text = "中".repeat(3000) + "const model = 'fixture';";
    let path = archive(&f, &text, "0");
    let before = fs::read(&path).unwrap();
    let index = f.call("artifact_inspect", json!({"path":path})).unwrap();
    assert_eq!(index["total_entries"], 2);
    let first = f
        .call("artifact_inspect", json!({"path":path,"member":"main.js"}))
        .unwrap();
    assert_eq!(first["text"].as_str().unwrap().len(), 6000);
    let next = f
        .call(
            "artifact_inspect",
            json!({"path":path,"member":"main.js","offset":first["next_offset"]}),
        )
        .unwrap();
    assert_eq!(
        format!(
            "{}{}",
            first["text"].as_str().unwrap(),
            next["text"].as_str().unwrap()
        ),
        text
    );
    let hits = f
        .call(
            "artifact_inspect",
            json!({"path":path,"member":"main.js","search":"model"}),
        )
        .unwrap();
    assert_eq!(hits["matches"][0]["offset"], 9006);
    assert_eq!(hits["complete"], true);
    assert!(f
        .call("artifact_inspect", json!({"path":path,"member":"alias"}))
        .is_err());
    assert!(f
        .call(
            "artifact_inspect",
            json!({"path":path,"member":"../main.js"})
        )
        .is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}
#[test]
fn asar_rejects_corrupt_headers_outside_members_and_invalid_arguments() {
    let f = Fixture::new();
    let path = archive(&f, "content", "999999");
    assert!(f
        .call("artifact_inspect", json!({"path":path,"member":"main.js"}))
        .is_err());
    let path = f.file("bad.asar", b"truncated");
    assert!(f.call("artifact_inspect", json!({"path":path})).is_err());
    assert!(f
        .call(
            "artifact_inspect",
            json!({"path":"relative","unexpected":true})
        )
        .is_err());
    let path = archive(&f, "x", "0");
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() = 0x80;
    fs::write(&path, bytes).unwrap();
    assert!(f
        .call("artifact_inspect", json!({"path":path,"member":"main.js"}))
        .is_err());
}

#[test]
fn asar_search_resumes_across_a_multibyte_page_boundary() {
    let f = Fixture::new();
    let at = 1024 * 1024 - 2;
    let path = archive(&f, &("a".repeat(at) + "中文 marker"), "0");
    let first = f
        .call(
            "artifact_inspect",
            json!({"path":path,"member":"main.js","search":"中文"}),
        )
        .unwrap();
    assert_eq!(first["complete"], false);
    assert_eq!(first["matches"], json!([]));
    let second = f
        .call(
            "artifact_inspect",
            json!({"path":path,"member":"main.js","search":"中文","offset":first["next_offset"]}),
        )
        .unwrap();
    assert_eq!(second["matches"][0]["offset"], at);
    assert_eq!(second["complete"], true);
}

#[test]
fn repair_validates_supported_config_formats_before_touching_the_file() {
    let f = Fixture::new();
    for (name,original,old,new,bad) in [
        ("settings.toml","port = 1\n","1","2","["),
        ("settings.yaml","port: 1\n","1","2","["),
        ("settings.plist","<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>port</key><integer>1</integer></dict></plist>","<integer>1","<integer>2","<integer>"),
        ("settings.json","\u{feff}{\"port\":1}","1","2","]"),
    ] {
        let path=f.file(name,original);
        let args=|replacement:&str|json!({"path":path,"expected_sha256":hash(original.as_bytes()),"old_string":old,"new_string":replacement});
        assert!(f.call("file_repair",args(bad)).is_err(),"{name}");
        assert_eq!(fs::read_to_string(&path).unwrap(),original);
        let repaired=f.call("file_repair",args(new)).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(),original.replacen(old,new,1));
        f.call("file_repair",json!({"backup_id":repaired["backup_id"]})).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(),original);
    }
}
#[test]
fn sqlite_wal_copy_returns_committed_rows_and_leaves_original_files_untouched() {
    let f = Fixture::new();
    let path = f.0.join("client.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE settings (name TEXT, value TEXT); INSERT INTO settings VALUES ('model', 'custom');").unwrap();
    let original: Vec<_> = fs::read_dir(&f.0)
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let schema = f
        .call("storage_inspect", json!({"path":path,"format":"sqlite"}))
        .unwrap();
    assert_eq!(schema["tables"][0]["table"], "settings");
    let rows = f
        .call(
            "storage_inspect",
            json!({"path":path,"format":"sqlite","table":"settings","columns":["name","value"]}),
        )
        .unwrap();
    assert_eq!(rows["rows"][0]["value"], "custom");
    assert!(f
        .call(
            "storage_inspect",
            json!({"path":path,"format":"sqlite","table":"settings","columns":["missing"]})
        )
        .is_err());
    assert!(f
        .call(
            "storage_inspect",
            json!({"path":path,"format":"sqlite","table":"settings"})
        )
        .is_err());
    for (path, bytes) in original {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
#[test]
fn leveldb_reads_only_private_copy_with_chromium_encoding_and_pages_keys() {
    let f = Fixture::new();
    let path = f.0.join("leveldb");
    let mut db = DB::open(
        &path,
        Options {
            create_if_missing: true,
            ..Default::default()
        },
    )
    .unwrap();
    let mut value = vec![0];
    value.extend(
        "{\"model\":\"中文\"}"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    db.put(b"_file://\0\x01model", &value).unwrap();
    db.flush().unwrap();
    let before = fs::read(path.join("CURRENT")).unwrap();
    let keys = f
        .call("storage_inspect", json!({"path":path,"format":"leveldb"}))
        .unwrap();
    assert_eq!(keys["keys"][0]["text"], "_file://\0\u{1}model");
    let row = f
        .call(
            "storage_inspect",
            json!({"path":path,"format":"leveldb","key":"_file://\0\u{1}model"}),
        )
        .unwrap();
    assert_eq!(row["value"]["text"], "{\"model\":\"中文\"}");
    assert_eq!(fs::read(path.join("CURRENT")).unwrap(), before);
    assert_eq!(&*db.get(b"_file://\0\x01model").unwrap(), value);
}
#[test]
fn missing_or_corrupt_storage_never_creates_or_repairs_the_source() {
    let f = Fixture::new();
    let missing = f.0.join("missing.db");
    assert!(f
        .call("storage_inspect", json!({"path":missing,"format":"sqlite"}))
        .is_err());
    assert!(!missing.exists());
    assert!(f
        .call("storage_inspect", json!({"path":f.0,"format":"leveldb"}))
        .is_err());
    assert!(!f.0.join("CURRENT").exists());
    let path = f.file("bad.db", b"bad database");
    assert!(f
        .call("storage_inspect", json!({"path":path,"format":"sqlite"}))
        .is_err());
    assert_eq!(fs::read(path).unwrap(), b"bad database");
}
#[test]
fn snapshots_compare_changed_fields_without_exposing_values_and_are_scoped() {
    let f = Fixture::new();
    let path = f.file(
        "config.json",
        br#"{"model":"old","apiKey":"fixture-secret"}"#,
    );
    let before = f
        .call("diagnostic_snapshot", json!({"paths":[path]}))
        .unwrap();
    assert!(!before.to_string().contains("fixture-secret"));
    fs::write(&path, br#"{"model":"new","apiKey":"fixture-secret"}"#).unwrap();
    let after = f
        .call(
            "diagnostic_snapshot",
            json!({"paths":[path],"baseline_id":before["snapshot_id"]}),
        )
        .unwrap();
    assert_eq!(after["files"][0]["changed_json_paths"], json!(["/model"]));
    assert!(execute(
        "diagnostic_snapshot",
        json!({"paths":[path],"baseline_id":before["snapshot_id"]}),
        "other",
        &f.store(),
        &CancellationToken::new()
    )
    .is_err());
    fs::remove_file(&path).unwrap();
    let removed = f
        .call(
            "diagnostic_snapshot",
            json!({"paths":[path],"baseline_id":after["snapshot_id"]}),
        )
        .unwrap();
    assert_eq!(removed["files"][0]["changed"], true);
    assert!(removed["files"][0]["sha256"].is_null());
}
#[test]
fn repair_checks_old_hash_and_format_then_preserves_comments_and_restores_exact_bytes() {
    let f = Fixture::new();
    let original = "// keep comment\n{\"port\":1}\n";
    let path = f.file("config.jsonc", original);
    let edit = json!({"path":path,"expected_sha256":hash(original.as_bytes()),"old_string":"\"port\":1","new_string":"\"port\":2"});
    let repaired = f.call("file_repair", edit.clone()).unwrap();
    let recovery = f
        .call("diagnostic_snapshot", json!({"paths":[path]}))
        .unwrap();
    assert_eq!(
        recovery["files"][0]["matching_restore_backups"][0]["backup_id"],
        repaired["backup_id"]
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original.replace(":1", ":2")
    );
    assert!(f.call("file_repair", edit).is_err());
    assert!(execute(
        "file_repair",
        json!({"backup_id":repaired["backup_id"]}),
        "other",
        &f.store(),
        &CancellationToken::new()
    )
    .is_err());
    let restored = f
        .call("file_repair", json!({"backup_id":repaired["backup_id"]}))
        .unwrap();
    assert_eq!(restored["restored"], true);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(f
        .call("file_repair", json!({"backup_id":repaired["backup_id"]}))
        .is_err());
}
#[test]
fn rejected_repair_preserves_original_and_restore_never_overwrites_newer_manual_changes() {
    let f = Fixture::new();
    let original = r#"{"port":1}"#;
    let path = f.file("config.json", original);
    let args = |old: &str, new: &str| json!({"path":path,"expected_sha256":hash(original.as_bytes()),"old_string":old,"new_string":new});
    assert!(f.call("file_repair", args("1", "}")).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(f.call("file_repair", args("missing", "2")).is_err());
    let repaired = f.call("file_repair", args("1", "2")).unwrap();
    fs::write(&path, r#"{"port":3}"#).unwrap();
    assert!(f
        .call("file_repair", json!({"backup_id":repaired["backup_id"]}))
        .is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), r#"{"port":3}"#);
}
#[test]
fn restoring_a_repaired_broken_config_can_restore_the_original_invalid_text() {
    let f = Fixture::new();
    let path = f.file("broken.json", "{\"port\":}");
    let repaired=f.call("file_repair",json!({"path":path,"expected_sha256":hash(b"{\"port\":}"),"old_string":":}","new_string":":1}"})).unwrap();
    f.call("file_repair", json!({"backup_id":repaired["backup_id"]}))
        .unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "{\"port\":}");
}

#[cfg(unix)]
#[test]
fn text_repairs_preserve_parent_permissions_and_executable_file_modes() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let path = f.file("script.sh", "echo old\n");
    fs::set_permissions(&f.0, fs::Permissions::from_mode(0o750)).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    let repaired=f.call("file_repair",json!({"path":path,"expected_sha256":hash(b"echo old\n"),"old_string":"old","new_string":"new"})).unwrap();
    assert_eq!(
        fs::metadata(&f.0).unwrap().permissions().mode() & 0o777,
        0o750
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o751
    );
    f.call("file_repair", json!({"backup_id":repaired["backup_id"]}))
        .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o751
    );
}
