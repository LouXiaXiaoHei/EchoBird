use super::{
    collect_dsh_paths, collect_jsonl_paths, drizzle_sessions_sql, drizzle_usage_boundary,
    dsh_decode, dsh_owns_usage, is_unmeasured_drizzle_zero, mimo_db_path, Family, TokenUsage,
};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Serialize)]
pub struct DailyTokenEntry {
    pub ts: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Serialize)]
pub struct DailyTokenResponse {
    pub entries: Vec<DailyTokenEntry>,
    pub unavailable_families: Vec<String>,
}

#[derive(Clone)]
struct Sample {
    id: Option<String>,
    entry: DailyTokenEntry,
}

#[derive(Clone, Default)]
struct Scan {
    samples: Vec<Sample>,
    incomplete: bool,
}

struct CachedScan {
    mtime: SystemTime,
    size: u64,
    scan: Scan,
}

fn scan_cache() -> &'static Mutex<HashMap<PathBuf, CachedScan>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedScan>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn iso_timestamp(value: Option<&Value>) -> Option<i64> {
    let ts = chrono::DateTime::parse_from_rfc3339(value?.as_str()?)
        .ok()?
        .timestamp();
    (ts > 0).then_some(ts)
}

fn millis_timestamp(value: Option<&Value>) -> Option<i64> {
    let value = value?.as_i64()?;
    (value > 0).then_some(value / 1000)
}

fn entry(ts: Option<i64>, usage: TokenUsage) -> Option<DailyTokenEntry> {
    Some(DailyTokenEntry {
        ts: ts?,
        input_tokens: usage.input?,
        output_tokens: usage.output?,
    })
}

fn scan_claude(path: &Path) -> Scan {
    let Ok(file) = std::fs::File::open(path) else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let mut samples: HashMap<String, Option<DailyTokenEntry>> = HashMap::new();
    let mut incomplete = false;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            incomplete = true;
            break;
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            if !line.trim().is_empty() {
                incomplete = true;
            }
            continue;
        };
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let id = message
            .get("id")
            .or_else(|| value.get("uuid"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}:{}", path.to_string_lossy(), samples.len()));
        samples.insert(
            id,
            entry(
                iso_timestamp(value.get("timestamp")),
                TokenUsage::read(usage, "input_tokens", "output_tokens"),
            ),
        );
    }
    let mut scan = Scan {
        incomplete,
        ..Scan::default()
    };
    for (id, sample) in samples {
        if let Some(entry) = sample {
            scan.samples.push(Sample {
                id: Some(id),
                entry,
            });
        } else {
            scan.incomplete = true;
        }
    }
    scan
}

struct CodexSample {
    usage: TokenUsage,
    ts: Option<i64>,
}

#[derive(Default)]
pub(super) struct CodexUsage {
    previous: Option<TokenUsage>,
    metadata_seen: bool,
    thread_id: Option<String>,
    forked: bool,
    history_start: Option<u64>,
    response_samples: HashMap<String, CodexSample>,
    legacy_samples: Vec<CodexSample>,
    has_modern_usage: bool,
    rebase_after_rollback: bool,
    incomplete: bool,
}

