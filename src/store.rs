//! Plain-text persistence under %APPDATA%\Ada (settings and lists) and
//! %LOCALAPPDATA%\Ada (caches and the file shelf).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

fn dir_from_env(var: &str) -> PathBuf {
    let base = std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("Ada");
    // One-time migration from the old app name.
    if !dir.exists() {
        let old = base.join("YESDEV");
        if old.exists() {
            let _ = fs::rename(&old, &dir);
        }
    }
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Roaming data: settings, todo list, calendar events.
pub fn data_dir() -> PathBuf {
    dir_from_env("APPDATA")
}

/// Machine-local data: caches and the shelf folder.
pub fn local_dir() -> PathBuf {
    dir_from_env("LOCALAPPDATA")
}

pub fn read_text(name: &str) -> Option<String> {
    fs::read_to_string(data_dir().join(name)).ok()
}

/// Writes through a temporary file so a crash never leaves a half-written file.
pub fn write_text(name: &str, contents: &str) -> io::Result<()> {
    let path = data_dir().join(name);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, &path)
}

/// Reads `key=value` lines; blank lines and lines starting with `#` are ignored.
pub fn read_kv(name: &str) -> BTreeMap<String, String> {
    read_text(name)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

pub fn write_kv(name: &str, map: &BTreeMap<String, String>) -> io::Result<()> {
    let body: String = map.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    write_text(name, &body)
}

pub fn read_lines(name: &str) -> Vec<String> {
    read_text(name).unwrap_or_default().lines().map(str::to_string).collect()
}

pub fn write_lines(name: &str, lines: &[String]) -> io::Result<()> {
    let mut body = lines.join("\n");
    body.push('\n');
    write_text(name, &body)
}
