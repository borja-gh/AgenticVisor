//! Scan entry point: runs each adapter, then the generic discovery
//! over whatever they do not cover.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::adapters::{claude_code, claude_desktop, cursor_cli, cursor_desktop, generic};
use crate::entry::Entry;

pub struct Scan {
    pub entries: Vec<Entry>,
}

pub fn run_scan(home: &Path) -> Scan {
    let mut entries = Vec::new();
    entries.extend(claude_code::scan(home));
    entries.extend(claude_desktop::scan(home));
    entries.extend(cursor_cli::scan(home));
    entries.extend(cursor_desktop::scan(home));

    let app_support = home.join("Library").join("Application Support");
    let claimed: HashSet<String> = [
        home.join(".claude"),
        home.join(".claude.json"),
        home.join(".claude.json.backup"),
        app_support.join("Claude"),
        home.join(".cursor"),
        app_support.join("Cursor"),
    ]
    .iter()
    .map(|p| p.to_string_lossy().into_owned())
    .collect();
    entries.extend(generic::scan(home, &claimed));

    // Sort by size, largest first. Rust's sort is stable.
    entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Scan { entries }
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Response of /api/scan: the same keys plus token usage.
pub fn scan_json(home: &Path) -> Value {
    let now = now_secs();
    let scan = run_scan(home);
    let total_size: u64 = scan.entries.iter().map(|e| e.size_bytes).sum();

    let mut by_tool: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_category: BTreeMap<String, u64> = BTreeMap::new();
    for e in &scan.entries {
        *by_tool.entry(e.tool.clone()).or_default() += e.size_bytes;
        *by_category.entry(e.category.clone()).or_default() += e.size_bytes;
    }

    let entries: Vec<Value> = scan.entries.iter().map(|e| e.to_json(now)).collect();
    json!({
        "generated_at": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "entries": entries,
        "summary": {
            "total_size_bytes": total_size,
            "total_entries": scan.entries.len(),
            "by_tool": by_tool,
            "by_category": by_category,
        }
    })
}