impl CodexUsage {
    pub(super) fn consume(&mut self, value: &Value) {
        let Some(payload) = value.get("payload") else {
            return;
        };
        let kind = value.get("type").and_then(Value::as_str);
        if kind == Some("session_meta") && !self.metadata_seen {
            self.metadata_seen = true;
            self.thread_id = payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string);
            self.forked = payload
                .get("forked_from_id")
                .and_then(Value::as_str)
                .is_some();
            self.history_start = payload
                .get("subagent_history_start_ordinal")
                .and_then(Value::as_u64);
            if self.forked && self.history_start.is_none() {
                self.incomplete = true;
            }
        }
        let owns_record = match self.history_start {
            Some(start) => value
                .get("ordinal")
                .and_then(Value::as_u64)
                .is_some_and(|ordinal| ordinal >= start),
            None => !self.forked,
        };
        if kind == Some("token_usage_record") {
            let own_thread = match (
                self.thread_id.as_deref(),
                payload.get("thread_id").and_then(Value::as_str),
            ) {
                (Some(expected), Some(actual)) => expected == actual,
                _ => owns_record,
            };
            if !own_thread || (self.history_start.is_some() && !owns_record) {
                return;
            }
            // Native per-response records precede their legacy counter updates.
            // Keep legacy-only history, then use the measured response records.
            self.has_modern_usage = true;
            let Some(id) = payload
                .get("response_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                self.incomplete = true;
                return;
            };
            let Some(usage) = payload.get("usage") else {
                self.incomplete = true;
                return;
            };
            self.response_samples.insert(
                id.to_string(),
                CodexSample {
                    usage: TokenUsage::read(usage, "input_tokens", "output_tokens"),
                    ts: iso_timestamp(value.get("timestamp")),
                },
            );
            return;
        }
        if kind == Some("event_msg")
            && payload.get("type").and_then(Value::as_str) == Some("thread_rolled_back")
        {
            self.rebase_after_rollback = true;
            self.incomplete = true;
            return;
        }
        if kind != Some("event_msg")
            || payload.get("type").and_then(Value::as_str) != Some("token_count")
        {
            return;
        }
        if self.has_modern_usage {
            return;
        }
        let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
            return;
        };
        let total = info
            .get("total_token_usage")
            .map(|usage| TokenUsage::read(usage, "input_tokens", "output_tokens"))
            .unwrap_or_default();
        // Context-window exhaustion also emits synthetic zeroed counters.
        // Its nonzero total_tokens is not measured input/output usage.
        let synthetic_zero = total.input == Some(0)
            && total.output == Some(0)
            && info
                .pointer("/total_token_usage/total_tokens")
                .and_then(Value::as_u64)
                .is_some_and(|total| total > 0);
        let last = info
            .get("last_token_usage")
            .map(|usage| TokenUsage::read(usage, "input_tokens", "output_tokens"))
            .unwrap_or_default();
        if synthetic_zero || self.rebase_after_rollback {
            self.rebase_after_rollback = false;
            self.incomplete = true;
            self.previous = Some(total);
            return;
        }
        let mut ts = iso_timestamp(value.get("timestamp"));
        let delta = match (total.input, total.output, self.previous) {
            (Some(input), Some(output), None) => {
                if last.input != Some(input) || last.output != Some(output) {
                    // An initial cumulative snapshot is known for the session,
                    // but its previous requests cannot be assigned to this day.
                    self.incomplete = true;
                    ts = None;
                }
                Some(total)
            }
            (Some(input), Some(output), Some(before))
                if last.input == Some(input)
                    && last.output == Some(output)
                    && (before.input.is_some_and(|before| input < before)
                        || before.output.is_some_and(|before| output < before)) =>
            {
                self.incomplete = true;
                Some(total)
            }
            (
                Some(input),
                Some(output),
                Some(TokenUsage {
                    input: Some(before_input),
                    output: Some(before_output),
                }),
            ) => {
                if input >= before_input && output >= before_output {
                    if input == before_input && output == before_output {
                        None
                    } else {
                        Some(TokenUsage {
                            input: Some(input - before_input),
                            output: Some(output - before_output),
                        })
                    }
                } else {
                    self.incomplete = true;
                    (last.input == Some(input) && last.output == Some(output)).then_some(total)
                }
            }
            _ => {
                self.incomplete = true;
                ts = None;
                let before = self.previous.unwrap_or_default();
                let known_delta = |current: Option<u64>, previous: Option<u64>| {
                    current.and_then(|current| match previous {
                        Some(previous) => current.checked_sub(previous),
                        None => Some(current),
                    })
                };
                Some(TokenUsage {
                    input: known_delta(total.input, before.input),
                    output: known_delta(total.output, before.output),
                })
            }
        };
        let before = self.previous.unwrap_or_default();
        self.previous = Some(TokenUsage {
            input: total.input.or(before.input),
            output: total.output.or(before.output),
        });
        if owns_record {
            if let Some(usage) = delta {
                self.legacy_samples.push(CodexSample { usage, ts });
            }
        }
    }

    pub(super) fn total_usage(&self) -> TokenUsage {
        let mut usage = TokenUsage::default();
        for sample in self
            .legacy_samples
            .iter()
            .chain(self.response_samples.values())
        {
            usage.add(sample.usage);
        }
        usage
    }

    fn into_scan(self) -> Scan {
        let mut scan = Scan {
            incomplete: self.incomplete,
            ..Scan::default()
        };
        for (id, sample) in self
            .legacy_samples
            .into_iter()
            .map(|sample| (None, sample))
            .chain(
                self.response_samples
                    .into_iter()
                    .map(|(id, sample)| (Some(id), sample)),
            )
        {
            if let Some(entry) = entry(sample.ts, sample.usage) {
                scan.samples.push(Sample { id, entry });
            } else {
                scan.incomplete = true;
            }
        }
        scan
    }
}

fn scan_codex(path: &Path) -> Scan {
    let Ok(file) = std::fs::File::open(path) else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let mut usage = CodexUsage::default();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            usage.incomplete = true;
            break;
        };
        match serde_json::from_str::<Value>(&line) {
            Ok(value) => usage.consume(&value),
            Err(_) if !line.trim().is_empty() => usage.incomplete = true,
            Err(_) => {}
        }
    }
    usage.into_scan()
}

fn scan_deepseek(path: &Path) -> Scan {
    let Some(bytes) = dsh_decode(path) else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let mut samples: HashMap<(u64, u64), Option<DailyTokenEntry>> = HashMap::new();
    let mut scan = Scan::default();
    let mut lines = text.lines();
    let Some(header) = lines
        .next()
        .and_then(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|header| header.get("type").and_then(Value::as_str) == Some("session"))
    else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            if !line.trim().is_empty() {
                scan.incomplete = true;
            }
            continue;
        };
        let Some(data) = value.get("data") else {
            continue;
        };
        let usage = match value.get("type").and_then(Value::as_str) {
            Some("assistant/message") => data.get("usage"),
            Some("assistant/chunk")
                if data.pointer("/chunk/type").and_then(Value::as_str) == Some("usage") =>
            {
                data.pointer("/chunk/usage")
            }
            _ => None,
        };
        let Some(usage) = usage else { continue };
        match dsh_owns_usage(&header, &value) {
            Some(true) => {}
            Some(false) => continue,
            None => {
                scan.incomplete = true;
                continue;
            }
        }
        let (Some(turn), Some(step)) = (
            data.get("turn").and_then(Value::as_u64),
            data.get("step").and_then(Value::as_u64),
        ) else {
            scan.incomplete = true;
            continue;
        };
        samples.insert(
            (turn, step),
            entry(
                millis_timestamp(value.get("time")),
                TokenUsage::read(usage, "inputTokens", "outputTokens"),
            ),
        );
    }
    for sample in samples.into_values() {
        if let Some(entry) = sample {
            scan.samples.push(Sample { id: None, entry });
        } else {
            scan.incomplete = true;
        }
    }
    scan
}

