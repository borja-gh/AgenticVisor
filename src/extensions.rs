//! Skills and plugins: inventory and actions to enable, disable and edit.
//! Equivalente a extensions.py.
//!
//! - Only paths inside the known skill and plugin folders are touched.
//! - Before writing, a backup goes to the data folder.
//! - Disabling a skill MOVES its folder (or its symlink) to skills-disabled/.
//! - Plugins are enabled with a boolean in settings.json (Claude Code's switch).

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Value};

use crate::backup::{backup_file, log_change};
use crate::fsutil::{dir_stats, human_size, read_frontmatter_field};

pub const SKILL_FILE: &str = "SKILL.md";

fn read_json(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

struct Paths {
    settings: PathBuf,
    installed: PathBuf,
    claude_skills: PathBuf,
    claude_skills_disabled: PathBuf,
    cursor_skills: PathBuf,
    cursor_plugins: PathBuf,
}

fn paths(home: &Path) -> Paths {
    let claude = home.join(".claude");
    let cursor = home.join(".cursor");
    Paths {
        settings: claude.join("settings.json"),
        installed: claude.join("plugins").join("installed_plugins.json"),
        claude_skills: claude.join("skills"),
        claude_skills_disabled: claude.join("skills-disabled"),
        cursor_skills: cursor.join("skills-cursor"),
        cursor_plugins: cursor.join("plugins"),
    }
}

fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let mut items: Vec<PathBuf> = fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    items.sort_by_key(|p| p.file_name().unwrap_or_default().to_string_lossy().to_lowercase());
    items
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn skill_entry(path: &Path, name: &str, enabled: bool) -> Value {
    let is_link = fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
    let real = if is_link { fs::canonicalize(path).unwrap_or(path.to_path_buf()) } else { path.to_path_buf() };
    let (size, count, mtime) = if real.is_dir() { dir_stats(&real, &[]) } else { (0, 0, 0.0) };
    let skill_md = path.join(SKILL_FILE);
    json!({
        "name": name,
        "path": path.to_string_lossy(),
        "enabled": enabled,
        "linked_to": if is_link { Value::String(real.to_string_lossy().into_owned()) } else { Value::Null },
        "description": read_frontmatter_field(&skill_md, "description", 200).unwrap_or_default(),
        "has_skill_md": skill_md.is_file(),
        "size_bytes": size,
        "size_human": human_size(size),
        "item_count": count,
        "last_modified": mtime,
    })
}

fn skills_in(folder: &Path, enabled: bool) -> Vec<Value> {
    if !folder.is_dir() {
        return Vec::new();
    }
    sorted_children(folder)
        .into_iter()
        .filter(|item| {
            let name = name_of(item);
            let is_link = fs::symlink_metadata(item).map(|m| m.file_type().is_symlink()).unwrap_or(false);
            !name.starts_with('.') && (item.is_dir() || is_link)
        })
        .map(|item| skill_entry(&item, &name_of(&item), enabled))
        .collect()
}

fn claude_plugins(home: &Path) -> Vec<Value> {
    let p = paths(home);
    let installed = read_json(&p.installed);
    let enabled_map = read_json(&p.settings);
    let enabled_map = enabled_map.get("enabledPlugins").cloned().unwrap_or_else(|| json!({}));
    let Some(installed) = installed.get("plugins").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut plugins = Vec::new();
    for (plugin_id, installs) in installed {
        let Some(install) = installs.as_array().and_then(|a| a.first()) else {
            continue;
        };
        let install_path = PathBuf::from(install.get("installPath").and_then(Value::as_str).unwrap_or(""));
        let manifest = read_json(&install_path.join(".claude-plugin").join("plugin.json"));
        let (name, marketplace) = match plugin_id.split_once('@') {
            Some((n, m)) => (n.to_string(), m.to_string()),
            None => (plugin_id.clone(), String::new()),
        };
        let enabled = enabled_map.get(plugin_id).and_then(Value::as_bool);
        let version = install
            .get("version")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .or_else(|| manifest.get("version").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let (size, count, mtime) = if install_path.is_dir() { dir_stats(&install_path, &[]) } else { (0, 0, 0.0) };
        plugins.push(json!({
            "id": plugin_id,
            "name": manifest.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or(name),
            "marketplace": marketplace,
            "version": version,
            "description": manifest.get("description").and_then(Value::as_str).unwrap_or(""),
            "enabled": enabled,
            "install_path": install_path.to_string_lossy(),
            "size_bytes": size,
            "size_human": human_size(size),
            "item_count": count,
            "last_modified": mtime,
        }));
    }
    plugins.sort_by_key(|p| p["name"].as_str().unwrap_or("").to_lowercase());
    plugins
}

fn cursor_plugins(home: &Path) -> Vec<Value> {
    let root = paths(home).cursor_plugins;
    let mut plugins = Vec::new();
    for group in ["local", "cache"] {
        let folder = root.join(group);
        if !folder.is_dir() {
            continue;
        }
        for item in sorted_children(&folder).into_iter().filter(|p| p.is_dir()) {
            let (size, count, mtime) = dir_stats(&item, &[]);
            plugins.push(json!({
                "name": name_of(&item), "group": group, "path": item.to_string_lossy(),
                "size_bytes": size, "size_human": human_size(size),
                "item_count": count, "last_modified": mtime,
            }));
        }
    }
    plugins
}

pub fn inventory(home: &Path) -> Value {
    let p = paths(home);
    let mut claude_skills = skills_in(&p.claude_skills, true);
    claude_skills.extend(skills_in(&p.claude_skills_disabled, false));
    json!({
        "claude_plugins": claude_plugins(home),
        "claude_skills": claude_skills,
        "cursor_skills": skills_in(&p.cursor_skills, true),
        "cursor_plugins": cursor_plugins(home),
    })
}

fn is_plain_name(name: &str) -> bool {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*$").expect("valid regex").is_match(name)
}

/// Enables or disables a Claude Code plugin in settings.json (previous copy and log entry).
pub fn set_plugin_enabled(home: &Path, plugin_id: &str, enabled: bool, backup_dir: &Path) -> Result<Value, String> {
    let p = paths(home);
    let installed = read_json(&p.installed);
    let known = installed.get("plugins").and_then(Value::as_object).is_some_and(|m| m.contains_key(plugin_id));
    if !known {
        return Err(format!("Plugin not installed: {plugin_id}"));
    }
    if !p.settings.is_file() {
        return Err("~/.claude/settings.json does not exist".into());
    }
    let mut data: Value = serde_json::from_str(&fs::read_to_string(&p.settings).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Could not read settings.json: {e}"))?;
    let previous = data.get("enabledPlugins").and_then(|m| m.get(plugin_id)).cloned().unwrap_or(Value::Null);
    let backup = backup_file(&p.settings, backup_dir, "settings.json")?;

    let map = data
        .as_object_mut()
        .ok_or("settings.json is not a JSON object")?
        .entry("enabledPlugins")
        .or_insert_with(|| json!({}));
    map[plugin_id] = json!(enabled);

    let tmp = p.settings.with_file_name("settings.json.dashboard-tmp");
    let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())? + "\n";
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &p.settings).map_err(|e| e.to_string())?;
    log_change(backup_dir, &format!("plugin {plugin_id}: {} -> {enabled}", log_literal(&previous)));
    Ok(json!({"ok": true, "backup": backup.to_string_lossy()}))
}

/// How the previous value is written to the log (None, True, False).
fn log_literal(value: &Value) -> String {
    match value {
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// Moves a skill to skills-disabled/ or back to skills/. A symlink is only unlinked.
pub fn set_skill_disabled(home: &Path, name: &str, disabled: bool) -> Result<Value, String> {
    if !is_plain_name(name) {
        return Err(format!("Invalid skill name: {name}"));
    }
    let p = paths(home);
    let active = p.claude_skills.join(name);
    let parked = p.claude_skills_disabled.join(name);
    let (src, dst) = if disabled { (active, parked) } else { (parked, active) };
    if fs::symlink_metadata(&src).is_err() {
        return Err(format!("Not in {}/: {name}", name_of(src.parent().unwrap_or(Path::new("")))));
    }
    if fs::symlink_metadata(&dst).is_ok() {
        return Err(format!("{name} already exists in {}/", name_of(dst.parent().unwrap_or(Path::new("")))));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::rename(&src, &dst).map_err(|e| e.to_string())?;
    Ok(json!({"ok": true, "path": dst.to_string_lossy()}))
}

/// Only SKILL.md inside a known skill folder. Symlinks are not resolved: the path the user sees is validated.
fn skill_md_path(home: &Path, raw: &str) -> Result<PathBuf, String> {
    let path = std::path::absolute(raw).map_err(|e| e.to_string())?;
    let p = paths(home);
    let roots = [&p.claude_skills, &p.claude_skills_disabled, &p.cursor_skills];
    let skill_dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
    let parent_ok = skill_dir.parent().is_some_and(|parent| roots.iter().any(|r| r.as_path() == parent));
    if name_of(&path) != SKILL_FILE || !parent_ok || name_of(&skill_dir).starts_with('.') {
        return Err("Only SKILL.md files of known skills can be edited".into());
    }
    if !path.is_file() {
        return Err("SKILL.md does not exist".into());
    }
    Ok(path)
}

fn mtime_ns(path: &Path) -> Result<u64, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = meta.modified().map_err(|e| e.to_string())?;
    Ok(modified.duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos() as u64)
}

pub fn read_skill(home: &Path, raw: &str) -> Result<Value, String> {
    let path = skill_md_path(home, raw)?;
    let real = fs::canonicalize(&path).unwrap_or(path.clone());
    Ok(json!({
        "path": path.to_string_lossy(),
        "content": String::from_utf8_lossy(&fs::read(&path).map_err(|e| e.to_string())?).into_owned(),
        "mtime_ns": mtime_ns(&path)?,
        "linked_to": if real != path { Value::String(real.to_string_lossy().into_owned()) } else { Value::Null },
    }))
}

pub fn write_skill(home: &Path, raw: &str, content: &str, expected_mtime_ns: u64, backup_dir: &Path) -> Result<Value, String> {
    let path = skill_md_path(home, raw)?;
    if mtime_ns(&path)? != expected_mtime_ns {
        return Err("The file changed since you opened it. Reload before saving.".into());
    }
    let real = fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let label = format!("{}__SKILL.md", name_of(path.parent().unwrap_or(Path::new(""))));
    let backup = backup_file(&real, backup_dir, &label)?;
    let tmp = real.with_file_name(format!("{}.dashboard-tmp", name_of(&real)));
    fs::write(&tmp, content).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &real).map_err(|e| e.to_string())?;
    Ok(json!({"ok": true, "backup": backup.to_string_lossy(), "mtime_ns": mtime_ns(&real)?}))
}
