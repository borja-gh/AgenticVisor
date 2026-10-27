//! Generic discovery: any
//! folder in $HOME or Application Support with an AI keyword appears
//! in the inventory, even without a dedicated adapter.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::entry::Entry;
use crate::fsutil::{classify_by_name, dir_stats};

const KEYWORDS: &[&str] = &[
    "claude", "cursor", "anthropic", "gemini", "copilot", "codeium", "windsurf", "continue", "cline",
    "aider", "opencode", "zed", "ollama", "lmstudio", "sourcegraph", "cody", "tabnine", "antigravity",
];

fn matches(name: &str) -> bool {
    let lname = name.to_lowercase();
    KEYWORDS.iter().any(|kw| lname.contains(kw))
}

/// `claimed_roots` are the paths already covered by a specific adapter; they are
/// excluded so the same thing is not listed twice with less detail.
pub fn scan(home: &Path, claimed_roots: &HashSet<String>) -> Vec<Entry> {
    let mut out = Vec::new();
    // In $HOME only hidden folders (~/.something); in Application Support, all of them.
    let app_support = home.join("Library").join("Application Support");
    let mut candidates: Vec<_> = fs::read_dir(home)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                .collect()
        })
        .unwrap_or_default();
    if let Ok(rd) = fs::read_dir(&app_support) {
        candidates.extend(rd.flatten().map(|e| e.path()));
    }

    for child in candidates {
        let name = child.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if claimed_roots.contains(&child.to_string_lossy().into_owned()) || !child.is_dir() {
            continue;
        }
        if !matches(&name) {
            continue;
        }
        let (category, sensitive) = classify_by_name(&name);
        let (size, count, mtime) = dir_stats(&child, &[]);
        out.push(
            Entry::new(&child.to_string_lossy(), &format!("unknown:{name}"), category, &child.to_string_lossy())
                .with_stats(size, count, mtime)
                .sensitive(sensitive)
                .note("Detected automatically; no dedicated adapter yet"),
        );
    }
    out
}
