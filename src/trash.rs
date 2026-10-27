//! Dashboard trash. It never deletes directly: it moves items to a folder of its own
//! and records every move.
//!
//! Only paths inside $HOME are touched. Emptying the trash does delete for real,
//! and the server exposes it as a separate endpoint that the interface only calls
//! after an explicit confirmation.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::fsutil::{dir_stats, human_size};

const AUDIT_FILE: &str = "audit.log";

fn audit(trash_dir: &Path, line: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(trash_dir.join(AUDIT_FILE)) {
        let _ = writeln!(file, "{stamp}\t{line}");
    }
}

fn is_inside(home: &Path, path: &Path) -> bool {
    path == home || path.starts_with(home)
}

/// Moves `path` to the trash. Returns {ok, moved_to} or {ok:false, error}.
pub fn move_to_trash(home: &Path, trash_dir: &Path, path_str: &str) -> Value {
    let path = PathBuf::from(path_str);
    if !path.exists() {
        return json!({"ok": false, "error": "The path no longer exists"});
    }
    if !is_inside(home, &path) || path == home {
        return json!({"ok": false, "error": "Path outside $HOME, rejected"});
    }
    if fs::create_dir_all(trash_dir).is_err() {
        return json!({"ok": false, "error": "Could not create the trash"});
    }

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut dest = trash_dir.join(format!("{stamp}__{name}"));
    let mut counter = 1;
    while dest.exists() {
        dest = trash_dir.join(format!("{stamp}__{name}__{counter}"));
        counter += 1;
    }

    if let Err(err) = fs::rename(&path, &dest) {
        // rename does not work across different disks: it falls back to copy and delete.
        if let Err(copy_err) = copy_then_remove(&path, &dest) {
            return json!({"ok": false, "error": format!("{err}; {copy_err}")});
        }
    }
    audit(trash_dir, &format!("MOVED\t{}\t->\t{}", path.display(), dest.display()));
    json!({"ok": true, "moved_to": dest.to_string_lossy()})
}

fn copy_then_remove(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        copy_dir(from, to)?;
        fs::remove_dir_all(from)
    } else {
        fs::copy(from, to)?;
        fs::remove_file(from)
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)?.flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Size and number of files in the trash: what still takes up disk space.
pub fn trash_info(trash_dir: &Path) -> Value {
    if !trash_dir.is_dir() {
        return json!({"size_bytes": 0, "size_human": "0B", "item_count": 0});
    }
    let (size, count, _) = dir_stats(trash_dir, &[]);
    json!({"size_bytes": size, "size_human": human_size(size), "item_count": count})
}

/// Really deletes everything in the trash, except the log.
pub fn empty_trash(trash_dir: &Path) -> Value {
    if !trash_dir.is_dir() {
        return json!({"ok": true, "removed": 0, "freed_bytes": 0, "freed_human": "0B"});
    }
    let mut freed = 0u64;
    let mut removed = 0u64;
    let Ok(read) = fs::read_dir(trash_dir) else {
        return json!({"ok": false, "error": "Could not read the trash"});
    };
    for item in read.flatten().map(|e| e.path()) {
        if item.file_name().is_some_and(|n| n == AUDIT_FILE) {
            continue;
        }
        let (size, _, _) = if item.is_dir() { dir_stats(&item, &[]) } else { (fs::metadata(&item).map(|m| m.len()).unwrap_or(0), 0, 0.0) };
        let result = if item.is_dir() { fs::remove_dir_all(&item) } else { fs::remove_file(&item) };
        if result.is_ok() {
            freed += size;
            removed += 1;
        }
    }
    audit(trash_dir, &format!("EMPTIED\t{removed} items\t{}", human_size(freed)));
    json!({"ok": true, "removed": removed, "freed_bytes": freed, "freed_human": human_size(freed)})
}
