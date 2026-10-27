//! Immediate contents (one level) of a folder, for each row's dropdown.
//! Only inside $HOME.

use std::fs;
use std::path::Path;

use chrono::{Local, TimeZone};
use serde_json::{json, Value};

use crate::fsutil::{dir_stats, human_size, mtime_secs};

pub fn list_children(home: &Path, path_str: &str) -> Value {
    if path_str.is_empty() {
        return json!({"error": "missing path parameter"});
    }
    let path = Path::new(path_str);
    if !(path == home || path.starts_with(home)) {
        return json!({"error": "Path outside $HOME, rejected"});
    }
    if !path.is_dir() {
        return json!({"children": []});
    }
    let Ok(read) = fs::read_dir(path) else {
        return json!({"children": []});
    };

    let now = chrono::Local::now().timestamp() as f64;
    let mut children: Vec<(u64, Value)> = Vec::new();
    for entry in read.flatten() {
        let child = entry.path();
        // is_dir follows symlinks; a file's size does not.
        let is_dir = child.is_dir();
        let (size, count, mtime) = if is_dir {
            dir_stats(&child, &[])
        } else {
            match fs::symlink_metadata(&child) {
                Ok(meta) => (meta.len(), 1, mtime_secs(&meta)),
                Err(_) => continue,
            }
        };
        let iso = if mtime != 0.0 {
            Local
                .timestamp_opt(mtime as i64, 0)
                .single()
                .map(|dt| dt.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "—".to_string())
        } else {
            "—".to_string()
        };
        let age_days = if mtime != 0.0 { json!(((now - mtime) / 86400.0) as i64) } else { Value::Null };
        let child_str = child.to_string_lossy().into_owned();
        children.push((
            size,
            json!({
                "id": child_str,
                "path": child_str,
                "name": entry.file_name().to_string_lossy(),
                "is_dir": is_dir,
                "size_bytes": size,
                "size_human": human_size(size),
                "item_count": count,
                "last_modified_iso": iso,
                "age_days": age_days,
            }),
        ));
    }
    children.sort_by(|a, b| b.0.cmp(&a.0));
    json!({"children": children.into_iter().map(|(_, v)| v).collect::<Vec<_>>()})
}
