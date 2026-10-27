//! Local artifacts: Cursor canvases and Claude Code publications.
//! No internet calls: the links come from text already in the transcripts.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Local, TimeZone};
use regex::Regex;
use serde_json::{json, Value};

use crate::fsutil::{human_size, mtime_secs, resolve_dashed_path};

const CANVAS_SUFFIX: &str = ".canvas.tsx";

fn cursor_root(home: &Path) -> PathBuf {
    home.join(".cursor").join("projects")
}

fn claude_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".claude").join("projects"),
        home.join("Library").join("Application Support").join("Claude").join("local-agent-mode-sessions"),
    ]
}

pub fn cursor_canvases(home: &Path) -> Vec<Value> {
    let root = cursor_root(home);
    let Ok(projects) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut canvases = Vec::new();
    for project in projects.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let folder = project.join("canvases");
        let Ok(files) = fs::read_dir(&folder) else {
            continue;
        };
        for file in files.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
            let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(CANVAS_SUFFIX) else {
                continue;
            };
            let project_folder = project.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let project_path = resolve_dashed_path(&project_folder, "/")
                .unwrap_or_else(|| format!("/{}", project_folder.replace('-', "/")));
            let Ok(meta) = fs::metadata(&file) else {
                continue;
            };
            canvases.push(json!({
                "name": stem,
                "path": file.to_string_lossy(),
                "project": project_path,
                "size_bytes": meta.len(),
                "size_human": human_size(meta.len()),
                "last_modified": mtime_secs(&meta),
            }));
        }
    }
    canvases.sort_by(|a, b| {
        b["last_modified"].as_f64().unwrap_or(0.0).partial_cmp(&a["last_modified"].as_f64().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal)
    });
    canvases
}

fn checked_canvas(home: &Path, raw: &str) -> Result<PathBuf, String> {
    let path = std::path::absolute(raw).map_err(|e| e.to_string())?;
    let root = cursor_root(home);
    let valid = path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(CANVAS_SUFFIX))
        && path.parent().and_then(Path::file_name).is_some_and(|n| n == "canvases")
        && path.parent().and_then(Path::parent).and_then(Path::parent) == Some(root.as_path());
    if !valid || !path.is_file() {
        return Err("Only Cursor canvases can be read".into());
    }
    Ok(path)
}

pub fn read_canvas(home: &Path, raw: &str) -> Result<Value, String> {
    let path = checked_canvas(home, raw)?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "content": String::from_utf8_lossy(&fs::read(&path).map_err(|e| e.to_string())?).into_owned(),
    }))
}

#[derive(Clone)]
struct Publication {
    title: String,
    description: String,
    url: String,
    file_path: String,
    project: String,
    timestamp: f64,
}

type Cache = HashMap<PathBuf, ((u64, i128), Vec<Publication>)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn url_re() -> Regex {
    Regex::new(r#"https://claude\.ai/[^\s"']*artifact/[A-Za-z0-9_-]+"#).expect("valid regex")
}

fn html_title(file_path: &str) -> Option<String> {
    if file_path.is_empty() {
        return None;
    }
    let mut head = vec![0u8; 8192];
    let mut file = fs::File::open(file_path).ok()?;
    let n = file.read(&mut head).ok()?;
    let text = String::from_utf8_lossy(&head[..n]).into_owned();
    let re = Regex::new(r"(?is)<title>(.*?)</title>").expect("valid regex");
    re.captures(&text).map(|c| c[1].split_whitespace().collect::<Vec<_>>().join(" "))
}

fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Publications in a transcript: Artifact calls with action publish whose result carries the URL.
fn publications_in(transcript: &Path) -> Vec<Publication> {
    let Ok(text) = fs::read_to_string(transcript) else {
        return Vec::new();
    };
    if !text.contains("\"Artifact\"") {
        return Vec::new();
    }
    let cwd = text
        .lines()
        .take(50)
        .find(|l| l.contains("\"cwd\""))
        .and_then(|l| serde_json::from_str::<Value>(l).ok())
        .and_then(|d| d.get("cwd").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();

    let mut calls: HashMap<String, (Value, Option<String>)> = HashMap::new();
    let mut results: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        if line.contains("\"Artifact\"") {
            let Ok(d) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            for block in d.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) == Some("tool_use") && block.get("name").and_then(Value::as_str) == Some("Artifact") {
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        let input = block.get("input").cloned().unwrap_or(json!({}));
                        calls.insert(id.to_string(), (input, d.get("timestamp").and_then(Value::as_str).map(str::to_string)));
                    }
                }
            }
        } else if line.contains("\"tool_result\"") && line.contains("Published ") {
            let Ok(d) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            for block in d.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                    if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                        results.insert(id.to_string(), tool_result_text(block.get("content").unwrap_or(&Value::Null)));
                    }
                }
            }
        }
    }

    let fallback = fs::metadata(transcript).map(|m| mtime_secs(&m)).unwrap_or(0.0);
    let url_re = url_re();
    let mut out = Vec::new();
    for (id, (input, timestamp)) in calls {
        let Some(result) = results.get(&id) else {
            continue;
        };
        let Some(found) = url_re.find(result) else {
            continue;
        };
        if !result.starts_with("Published ") {
            continue;
        }
        let file_path = input.get("file_path").and_then(Value::as_str).unwrap_or("").to_string();
        let stem = Path::new(&file_path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let title = input
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| html_title(&file_path))
            .unwrap_or_else(|| if stem.is_empty() { "(untitled)".into() } else { stem });
        let timestamp = timestamp
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(&t.replace('Z', "+00:00")).ok())
            .map(|dt| dt.timestamp_micros() as f64 / 1_000_000.0)
            .unwrap_or(fallback);
        out.push(Publication {
            title,
            description: input.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
            url: found.as_str().to_string(),
            file_path,
            project: cwd.clone(),
            timestamp,
        });
    }
    out
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

pub fn claude_artifacts(home: &Path) -> Vec<Value> {
    let mut publications: Vec<Publication> = Vec::new();
    for root in claude_roots(home) {
        if !root.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_jsonl(&root, &mut files);
        for transcript in files {
            let Ok(meta) = fs::metadata(&transcript) else {
                continue;
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i128)
                .unwrap_or(0);
            let signature = (meta.len(), mtime);
            let cached = cache().lock().ok().and_then(|c| c.get(&transcript).filter(|(s, _)| *s == signature).map(|(_, v)| v.clone()));
            let found = match cached {
                Some(v) => v,
                None => {
                    let v = publications_in(&transcript);
                    if let Ok(mut c) = cache().lock() {
                        c.insert(transcript.clone(), (signature, v.clone()));
                    }
                    v
                }
            };
            publications.extend(found);
        }
    }
    publications.sort_by(|a, b| b.timestamp.partial_cmp(&a.timestamp).unwrap_or(std::cmp::Ordering::Equal));
    publications
        .into_iter()
        .map(|p| {
            let date = Local
                .timestamp_opt(p.timestamp as i64, 0)
                .single()
                .map(|dt| dt.format("%Y-%m-%d").to_string())
                .unwrap_or_default();
            json!({
                "title": p.title, "description": p.description, "url": p.url,
                "file_path": p.file_path, "project": p.project, "date": date,
                "timestamp": p.timestamp,
            })
        })
        .collect()
}

pub fn inventory(home: &Path) -> Value {
    json!({"canvases": cursor_canvases(home), "claude_artifacts": claude_artifacts(home)})
}
