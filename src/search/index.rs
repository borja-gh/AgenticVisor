//! Local text index: SQLite with FTS5 trigrams, to find any
//! substring of the sessions.
//!
//! It is incremental: a session is re-indexed only if its version changed.
//! It contains everything written in the sessions, including secrets pasted into a chat;
//! the file is created with 0600 permissions and lives in the dashboard's data folder.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};

use super::sources::{SessionSource, EXTRACTOR_VERSION};

pub const MAX_MESSAGE_CHARS: usize = 50_000;
pub const TOOL_OUTPUT_CAP_CHARS: usize = 5_000;
pub const TOOL_OUTPUT_MODES: &[&str] = &["none", "capped", "full"];
pub const DEFAULT_TOOL_MODE: &str = "none";
const MAX_MATCH_ROWS: i64 = 2000;
pub const MIN_QUERY_CHARS: usize = 3;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    key TEXT PRIMARY KEY,
    tool TEXT NOT NULL,
    path TEXT NOT NULL,
    title TEXT NOT NULL,
    version TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    session_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_session ON messages(session_key);
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    text, content='messages', content_rowid='id', tokenize='trigram'
);
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

#[derive(Serialize, Clone, Debug, Default)]
pub struct SyncSummary {
    pub indexed: u64,
    pub unchanged: u64,
    pub removed: u64,
    pub failed: u64,
    pub seconds: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Hit {
    pub key: String,
    pub hits: u64,
    pub kinds: Vec<String>,
    pub snippet: String,
    pub tool: String,
    pub path: String,
    pub title: String,
}

fn open(db: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = db.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if !db.exists() {
        // It is created as 0600 from the start: there is never a moment when others can read it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let _ = OpenOptions::new().create(true).write(true).mode(0o600).open(db);
        }
        #[cfg(not(unix))]
        {
            let _ = OpenOptions::new().create(true).write(true).open(db);
        }
    }
    let con = Connection::open(db)?;
    con.pragma_update(None, "journal_mode", "WAL")?;
    con.execute_batch(SCHEMA)?;
    Ok(con)
}

fn meta_value(con: &Connection, key: &str) -> Option<String> {
    con.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |row| row.get(0)).ok()
}

fn set_meta(con: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    con.execute("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)", params![key, value])?;
    Ok(())
}

/// If the extraction logic or the tool-output mode changed, the
/// whole index is emptied. The tables are recreated: deleting millions of rows
/// firing the FTS trigger one by one would be much slower.
fn reset_if_index_version_changed(con: &Connection, index_version: &str, tool_mode: &str) -> rusqlite::Result<()> {
    if meta_value(con, "index_version").as_deref() == Some(index_version) {
        return Ok(());
    }
    con.execute_batch("DROP TABLE IF EXISTS messages_fts; DROP TABLE IF EXISTS messages; DROP TABLE IF EXISTS sessions;")?;
    con.execute_batch(SCHEMA)?;
    set_meta(con, "index_version", index_version)?;
    set_meta(con, "tool_mode", tool_mode)
}

fn collect_messages(src: &SessionSource, tool_mode: &str) -> Vec<(String, String)> {
    let mut messages = vec![("title".to_string(), src.title.clone())];
    for (kind, text) in (src.read)() {
        if text.trim().is_empty() {
            continue;
        }
        let limit = if kind == "tool" {
            match tool_mode {
                "none" => continue,
                "capped" => TOOL_OUTPUT_CAP_CHARS,
                _ => MAX_MESSAGE_CHARS,
            }
        } else {
            MAX_MESSAGE_CHARS
        };
        messages.push((kind, truncate_no_ellipsis(&text, limit)));
    }
    messages
}

