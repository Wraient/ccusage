use std::{env, path::PathBuf};

use crate::Result;

const ZCODE_DATA_DIR_ENV: &str = "ZCODE_DATA_DIR";

/// The ZCode agent runtime keeps one SQLite database for every session,
/// message, and model call on the machine. The desktop app itself has no
/// override for this location; the environment variable is the hook ccusage
/// (and its tests) use to point at a different root.
pub(super) fn zcode_db_path() -> Result<PathBuf> {
    let data_dir = match env::var(ZCODE_DATA_DIR_ENV) {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let home = crate::home::home_dir()
                .ok_or_else(|| crate::cli_error("home directory is not set"))?;
            home.join(".zcode")
        }
    };
    Ok(data_dir.join("cli").join("db").join("db.sqlite"))
}
