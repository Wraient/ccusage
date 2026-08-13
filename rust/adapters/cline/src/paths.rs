use std::{collections::HashSet, env, fs, path::{Path, PathBuf}};

use crate::Result;

const CLINE_HOME_ENV: &str = "CLINE_HOME";

/// Discovers cline `*.messages.json` transcript files under the sessions
/// directory. Each file holds the per-message model + metrics that the adapter
/// turns into usage entries, which is what lets ccusage show a per-model
/// breakdown when a session switches models mid-conversation.
pub(super) fn cline_messages_files() -> Result<Vec<PathBuf>> {
    let homes = if let Ok(paths) = env::var(CLINE_HOME_ENV) {
        paths
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    } else {
        let home =
            crate::home::home_dir().ok_or_else(|| crate::cli_error("home directory is not set"))?;
        vec![home.join(".cline")]
    };
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for home in &homes {
        let sessions_dir = home.join("data").join("sessions");
        collect_messages_files(&sessions_dir, &mut files, &mut seen);
    }
    files.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    Ok(files)
}

fn collect_messages_files(dir: &Path, files: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_file()
            && path.extension().is_some_and(|ext| ext == "json")
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".messages.json"))
            && seen.insert(path.clone())
        {
            files.push(path);
        } else if file_type.is_dir() {
            collect_messages_files(&path, files, seen);
        }
    }
}