/// Cuts text to `limit` characters, without adding anything.
fn truncate_no_ellipsis(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn delete_session(con: &Connection, key: &str) -> rusqlite::Result<()> {
    // Deleting the messages fires the trigger that also cleans the FTS index.
    con.execute("DELETE FROM messages WHERE session_key = ?1", params![key])?;
    con.execute("DELETE FROM sessions WHERE key = ?1", params![key])?;
    Ok(())
}

fn replace_session(con: &mut Connection, src: &SessionSource, messages: &[(String, String)]) -> rusqlite::Result<()> {
    // One transaction per session: if something fails halfway, the index stays as it was.
    let tx = con.transaction()?;
    delete_session(&tx, &src.key)?;
    tx.execute(
        "INSERT INTO sessions(key, tool, path, title, version) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![src.key, src.tool, src.path, src.title, src.version],
    )?;
    {
        let mut insert = tx.prepare("INSERT INTO messages(session_key, kind, text) VALUES (?1, ?2, ?3)")?;
        for (kind, text) in messages {
            insert.execute(params![src.key, kind, text])?;
        }
    }
    tx.commit()
}

/// Brings the index up to date with `sources`. Changing `tool_mode` rebuilds everything.
pub fn sync(db: &Path, sources: Vec<SessionSource>, tool_mode: &str) -> Result<SyncSummary, String> {
    let started = Instant::now();
    let mut con = open(db).map_err(|e| e.to_string())?;
    reset_if_index_version_changed(&con, &format!("{EXTRACTOR_VERSION}:{tool_mode}"), tool_mode)
        .map_err(|e| e.to_string())?;

    let known: HashMap<String, String> = {
        let mut stmt = con.prepare("SELECT key, version FROM sessions").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        rows.flatten().collect()
    };

    let mut summary = SyncSummary::default();
    let mut seen: HashSet<String> = HashSet::new();
    for src in sources {
        seen.insert(src.key.clone());
        if known.get(&src.key) == Some(&src.version) {
            summary.unchanged += 1;
            continue;
        }
        let messages = collect_messages(&src, tool_mode);
        match replace_session(&mut con, &src, &messages) {
            Ok(()) => summary.indexed += 1,
            Err(_) => summary.failed += 1,
        }
    }

    let removed: Vec<&String> = known.keys().filter(|k| !seen.contains(*k)).collect();
    {
        let tx = con.transaction().map_err(|e| e.to_string())?;
        for key in &removed {
            delete_session(&tx, key).map_err(|e| e.to_string())?;
        }
        set_meta(&tx, "last_sync", &chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    summary.removed = removed.len() as u64;
    let _ = con.pragma_update(None, "wal_checkpoint", "TRUNCATE");
    summary.seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    Ok(summary)
}

/// Sessions containing `query`, from most to least relevant. In the snippet,
/// matches are marked with \x01 and \x02 (the interface turns them into <mark>).
pub fn search(db: &Path, query: &str, limit: usize) -> Result<Vec<Hit>, String> {
    let q = query.trim();
    if q.chars().count() < MIN_QUERY_CHARS || !db.exists() {
        return Ok(Vec::new());
    }
    let con = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| e.to_string())?;
    // Double quotes = exact phrase; quotes typed by the user are escaped.
    let match_expr = format!("\"{}\"", q.replace('"', "\"\""));
    let mut stmt = con
        .prepare(
            "SELECT m.session_key, m.kind, \
             snippet(messages_fts, 0, char(1), char(2), ' … ', 12), bm25(messages_fts) \
             FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid \
             WHERE messages_fts MATCH ?1 ORDER BY bm25(messages_fts) LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![match_expr, MAX_MATCH_ROWS], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut groups: Vec<(String, u64, Vec<String>, String)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (session_key, kind, snippet) in rows.flatten() {
        let i = *index.entry(session_key.clone()).or_insert_with(|| {
            groups.push((session_key.clone(), 0, Vec::new(), snippet.clone()));
            groups.len() - 1
        });
        groups[i].1 += 1;
        if !groups[i].2.contains(&kind) {
            groups[i].2.push(kind);
        }
    }

    let mut out = Vec::new();
    for (key, hits, mut kinds, snippet) in groups.into_iter().take(limit) {
        kinds.sort();
        let meta = con.query_row("SELECT tool, path, title FROM sessions WHERE key = ?1", params![key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        });
        if let Ok((tool, path, title)) = meta {
            out.push(Hit { key, hits, kinds, snippet, tool, path, title });
        }
    }
    Ok(out)
}

pub fn stats(db: &Path) -> Value {
    if !db.exists() {
        return json!({"sessions": 0, "messages": 0, "last_sync": null, "tool_mode": null});
    }
    let Ok(con) = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else {
        return json!({"sessions": 0, "messages": 0, "last_sync": null, "tool_mode": null});
    };
    let count = |sql: &str| con.query_row(sql, [], |row| row.get::<_, u64>(0)).unwrap_or(0);
    json!({
        "sessions": count("SELECT count(*) FROM sessions"),
        "messages": count("SELECT count(*) FROM messages"),
        "last_sync": meta_value(&con, "last_sync"),
        "tool_mode": meta_value(&con, "tool_mode"),
    })
}

/// Saved tool-output mode. An unknown or missing value falls back to the default.
pub fn read_tool_mode(config: &Path) -> String {
    let mode = fs::read_to_string(config)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("tool_outputs").and_then(Value::as_str).map(str::to_string));
    match mode {
        Some(m) if TOOL_OUTPUT_MODES.contains(&m.as_str()) => m,
        _ => DEFAULT_TOOL_MODE.to_string(),
    }
}

pub fn write_tool_mode(config: &Path, mode: &str) -> Result<(), String> {
    if !TOOL_OUTPUT_MODES.contains(&mode) {
        return Err(format!("Invalid option: {mode}"));
    }
    if let Some(parent) = config.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(config, json!({"tool_outputs": mode}).to_string()).map_err(|e| e.to_string())
}

/// Background indexing state, shared with the server.
#[derive(Default)]
pub struct IndexState {
    pub running: bool,
    pub last_error: Option<String>,
    pub last_summary: Option<SyncSummary>,
}

pub type SharedIndex = Arc<Mutex<IndexState>>;

pub fn new_shared() -> SharedIndex {
    Arc::new(Mutex::new(IndexState::default()))
}

/// Starts an indexing pass in a thread. False if one is already running.
pub fn start(shared: &SharedIndex, home: std::path::PathBuf, db: std::path::PathBuf, config: std::path::PathBuf) -> bool {
    {
        let mut state = shared.lock().expect("index state");
        if state.running {
            return false;
        }
        state.running = true;
        state.last_error = None;
    }
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        let mode = read_tool_mode(&config);
        let result = sync(&db, super::sources::all_sources(&home), &mode);
        let mut state = shared.lock().expect("index state");
        match result {
            Ok(summary) => state.last_summary = Some(summary),
            Err(err) => state.last_error = Some(err),
        }
        state.running = false;
    });
    true
}

/// Full state for /api/index_status and the reindex responses.
pub fn info(shared: &SharedIndex, db: &Path, config: &Path) -> Value {
    let state = shared.lock().expect("index state");
    json!({
        "running": state.running,
        "last_error": state.last_error,
        "last_summary": state.last_summary,
        "stats": stats(db),
        "tool_outputs": read_tool_mode(config),
    })
}