fn cached_scan(path: &Path, family: Family) -> Scan {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let Ok(mtime) = metadata.modified() else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let size = metadata.len();
    if let Ok(cache) = scan_cache().lock() {
        if let Some(cached) = cache
            .get(path)
            .filter(|cached| cached.mtime == mtime && cached.size == size)
        {
            return cached.scan.clone();
        }
    }
    let scan = match family {
        Family::Claude => scan_claude(path),
        Family::Codex => scan_codex(path),
        Family::DeepSeek => scan_deepseek(path),
        _ => Scan::default(),
    };
    if let Ok(mut cache) = scan_cache().lock() {
        cache.insert(
            path.to_path_buf(),
            CachedScan {
                mtime,
                size,
                scan: scan.clone(),
            },
        );
    }
    scan
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .unwrap_or(false)
}

fn scan_drizzle(path: &Path, cutoff: i64) -> Scan {
    if !path.is_file() {
        return Scan::default();
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let has_v2_sessions = table_exists(&conn, "session_v2");
    let has_v2_messages = table_exists(&conn, "session_message");
    let mut sources = Vec::new();
    for table in ["session_message", "message"] {
        if !table_exists(&conn, table) {
            continue;
        }
        let mut filter = String::new();
        if table == "message" {
            if has_v2_sessions {
                filter.push_str(" AND NOT EXISTS(SELECT 1 FROM session_v2 WHERE id=s.id)");
            }
            if has_v2_messages {
                filter.push_str(
                    " AND NOT EXISTS(SELECT 1 FROM session_message WHERE session_id=s.id)",
                );
            }
        }
        let boundary = drizzle_usage_boundary(&conn, table);
        let legacy = i32::from(table == "message");
        sources.push(format!("SELECT m.time_created,m.data,{legacy} FROM {table} m JOIN sessions s ON s.id=m.session_id WHERE s.time_archived IS NULL AND (m.time_created >= ?1 OR m.time_updated >= ?1){filter}{boundary}"));
    }
    if sources.is_empty() {
        return Scan::default();
    }
    let query = format!(
        "WITH sessions AS ({}) {}",
        drizzle_sessions_sql(&conn),
        sources.join(" UNION ALL ")
    );
    let Ok(mut statement) = conn.prepare(&query) else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let Ok(rows) = statement.query_map([cutoff.saturating_mul(1000)], |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
        ))
    }) else {
        return Scan {
            incomplete: true,
            ..Scan::default()
        };
    };
    let mut scan = Scan::default();
    for row in rows {
        let Ok((created, data, legacy)) = row else {
            scan.incomplete = true;
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            scan.incomplete = true;
            continue;
        };
        let Some(tokens) = value.get("tokens") else {
            continue;
        };
        let usage = TokenUsage::read(tokens, "input", "output");
        // Legacy assistant messages start with default zeros before finish-step.
        if legacy && is_unmeasured_drizzle_zero(&value, usage) {
            scan.incomplete = true;
            continue;
        }
        let ts = created
            .filter(|created| *created > 0)
            .map(|created| created / 1000);
        if let Some(entry) = entry(ts, usage) {
            scan.samples.push(Sample { id: None, entry });
        } else {
            scan.incomplete = true;
        }
    }
    scan
}

fn hermes_has_usage(path: &Path) -> bool {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return path.is_file();
    };
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE archived=0 AND message_count>0)",
        [],
        |row| row.get(0),
    )
    .unwrap_or(true)
}

pub fn daily_tokens(family: Option<Family>) -> DailyTokenResponse {
    let Some(home) = dirs::home_dir() else {
        return DailyTokenResponse {
            entries: Vec::new(),
            unavailable_families: Vec::new(),
        };
    };
    daily_tokens_at(&home, family, SystemTime::now())
}

