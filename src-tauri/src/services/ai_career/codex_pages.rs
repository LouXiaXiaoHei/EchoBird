use super::{Family, PathBuf, SystemTime};
use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;

pub(super) type PageStamps = Vec<(PathBuf, super::persistent_cache::FileStamp)>;

pub(super) fn is_continuation(path: &Path) -> bool {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let Some((prefix, page)) = stem.rsplit_once('_') else {
        return false;
    };
    prefix
        .get(prefix.len().saturating_sub(36)..)
        .is_some_and(|thread| uuid::Uuid::parse_str(thread).is_ok())
        && uuid::Uuid::parse_str(page).is_ok()
}

pub(super) fn page_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if is_continuation(path) {
        return stem.rsplit_once('_').map(|(_, page)| page.to_string());
    }
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    uuid::Uuid::parse_str(id).ok().map(|id| id.to_string())
}

pub(super) fn stamps(paths: &[PathBuf]) -> Result<Option<PageStamps>, String> {
    let mut stamps = Vec::new();
    for path in paths {
        let Some(metadata) = super::path_metadata(path).map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        stamps.push((
            path.clone(),
            super::persistent_cache::FileStamp::read(&metadata)
                .map_err(|error| error.to_string())?,
        ));
    }
    Ok(Some(stamps))
}

fn thread(path: &Path) -> Result<String, String> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let first = stem.split('_').next().unwrap_or(&stem);
    if let Some(id) = first.get(first.len().saturating_sub(36)..) {
        if let Ok(id) = uuid::Uuid::parse_str(id) {
            return Ok(id.to_string());
        }
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    for line in std::io::BufReader::new(file).lines().take(8) {
        let line = line.map_err(|error| error.to_string())?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if value["type"] == "session_meta" {
                if let Some(id) = value["payload"]["id"].as_str().filter(|id| !id.is_empty()) {
                    return Ok(id.to_string());
                }
            }
        }
    }
    Ok(path.to_string_lossy().into_owned())
}

pub(super) fn groups(
    candidates: &[(SystemTime, PathBuf, Family)],
) -> Result<Vec<Vec<PathBuf>>, String> {
    let mut groups: HashMap<String, (SystemTime, Vec<PathBuf>)> = HashMap::new();
    for (mtime, path, _) in candidates {
        let entry = groups.entry(thread(path)?).or_insert((*mtime, Vec::new()));
        entry.0 = entry.0.max(*mtime);
        entry.1.push(path.clone());
    }
    let mut groups: Vec<_> = groups.into_values().collect();
    for (_, paths) in &mut groups {
        paths.sort();
    }
    groups.sort_by(|(a, paths_a), (b, paths_b)| b.cmp(a).then_with(|| paths_a.cmp(paths_b)));
    Ok(groups.into_iter().map(|(_, paths)| paths).collect())
}
