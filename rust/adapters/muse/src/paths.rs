use std::{collections::HashSet, env, fs, path::{Path, PathBuf}};

use crate::Result;

const XDG_DATA_HOME_ENV: &str = "XDG_DATA_HOME";

/// Discovers Muse Code `session.jsonl` logs under the data directory. Each
/// session directory `sessions/YYYY/MM/DD/<session-uuid>/` holds one log, and
/// `subagent/<child-uuid>/` logs hold the child agents' own model calls, which
/// the parent log does not record — they must be walked too or the session
/// undercounts.
pub(super) fn muse_session_files() -> Result<Vec<PathBuf>> {
    let data_home = if let Ok(dir) = env::var(XDG_DATA_HOME_ENV) {
        if dir.is_empty() {
            return Ok(Vec::new());
        }
        PathBuf::from(dir)
    } else {
        let home = crate::home::home_dir()
            .ok_or_else(|| crate::cli_error("home directory is not set"))?;
        home.join(".local").join("share")
    };
    let sessions_dir = data_home.join("muse").join("sessions");
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    collect_session_files(&sessions_dir, &mut files, &mut seen);
    files.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    Ok(files)
}

fn collect_session_files(dir: &Path, files: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_file()
            && path
                .file_name()
                .is_some_and(|name| name == "session.jsonl")
            && seen.insert(path.clone())
        {
            files.push(path);
        } else if file_type.is_dir() {
            collect_session_files(&path, files, seen);
        }
    }
}