fn daily_tokens_at(home: &Path, selected: Option<Family>, now: SystemTime) -> DailyTokenResponse {
    let cutoff_time = now
        .checked_sub(Duration::from_secs(32 * 86_400))
        .unwrap_or(UNIX_EPOCH);
    let cutoff = cutoff_time
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let mut response = DailyTokenResponse {
        entries: Vec::new(),
        unavailable_families: Vec::new(),
    };
    let mut calls: HashMap<String, DailyTokenEntry> = HashMap::new();
    for family in Family::ALL {
        if selected.is_some_and(|selected| selected != family) {
            continue;
        }
        let root = family.root(home);
        let scan = match family {
            Family::Hermes => {
                if hermes_has_usage(&root.join("state.db")) {
                    response
                        .unavailable_families
                        .push(family.as_id().to_string());
                }
                continue;
            }
            Family::OpenCode => scan_drizzle(&root.join("opencode.db"), cutoff),
            Family::MiMo => scan_drizzle(&mimo_db_path(home), cutoff),
            _ => {
                let mut candidates = Vec::new();
                if family == Family::DeepSeek {
                    collect_dsh_paths(&root, family, &mut candidates);
                } else if let Some(depth) = family.jsonl_depth() {
                    let depth = if family == Family::Claude { 4 } else { depth };
                    collect_jsonl_paths(root.clone(), depth, family, &mut candidates);
                }
                candidates.retain(|(mtime, _, _)| *mtime >= cutoff_time);
                if let Ok(mut cache) = scan_cache().lock() {
                    let keep: HashSet<_> = candidates.iter().map(|(_, path, _)| path).collect();
                    cache.retain(|path, _| !path.starts_with(&root) || keep.contains(path));
                }
                let mut scan = Scan::default();
                for (_, path, _) in candidates {
                    let cached = cached_scan(&path, family);
                    scan.incomplete |= cached.incomplete;
                    scan.samples.extend(cached.samples);
                }
                scan
            }
        };
        if scan.incomplete {
            response
                .unavailable_families
                .push(family.as_id().to_string());
        }
        for sample in scan.samples {
            if sample.entry.ts < cutoff {
                continue;
            }
            if let Some(id) = sample.id {
                let key = format!("{}:{id}", family.as_id());
                let replace = calls.get(&key).map_or(true, |existing| {
                    if family == Family::Claude {
                        sample.entry.ts >= existing.ts
                    } else {
                        sample.entry.ts < existing.ts
                    }
                });
                if replace {
                    calls.insert(key, sample.entry);
                }
                continue;
            }
            response.entries.push(sample.entry);
        }
    }
    response.entries.extend(calls.into_values());
    response.entries.sort_by_key(|entry| entry.ts);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIRST_DAY: &str = "2026-10-09T23:59:58Z";
    const NEXT_DAY: &str = "2026-10-10T00:00:01Z";

    fn fixture_dir() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("echobird_daily_tokens_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_rows(path: &Path, rows: &[Value]) {
        std::fs::write(
            path,
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }

    fn counter(ts: &str, input: u64, output: u64, last_input: u64, last_output: u64) -> Value {
        json!({"type":"event_msg","timestamp":ts,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"output_tokens":output},"last_token_usage":{"input_tokens":last_input,"output_tokens":last_output}}}})
    }

    fn modern_response(
        ts: &str,
        id: &str,
        input: u64,
        output: u64,
        total_input: u64,
        total_output: u64,
    ) -> Value {
        json!({"type":"token_usage_record","timestamp":ts,"payload":{"response_id":id,"thread_id":"fixture","usage":{"input_tokens":input,"output_tokens":output},"thread_token_usage":{"input_tokens":total_input,"output_tokens":total_output}}})
    }

    fn totals(scan: &Scan) -> (u64, u64) {
        scan.samples.iter().fold((0, 0), |(input, output), sample| {
            (
                input + sample.entry.input_tokens,
                output + sample.entry.output_tokens,
            )
        })
    }

    fn codex_session_totals(path: &Path) -> (Option<u64>, Option<u64>) {
        let session = super::super::parse_codex_session_jsonl(path).unwrap();
        (session.input_tokens, session.output_tokens)
    }

    #[test]
    fn claude_daily_tokens_use_final_call_time_deduplicate_files_and_keep_zero() {
        let home = fixture_dir();
        let root = Family::Claude.root(&home).join("project");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("fixture.jsonl");
        write_rows(
            &path,
            &[
                json!({"timestamp":FIRST_DAY,"message":{"id":"call","role":"assistant","usage":{"input_tokens":10,"output_tokens":2}}}),
                json!({"timestamp":NEXT_DAY,"message":{"id":"call","role":"assistant","usage":{"input_tokens":10,"output_tokens":5}}}),
                json!({"timestamp":NEXT_DAY,"message":{"id":"zero","role":"assistant","usage":{"input_tokens":0,"output_tokens":0}}}),
                json!({"timestamp":NEXT_DAY,"message":{"id":"partial","role":"assistant","usage":{"input_tokens":20}}}),
                json!({"message":{"id":"no-time","role":"assistant","usage":{"input_tokens":20,"output_tokens":3}}}),
                json!({"timestamp":"2010-01-01T00:00:00Z","message":{"id":"old","role":"assistant","usage":{"input_tokens":1000,"output_tokens":1000}}}),
                json!({"timestamp":"not-time","message":{"id":"invalid-time","role":"assistant","usage":{"input_tokens":1000,"output_tokens":1000}}}),
            ],
        );
        write_rows(
            &root.join("copy.jsonl"),
            &[
                json!({"timestamp":NEXT_DAY,"message":{"id":"call","role":"assistant","usage":{"input_tokens":10,"output_tokens":5}}}),
            ],
        );
        let now = UNIX_EPOCH
            + Duration::from_secs(
                iso_timestamp(Some(&json!("2026-10-11T12:00:00Z"))).unwrap() as u64
            );
        let response = daily_tokens_at(&home, Some(Family::Claude), now);
        assert_eq!(response.entries.len(), 2);
        assert!(response
            .entries
            .iter()
            .all(|entry| entry.ts == iso_timestamp(Some(&json!(NEXT_DAY))).unwrap()));
        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.input_tokens)
                .sum::<u64>(),
            10
        );
        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.output_tokens)
                .sum::<u64>(),
            5
        );
        assert_eq!(response.unavailable_families, ["claude"]);
        assert!(scan_claude(&root.join("missing.jsonl")).incomplete);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn codex_modern_usage_and_legacy_counters_do_not_double_count_or_fabricate_resets() {
        let dir = fixture_dir();
        let path = dir.join("fixture.jsonl");
        write_rows(
            &path,
            &[
                json!({"type":"session_meta","payload":{"id":"fixture"}}),
                counter(FIRST_DAY, 10, 2, 10, 2),
                counter(NEXT_DAY, 10, 2, 10, 2),
                modern_response(NEXT_DAY, "response-one", 15, 5, 25, 7),
                counter(NEXT_DAY, 25, 7, 15, 5),
                modern_response(NEXT_DAY, "response-one", 15, 5, 25, 7),
                counter(NEXT_DAY, 2, 1, 2, 1),
                modern_response(NEXT_DAY, "response-after-reset", 2, 1, 27, 8),
                modern_response(NEXT_DAY, "response-three", 3, 1, 30, 9),
                counter(NEXT_DAY, 5, 2, 3, 1),
            ],
        );
        let scan = scan_codex(&path);
        assert_eq!(scan.samples.len(), 4);
        assert_eq!(totals(&scan), (30, 9));
        assert_eq!(codex_session_totals(&path), (Some(30), Some(9)));
        assert!(!scan.incomplete);
        assert_eq!(
            scan.samples
                .iter()
                .filter(|sample| sample.entry.ts == iso_timestamp(Some(&json!(FIRST_DAY))).unwrap())
                .count(),
            1
        );
        // A headless rate-limit snapshot may carry the previous call's last usage.
        write_rows(
            &path,
            &[
                counter(FIRST_DAY, 100, 20, 10, 2),
                counter(NEXT_DAY, 110, 22, 10, 2),
            ],
        );
        let scan = scan_codex(&path);
        assert_eq!(totals(&scan), (10, 2));
        assert_eq!(scan.samples.len(), 1);
        assert!(scan.incomplete);
        assert_eq!(codex_session_totals(&path), (Some(110), Some(22)));
        write_rows(&path, &[counter(NEXT_DAY, 0, 0, 0, 0)]);
        let zero = scan_codex(&path);
        assert_eq!(zero.samples.len(), 1);
        assert_eq!(totals(&zero), (0, 0));
        assert_eq!(codex_session_totals(&path), (Some(0), Some(0)));
        // A new counter epoch with last == total retains its measured call.
        write_rows(
            &path,
            &[
                counter(FIRST_DAY, 10, 2, 10, 2),
                counter(NEXT_DAY, 2, 1, 2, 1),
                counter(NEXT_DAY, 5, 2, 3, 1),
            ],
        );
        assert_eq!(totals(&scan_codex(&path)), (15, 4));
        assert_eq!(codex_session_totals(&path), (Some(15), Some(4)));
        // Unknown decreases only establish the next baseline.
        write_rows(
            &path,
            &[
                counter(FIRST_DAY, 10, 2, 10, 2),
                counter(NEXT_DAY, 5, 1, 2, 1),
                counter(NEXT_DAY, 8, 2, 3, 1),
            ],
        );
        assert_eq!(totals(&scan_codex(&path)), (13, 3));
        assert_eq!(codex_session_totals(&path), (Some(13), Some(3)));
        // Rollback replays an earlier counter; it is not a second API call.
        write_rows(
            &path,
            &[
                counter(FIRST_DAY, 10, 2, 10, 2),
                json!({"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}),
                counter(NEXT_DAY, 2, 1, 2, 1),
                counter(NEXT_DAY, 5, 2, 3, 1),
            ],
        );
        assert_eq!(totals(&scan_codex(&path)), (13, 3));
        assert_eq!(codex_session_totals(&path), (Some(13), Some(3)));
        let mut full_context = counter(NEXT_DAY, 0, 0, 0, 0);
        full_context["payload"]["info"]["total_token_usage"]["total_tokens"] = json!(100);
        write_rows(
            &path,
            &[full_context.clone(), counter(NEXT_DAY, 2, 1, 2, 1)],
        );
        let scan = scan_codex(&path);
        assert_eq!(scan.samples.len(), 1);
        assert_eq!(totals(&scan), (2, 1));
        write_rows(
            &path,
            &[
                counter(FIRST_DAY, 10, 2, 10, 2),
                full_context,
                counter(NEXT_DAY, 10, 2, 10, 2),
            ],
        );
        assert_eq!(totals(&scan_codex(&path)), (20, 4));
        assert_eq!(codex_session_totals(&path), (Some(20), Some(4)));
        write_rows(
            &path,
            &[
                counter(FIRST_DAY, 10, 2, 10, 2),
                json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"output_tokens":0,"total_tokens":100}}}}),
            ],
        );
        assert_eq!(codex_session_totals(&path), (Some(10), Some(2)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn codex_session_keeps_undated_and_partial_known_usage_without_fabricating_daily_values() {
        let dir = fixture_dir();
        let path = dir.join("fixture.jsonl");
        let mut undated = modern_response(NEXT_DAY, "undated", 10, 2, 10, 2);
        undated.as_object_mut().unwrap().remove("timestamp");
        let mut partial = modern_response(NEXT_DAY, "partial", 3, 1, 13, 3);
        partial["payload"]["usage"]
            .as_object_mut()
            .unwrap()
            .remove("output_tokens");
        write_rows(
            &path,
            &[
                json!({"type":"session_meta","payload":{"id":"fixture"}}),
                undated.clone(),
                undated,
                counter(NEXT_DAY, 10, 2, 10, 2),
                partial,
                modern_response(NEXT_DAY, "zero", 0, 0, 13, 3),
            ],
        );
        assert_eq!(codex_session_totals(&path), (Some(13), Some(2)));
        let scan = scan_codex(&path);
        assert_eq!(scan.samples.len(), 1);
        assert_eq!(totals(&scan), (0, 0));
        assert!(scan.incomplete);
        let input_only = json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10}}}});
        write_rows(&path, std::slice::from_ref(&input_only));
        assert_eq!(codex_session_totals(&path), (Some(10), None));
        write_rows(
            &path,
            &[
                input_only.clone(),
                input_only,
                counter(NEXT_DAY, 15, 2, 5, 2),
                counter(NEXT_DAY, 18, 3, 3, 1),
            ],
        );
        assert_eq!(codex_session_totals(&path), (Some(18), Some(3)));
        let scan = scan_codex(&path);
        assert_eq!(totals(&scan), (3, 1));
        assert_eq!(scan.samples.len(), 1);
        assert!(scan.incomplete);
        write_rows(
            &path,
            &[
                json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10}}}}),
                counter(NEXT_DAY, 2, 1, 2, 1),
            ],
        );
        assert_eq!(codex_session_totals(&path), (Some(12), Some(1)));
        assert_eq!(totals(&scan_codex(&path)), (2, 1));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn claude_nested_subagents_count_own_usage_once() {
        let home = fixture_dir();
        let root = Family::Claude.root(&home).join("project");
        let agents = root.join("session").join("subagents");
        std::fs::create_dir_all(&agents).unwrap();
        let parent_call = json!({"timestamp":NEXT_DAY,"message":{"id":"parent","role":"assistant","usage":{"input_tokens":10,"output_tokens":2}}});
        write_rows(
            &root.join("session.jsonl"),
            std::slice::from_ref(&parent_call),
        );
        write_rows(
            &agents.join("agent.jsonl"),
            &[
                parent_call,
                json!({"timestamp":NEXT_DAY,"message":{"id":"child","role":"assistant","usage":{"input_tokens":20,"output_tokens":3}}}),
            ],
        );
        let now = UNIX_EPOCH
            + Duration::from_secs(
                iso_timestamp(Some(&json!("2026-10-11T12:00:00Z"))).unwrap() as u64
            );
        let response = daily_tokens_at(&home, Some(Family::Claude), now);
        assert_eq!(response.entries.len(), 2);
        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.input_tokens)
                .sum::<u64>(),
            30
        );
        assert_eq!(
            response
                .entries
                .iter()
                .map(|entry| entry.output_tokens)
                .sum::<u64>(),
            5
        );
        assert!(response.unavailable_families.is_empty());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn codex_forks_ignore_inherited_usage_and_later_parent_metadata() {
        let dir = fixture_dir();
        let path = dir.join("fork.jsonl");
        let mut inherited = counter(NEXT_DAY, 10, 2, 10, 2);
        inherited["ordinal"] = json!(1);
        let mut own_legacy = counter(NEXT_DAY, 12, 3, 2, 1);
        own_legacy["ordinal"] = json!(5);
        let mut own = counter(NEXT_DAY, 27, 8, 15, 5);
        own["ordinal"] = json!(7);
        let mut own_record = modern_response(NEXT_DAY, "own-response", 15, 5, 27, 8);
        own_record["ordinal"] = json!(6);
        let mut parent_record = own_record.clone();
        parent_record["payload"]["thread_id"] = json!("parent");
        parent_record["payload"]["response_id"] = json!("inherited-response");
        parent_record["ordinal"] = json!(2);
        write_rows(
            &path,
            &[
                json!({"type":"session_meta","ordinal":0,"payload":{"id":"fixture","forked_from_id":"parent","subagent_history_start_ordinal":5}}),
                inherited.clone(),
                parent_record.clone(),
                json!({"type":"session_meta","ordinal":3,"payload":{"id":"parent"}}),
                own_legacy,
                own_record.clone(),
                own.clone(),
            ],
        );
        let scan = scan_codex(&path);
        assert_eq!(totals(&scan), (17, 6));
        assert_eq!(scan.samples.len(), 2);
        assert_eq!(codex_session_totals(&path), (Some(17), Some(6)));
        assert_eq!(
            super::super::parse_codex_session_jsonl(&path)
                .unwrap()
                .session_token
                .as_deref(),
            Some("fixture")
        );
        // Without an ordinal boundary modern response ownership remains known;
        // inherited cumulative snapshots cannot safely be dated as new usage.
        write_rows(
            &path,
            &[
                json!({"type":"session_meta","payload":{"id":"fixture","forked_from_id":"parent"}}),
                inherited,
                parent_record,
                own,
                own_record,
            ],
        );
        let scan = scan_codex(&path);
        assert_eq!(totals(&scan), (15, 5));
        assert_eq!(codex_session_totals(&path), (Some(15), Some(5)));
        assert!(scan.incomplete);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deepseek_daily_usage_replaces_cross_day_chunk_and_rejects_missing_times() {
        let dir = fixture_dir();
        let path = dir.join("session.jsonl");
        let first = iso_timestamp(Some(&json!(FIRST_DAY))).unwrap() * 1000;
        let next = iso_timestamp(Some(&json!(NEXT_DAY))).unwrap() * 1000;
        write_rows(
            &path,
            &[
                json!({"type":"session","id":"fixture"}),
                json!({"type":"assistant/chunk","time":first,"data":{"turn":0,"step":0,"chunk":{"type":"usage","usage":{"inputTokens":10,"outputTokens":2}}}}),
                json!({"type":"assistant/message","time":next,"data":{"turn":0,"step":0,"usage":{"inputTokens":10,"outputTokens":5}}}),
                json!({"type":"assistant/message","time":next,"data":{"turn":0,"step":1,"usage":{"inputTokens":0,"outputTokens":0}}}),
                json!({"type":"assistant/message","data":{"turn":0,"step":2,"usage":{"inputTokens":99,"outputTokens":5}}}),
            ],
        );
        let scan = scan_deepseek(&path);
        assert_eq!(scan.samples.len(), 2);
        assert_eq!(totals(&scan), (10, 5));
        assert!(scan
            .samples
            .iter()
            .all(|sample| sample.entry.ts == next / 1000));
        assert!(scan.incomplete);
        let session = super::super::parse_dsh_session(&path).unwrap();
        assert_eq!(
            (session.input_tokens, session.output_tokens),
            (Some(109), Some(10))
        );
        let compressed = dir.join("session.jsonl.zstd");
        std::fs::write(
            &compressed,
            zstd::stream::encode_all(std::fs::read(&path).unwrap().as_slice(), 1).unwrap(),
        )
        .unwrap();
        assert_eq!(totals(&scan_deepseek(&compressed)), (10, 5));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deepseek_fork_uses_event_sequence_to_exclude_seeded_usage() {
        let dir = fixture_dir();
        let parent = dir.join("parent.jsonl");
        let fork = dir.join("fork.jsonl");
        let timestamp = iso_timestamp(Some(&json!(NEXT_DAY))).unwrap() * 1000;
        let inherited = json!({"type":"assistant/message","seq":2,"time":timestamp,"data":{"turn":0,"step":0,"usage":{"inputTokens":10,"outputTokens":2}}});
        let mut own = json!({"type":"assistant/message","seq":7,"time":timestamp,"data":{"turn":1,"step":0,"usage":{"inputTokens":20,"outputTokens":3}}});
        write_rows(
            &parent,
            &[json!({"type":"session","id":"parent"}), inherited.clone()],
        );
        write_rows(
            &fork,
            &[
                json!({"type":"session","id":"fork","parentSession":"parent","seedLength":7}),
                json!({"type":"text-chunks","seq":0,"data":{}}),
                inherited.clone(),
                own.clone(),
            ],
        );
        assert_eq!(totals(&scan_deepseek(&parent)), (10, 2));
        let scan = scan_deepseek(&fork);
        assert_eq!(totals(&scan), (20, 3));
        assert!(!scan.incomplete);
        let session = super::super::parse_dsh_session(&fork).unwrap();
        assert_eq!(
            (session.input_tokens, session.output_tokens),
            (Some(20), Some(3))
        );
        write_rows(
            &fork,
            &[
                json!({"type":"session","id":"fork","parentSession":"parent","origin":"subagent"}),
                own.clone(),
            ],
        );
        let scan = scan_deepseek(&fork);
        assert_eq!(totals(&scan), (20, 3));
        assert!(!scan.incomplete);
        let session = super::super::parse_dsh_session(&fork).unwrap();
        assert_eq!(
            (session.input_tokens, session.output_tokens),
            (Some(20), Some(3))
        );
        own.as_object_mut().unwrap().remove("seq");
        write_rows(
            &fork,
            &[
                json!({"type":"session","id":"fork","parentSession":"parent","seedLength":7}),
                inherited,
                own,
            ],
        );
        let scan = scan_deepseek(&fork);
        assert!(scan.samples.is_empty());
        assert!(scan.incomplete);
        let session = super::super::parse_dsh_session(&fork).unwrap();
        assert_eq!((session.input_tokens, session.output_tokens), (None, None));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sqlite_daily_usage_respects_migration_fork_boundaries_and_missing_metadata() {
        let dir = fixture_dir();
        let path = dir.join("fixture.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2 (id TEXT PRIMARY KEY,title TEXT,directory TEXT,time_created INTEGER,time_updated INTEGER,time_archived INTEGER,parent_id TEXT);
             CREATE TABLE session AS SELECT * FROM session_v2;
             CREATE TABLE session_message (id TEXT PRIMARY KEY,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);
             CREATE TABLE message (id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);"
        ).unwrap();
        let first = iso_timestamp(Some(&json!(FIRST_DAY))).unwrap() * 1000;
        let next = iso_timestamp(Some(&json!(NEXT_DAY))).unwrap() * 1000;
        for (table, id, created) in [
            ("session_v2", "modern", first),
            ("session_v2", "empty", first),
            ("session", "modern", first),
            ("session", "empty", first),
            ("session", "legacy", first),
            ("session", "fork", next),
        ] {
            conn.execute(
                &format!("INSERT INTO {table} VALUES (?1,'Fixture','/fixture',?2,?2,NULL,NULL)"),
                rusqlite::params![id, created],
            )
            .unwrap();
        }
        for (id, session, created, input, output) in [
            ("old-modern", "modern", first, 9000, 9000),
            ("old-empty", "empty", first, 8000, 8000),
            ("legacy", "legacy", first, 50, 8),
            ("inherited", "fork", first, 7000, 7000),
            ("own", "fork", next, 3, 1),
        ] {
            conn.execute(
                "INSERT INTO message VALUES (?1,?2,?3,?3,?4)",
                rusqlite::params![
                    id,
                    session,
                    created,
                    json!({"tokens":{"input":input,"output":output}}).to_string()
                ],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO session_message VALUES ('modern','modern','assistant',?1,?1,?2)",
            rusqlite::params![next, json!({"tokens":{"input":10,"output":2}}).to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message VALUES ('inherited-modern','modern','assistant',?1,?2,?3)",
            rusqlite::params![
                first - 1000,
                next,
                json!({"tokens":{"input":9000,"output":9000}}).to_string()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message VALUES ('missing-time','modern','assistant',NULL,?1,?2)",
            rusqlite::params![next, json!({"tokens":{"input":99,"output":99}}).to_string()],
        )
        .unwrap();
        for (id, data) in [
            (
                "pending-zero",
                json!({"role":"assistant","tokens":{"input":0,"output":0}}),
            ),
            (
                "error-zero",
                json!({"role":"assistant","error":{"name":"APIError"},"time":{"completed":next},"tokens":{"input":0,"output":0}}),
            ),
            (
                "measured-zero",
                json!({"role":"assistant","finish":"stop","time":{"completed":next},"tokens":{"input":0,"output":0}}),
            ),
        ] {
            conn.execute(
                "INSERT INTO message VALUES (?1,'legacy',?2,?2,?3)",
                rusqlite::params![id, next, data.to_string()],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO session_message VALUES ('modern-zero','modern','assistant',?1,?1,?2)",
            rusqlite::params![
                next,
                json!({"type":"assistant","tokens":{"input":0,"output":0}}).to_string()
            ],
        )
        .unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        let scan = scan_drizzle(&path, first / 1000 - 86400);
        assert_eq!(totals(&scan), (63, 11));
        assert_eq!(scan.samples.len(), 5);
        assert!(scan.incomplete);
        let rows = super::super::drizzle_history_page(&path, "opencode", "OpenCode", 0, 20);
        let tokens = |id: &str| {
            let session = rows
                .iter()
                .find(|row| row.session_token.as_deref() == Some(id))
                .unwrap();
            (session.input_tokens, session.output_tokens)
        };
        assert_eq!(tokens("fork"), (Some(3), Some(1)));
        assert_eq!(tokens("modern"), (Some(109), Some(101)));
        assert_eq!(tokens("empty"), (None, None));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.join("fixture.db-wal").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn daily_cache_invalidates_by_precise_time_and_size_without_storing_messages() {
        let dir = fixture_dir();
        let path = dir.join("fixture.jsonl");
        write_rows(
            &path,
            &[
                json!({"timestamp":NEXT_DAY,"message":{"id":"call","role":"assistant","usage":{"input_tokens":10,"output_tokens":2}}}),
            ],
        );
        let stamp = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        assert_eq!(totals(&cached_scan(&path, Family::Claude)), (10, 2));
        scan_cache()
            .lock()
            .unwrap()
            .get_mut(&path)
            .unwrap()
            .scan
            .samples[0]
            .entry
            .input_tokens = 777;
        assert_eq!(totals(&cached_scan(&path, Family::Claude)), (777, 2));
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, original.replace(":10", ":11")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp + Duration::from_millis(1))
            .unwrap();
        assert_eq!(totals(&cached_scan(&path, Family::Claude)), (11, 2));
        scan_cache()
            .lock()
            .unwrap()
            .get_mut(&path)
            .unwrap()
            .scan
            .samples[0]
            .entry
            .input_tokens = 777;
        std::fs::write(&path, format!("{original}\n")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp + Duration::from_millis(1))
            .unwrap();
        assert_eq!(totals(&cached_scan(&path, Family::Claude)), (10, 2));
        assert!(cached_scan(&dir.join("missing.jsonl"), Family::Claude).incomplete);
        let hermes = dir.join("state.db");
        let conn = rusqlite::Connection::open(&hermes).unwrap();
        conn.execute_batch("CREATE TABLE sessions(archived INTEGER,message_count INTEGER); INSERT INTO sessions VALUES (0,0);").unwrap();
        assert!(!hermes_has_usage(&hermes));
        conn.execute("UPDATE sessions SET message_count=1", [])
            .unwrap();
        assert!(hermes_has_usage(&hermes));
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
