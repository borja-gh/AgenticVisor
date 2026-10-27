//! Session preview: readable messages, paged.
//!
//! Only paths inside the known session folders are read. Tokens with
//! a credential shape are masked. Cursor's internal tags are
//! removed for display only: the indexed text does not change.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Value};

use crate::config::mask_secret_values;
use crate::search::sources::{iter_claude_messages, iter_cursor_bubbles, iter_cursor_chat, iter_cursor_transcript};

pub const PAGE_SIZE: usize = 40;
const MAX_MESSAGE_CHARS: usize = 4000;

fn roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".claude").join("projects"),
        home.join(".claude").join("transcripts"),
        home.join(".claude").join("plans"),
        home.join("Library").join("Application Support").join("Claude").join("local-agent-mode-sessions"),
        home.join(".cursor").join("projects"),
        home.join(".cursor").join("chats"),
        home.join(".cursor").join("plans"),
        home.join("Library").join("Application Support").join("Cursor").join("User").join("globalStorage").join("state.vscdb"),
    ]
}

fn checked(home: &Path, raw: &str) -> Result<PathBuf, String> {
    let path = std::path::absolute(raw).map_err(|e| e.to_string())?;
    let inside = roots(home).iter().any(|root| path == *root || path.starts_with(root));
    if !inside {
        return Err("That path is not a known session folder".into());
    }
    if !path.exists() {
        return Err("No longer exists".into());
    }
    Ok(path)
}

fn is_cursor(home: &Path, path: &Path) -> bool {
    path.strip_prefix(home).ok().and_then(|rel| rel.components().next()).is_some_and(|c| c.as_os_str() == ".cursor")
}

fn read_plan(path: &Path) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(&fs::read(path).unwrap_or_default()).into_owned();
    vec![("assistant".to_string(), text)]
}

fn is_chat_id(text: &str) -> bool {
    let re = Regex::new(r"^[0-9a-fA-F-]{36}$").expect("valid regex");
    re.is_match(text)
}

/// Label and messages for a session key.
fn source(home: &Path, key: &str) -> Result<(String, Vec<(String, String)>), String> {
    if let Some((raw_db, chat_id)) = key.split_once('#') {
        let db = checked(home, raw_db)?;
        if !is_chat_id(chat_id) {
            return Err("Invalid chat identifier".into());
        }
        return Ok(("Cursor Desktop · chat".into(), iter_cursor_bubbles(&db, chat_id)));
    }

    let path = checked(home, key)?;
    let tool = if is_cursor(home, &path) { "Cursor" } else { "Claude" };

    if path.is_dir() {
        let parent = path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let grandparent = path.parent().and_then(Path::parent).and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if parent == "projects" && (grandparent == ".cursor" || grandparent == ".claude") {
            return Err("This is a project folder: pick a specific conversation".into());
        }
        let store = path.join("store.db");
        if store.is_file() {
            return Ok((format!("{tool} · chat"), iter_cursor_chat(&store)));
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let mut files = Vec::new();
        collect_jsonl(&path, &mut files);
        files.sort_by(|a, b| {
            let rank = |p: &PathBuf| {
                let n = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                !(n == format!("{name}.jsonl") || n == "audit.jsonl")
            };
            rank(a).cmp(&rank(b)).then(a.cmp(b))
        });
        if files.is_empty() {
            return Err("This folder does not contain a conversation".into());
        }
        let mut messages = Vec::new();
        for file in &files {
            messages.extend(read_file(home, file));
        }
        return Ok((format!("{tool} · conversation"), messages));
    }

    match path.extension().and_then(|e| e.to_str()) {
        Some("jsonl") => Ok((format!("{tool} · jsonl"), read_file(home, &path))),
        Some("md") => Ok((format!("{tool} · plan"), read_plan(&path))),
        _ => Err("This item is not a conversation".into()),
    }
}

fn read_file(home: &Path, file: &Path) -> Vec<(String, String)> {
    if is_cursor(home, file) {
        iter_cursor_transcript(file)
    } else {
        iter_claude_messages(file)
    }
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => collect_jsonl(&path, out),
            Ok(t) if t.is_file() && path.extension().is_some_and(|e| e == "jsonl") => out.push(path),
            _ => {}
        }
    }
}

/// Removes Cursor's internal tags that do not help read the message.
fn clean_for_reading(text: &str) -> String {
    let timestamp = Regex::new(r"(?s)<timestamp>.*?</timestamp>\s*").expect("valid regex");
    let query_tag = Regex::new(r"</?user_query>").expect("valid regex");
    query_tag.replace_all(&timestamp.replace_all(text, ""), "").trim().to_string()
}

/// Page of messages of a session. `offset` is the first raw message to show.
pub fn preview(home: &Path, key: &str, offset: &str, limit: usize) -> Result<Value, String> {
    let offset: usize = if offset.chars().all(|c| c.is_ascii_digit()) && !offset.is_empty() {
        offset.parse().unwrap_or(0)
    } else {
        0
    };
    let (label, messages) = source(home, key)?;
    let window: Vec<(String, String)> = messages.into_iter().skip(offset).take(limit + 1).collect();
    let has_more = window.len() > limit;

    let mut shown = Vec::new();
    for (kind, raw) in window.into_iter().take(limit) {
        let text = clean_for_reading(&raw);
        if text.trim().is_empty() {
            continue;
        }
        let truncated = text.chars().count() > MAX_MESSAGE_CHARS;
        let clipped: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
        shown.push(json!({
            "kind": kind,
            "text": mask_secret_values(&clipped),
            "truncated": truncated,
        }));
    }
        Ok(json!({
        "label": label,
        "key": key,
        "offset": offset,
        "limit": limit,
        "has_more": has_more,
        "messages": shown,
    }))
}
