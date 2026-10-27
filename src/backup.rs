//! Backups before each write and change log.
//! Backup and change-log helpers.
//!
//! Backups and the log live in the dashboard's data folder
//! (`<data>/backups/` and `<data>/changes.log`), never inside the project.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Copies `source` to `backup_dir` with the date in its name. Returns the path of the copy.
pub fn backup_file(source: &Path, backup_dir: &Path, label: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(backup_dir).map_err(|e| e.to_string())?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = backup_dir.join(format!("{stamp}__{label}"));
    fs::copy(source, &dest).map_err(|e| format!("Could not create the backup: {e}"))?;
    Ok(dest)
}

/// Appends a line to the change log (`<data>/changes.log`).
pub fn log_change(backup_dir: &Path, line: &str) {
    let Some(data_dir) = backup_dir.parent() else {
        return;
    };
    let _ = fs::create_dir_all(data_dir);
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(data_dir.join("changes.log")) {
        let _ = writeln!(file, "{stamp}\t{line}");
    }
}
