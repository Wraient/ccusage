use std::{collections::HashMap, collections::HashSet};

use crate::{LoadedEntry, PricingMap, Result, cli::SharedArgs, debug_log};

use super::{
    parser::{ZcodeEntry, message_entry, model_usage_entry, to_loaded_entry},
    paths::zcode_db_path,
};

/// Every request row ZCode kept. `model_usage` is pruned to the last ~30 days,
/// which is why `message` exists as a second source: assistant messages carry
/// the same token summary and survive pruning.
const MODEL_USAGE_QUERY: &str = r#"
SELECT id, session_id, model_id, input_tokens, output_tokens,
       cache_creation_input_tokens, cache_read_input_tokens, started_at,
       status, assistant_message_id
FROM model_usage
"#;

const SESSION_QUERY: &str = r#"
SELECT id, directory
FROM session
"#;

const MESSAGE_QUERY: &str = r#"
SELECT id, session_id, time_created, data
FROM message
"#;

pub fn load_entries(shared: &SharedArgs, pricing: &PricingMap) -> Result<Vec<LoadedEntry>> {
    crate::progress::track_usage_load(
        crate::progress::UsageLoadAgent("ZCode"),
        shared.json,
        || load_entries_inner(shared, pricing),
    )
}

fn load_entries_inner(shared: &SharedArgs, pricing: &PricingMap) -> Result<Vec<LoadedEntry>> {
    let tz = crate::parse_tz(shared.timezone.as_deref());
    let db_path = zcode_db_path()?;
    let Ok(connection) =
        sqlite::Connection::open_with_flags(&db_path, sqlite::OpenFlags::new().with_read_only())
    else {
        debug_log(
            shared,
            format!("Failed to open ZCode database {}", db_path.display()),
        );
        return Ok(Vec::new());
    };

    let projects = session_projects(&connection, shared);
    let project_of = |session_id: &str| -> &str {
        projects
            .get(session_id)
            .map(String::as_str)
            .unwrap_or("zcode")
    };

    let mut entries = Vec::new();
    let mut matched_messages = HashSet::new();
    for row in model_usage_entries(&connection, shared) {
        if let Some(assistant_message_id) = row.assistant_message_id {
            matched_messages.insert((row.entry.session_id.clone(), assistant_message_id));
        }
        let session_id = row.entry.session_id.clone();
        entries.push(to_loaded_entry(
            row.entry,
            tz.as_ref(),
            pricing,
            project_of(&session_id),
        ));
    }
    for entry in message_backfill_entries(&connection, shared, &matched_messages) {
        let session_id = entry.session_id.clone();
        entries.push(to_loaded_entry(entry, tz.as_ref(), pricing, project_of(&session_id)));
    }
    entries.sort_by_key(|entry| entry.timestamp);
    Ok(entries)
}

struct ModelUsageRow {
    entry: ZcodeEntry,
    assistant_message_id: Option<String>,
}

/// The `model_usage` columns before conversion; the parser decides what is
/// usable and how tokens are netted.
struct RawModelUsageRow {
    id: String,
    session_id: String,
    model_id: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_creation: i64,
    cache_read: i64,
    started_at: i64,
    status: String,
    assistant_message_id: Option<String>,
}

fn model_usage_entries(
    connection: &sqlite::Connection,
    shared: &SharedArgs,
) -> Vec<ModelUsageRow> {
    let Some(mut statement) = prepare(connection, shared, MODEL_USAGE_QUERY) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    loop {
        match statement.next() {
            Ok(sqlite::State::Row) => {
                let Ok(raw) = (|| -> sqlite::Result<RawModelUsageRow> {
                    Ok(RawModelUsageRow {
                        id: statement.read::<String, _>(0)?,
                        session_id: statement.read::<String, _>(1)?,
                        model_id: statement.read::<String, _>(2)?,
                        input_tokens: statement.read::<i64, _>(3)?,
                        output_tokens: statement.read::<i64, _>(4)?,
                        cache_creation: statement.read::<i64, _>(5)?,
                        cache_read: statement.read::<i64, _>(6)?,
                        started_at: statement.read::<i64, _>(7)?,
                        status: statement.read::<String, _>(8)?,
                        assistant_message_id: statement.read::<Option<String>, _>(9)?,
                    })
                })() else {
                    continue;
                };
                if let Some(entry) = model_usage_entry(
                    &raw.id,
                    &raw.session_id,
                    &raw.model_id,
                    raw.input_tokens,
                    raw.output_tokens,
                    raw.cache_creation,
                    raw.cache_read,
                    raw.started_at,
                    &raw.status,
                ) {
                    rows.push(ModelUsageRow {
                        entry,
                        assistant_message_id: raw.assistant_message_id,
                    });
                }
            }
            Ok(sqlite::State::Done) => break,
            Err(_) => {
                debug_log(shared, "Failed to read a ZCode model_usage row".to_string());
                break;
            }
        }
    }
    rows
}

fn session_projects(connection: &sqlite::Connection, shared: &SharedArgs) -> HashMap<String, String> {
    let Some(mut statement) = prepare(connection, shared, SESSION_QUERY) else {
        return HashMap::new();
    };
    let mut projects = HashMap::new();
    loop {
        match statement.next() {
            Ok(sqlite::State::Row) => {
                let Ok(id) = statement.read::<String, _>(0) else {
                    continue;
                };
                let Ok(directory) = statement.read::<String, _>(1) else {
                    continue;
                };
                let project = std::path::Path::new(&directory)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "zcode".to_string());
                projects.insert(id, project);
            }
            Ok(sqlite::State::Done) => break,
            Err(_) => {
                debug_log(shared, "Failed to read a ZCode session row".to_string());
                break;
            }
        }
    }
    projects
}

/// Assistant messages from before `model_usage` was pruned. A message whose id
/// a kept `model_usage` row already attributed is skipped, so the two sources
/// never double count the same request.
fn message_backfill_entries(
    connection: &sqlite::Connection,
    shared: &SharedArgs,
    matched_messages: &HashSet<(String, String)>,
) -> Vec<ZcodeEntry> {
    let Some(mut statement) = prepare(connection, shared, MESSAGE_QUERY) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    loop {
        match statement.next() {
            Ok(sqlite::State::Row) => {
                let Ok(id) = statement.read::<String, _>(0) else {
                    continue;
                };
                let Ok(session_id) = statement.read::<String, _>(1) else {
                    continue;
                };
                let Ok(time_created) = statement.read::<i64, _>(2) else {
                    continue;
                };
                let Ok(data) = statement.read::<String, _>(3) else {
                    continue;
                };
                if matched_messages.contains(&(session_id.clone(), id.clone())) {
                    continue;
                }
                if let Some(entry) = message_entry(&id, &session_id, time_created, &data) {
                    entries.push(entry);
                }
            }
            Ok(sqlite::State::Done) => break,
            Err(_) => {
                debug_log(shared, "Failed to read a ZCode message row".to_string());
                break;
            }
        }
    }
    entries
}

/// A database created by a different ZCode version can be missing a table,
/// which is a prepare-time error rather than a reason to discard the rest.
fn prepare<'a>(
    connection: &'a sqlite::Connection,
    shared: &SharedArgs,
    query: &str,
) -> Option<sqlite::Statement<'a>> {
    match connection.prepare(query) {
        Ok(statement) => Some(statement),
        Err(_) => {
            debug_log(shared, format!("Failed to prepare ZCode query: {query}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use ccusage_test_support::fs_fixture;

    const SCHEMA: &str = r#"
CREATE TABLE model_usage (
  id TEXT PRIMARY KEY,
  session_id TEXT,
  model_id TEXT,
  status TEXT,
  started_at INTEGER,
  input_tokens INTEGER,
  output_tokens INTEGER,
  cache_creation_input_tokens INTEGER,
  cache_read_input_tokens INTEGER,
  assistant_message_id TEXT
);
CREATE TABLE message (
  id TEXT PRIMARY KEY,
  session_id TEXT,
  time_created INTEGER,
  data TEXT
);
CREATE TABLE session (
  id TEXT PRIMARY KEY,
  directory TEXT
);
"#;

    fn fixture_db(path: &Path, rows: &[(&str, Vec<sqlite::Value>)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let db = sqlite::open(path).unwrap();
        db.execute(SCHEMA).unwrap();
        for (sql, binds) in rows {
            let mut statement = db.prepare(sql).unwrap();
            for (idx, value) in binds.iter().enumerate() {
                statement.bind((idx + 1, value)).unwrap();
            }
            statement.next().unwrap();
        }
    }

    fn model_usage_row(
        id: &str,
        session_id: &str,
        model_id: &str,
        input: i64,
        output: i64,
        cache_read: i64,
        started_at: i64,
        status: &str,
        assistant_message_id: &str,
    ) -> (&'static str, Vec<sqlite::Value>) {
        (
            "INSERT INTO model_usage (id, session_id, model_id, input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens, started_at, status, assistant_message_id) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9)",
            vec![
                id.into(),
                session_id.into(),
                model_id.into(),
                input.into(),
                output.into(),
                cache_read.into(),
                started_at.into(),
                status.into(),
                assistant_message_id.into(),
            ],
        )
    }

    fn message_row(
        id: &str,
        session_id: &str,
        time_created: i64,
        data: &str,
    ) -> (&'static str, Vec<sqlite::Value>) {
        (
            "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
            vec![id.into(), session_id.into(), time_created.into(), data.into()],
        )
    }

    fn session_row(id: &str, directory: &str) -> (&'static str, Vec<sqlite::Value>) {
        (
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            vec![id.into(), directory.into()],
        )
    }

    fn load(db_path: &Path) -> Vec<LoadedEntry> {
        let shared = SharedArgs::default();
        let pricing = PricingMap::default();
        let tz = Some(jiff::tz::TimeZone::UTC);
        let connection =
            sqlite::Connection::open_with_flags(db_path, sqlite::OpenFlags::new().with_read_only())
                .unwrap();
        let projects = session_projects(&connection, &shared);
        let project_of = |session_id: &str| {
            projects
                .get(session_id)
                .map(String::as_str)
                .unwrap_or("zcode")
        };
        let mut entries = Vec::new();
        let mut matched = HashSet::new();
        for row in model_usage_entries(&connection, &shared) {
            if let Some(message_id) = row.assistant_message_id {
                matched.insert((row.entry.session_id.clone(), message_id));
            }
            let session_id = row.entry.session_id.clone();
            entries.push(to_loaded_entry(
                row.entry,
                tz.as_ref(),
                &pricing,
                project_of(&session_id),
            ));
        }
        for entry in message_backfill_entries(&connection, &shared, &matched) {
            let session_id = entry.session_id.clone();
            entries.push(to_loaded_entry(entry, tz.as_ref(), &pricing, project_of(&session_id)));
        }
        entries.sort_by_key(|entry| entry.timestamp);
        entries
    }

    #[test]
    fn loads_request_rows_and_backfills_older_messages() {
        let fixture = fs_fixture!({});
        let db_path = fixture.path("cli/db/db.sqlite");
        fixture_db(
            &db_path,
            &[
                session_row("sess-a", "/home/user/projects/ccusage"),
                model_usage_row(
                    "usage-1",
                    "sess-a",
                    "glm-5.2",
                    16424,
                    199,
                    16384,
                    1786089309138,
                    "completed",
                    "msg-1",
                ),
                // Same request as usage-1, so it must be skipped by the dedupe.
                message_row(
                    "msg-1",
                    "sess-a",
                    1786089309138,
                    r#"{"role":"assistant","modelID":"glm-5.2","tokens":{"total":16917,"input":16754,"output":163,"cache":{"read":7296,"write":0}}}"#,
                ),
                // From before model_usage was pruned: only the message survives.
                message_row(
                    "msg-old",
                    "sess-a",
                    1750000000000,
                    r#"{"role":"assistant","modelID":"glm-5.2","tokens":{"total":100,"input":90,"output":10,"cache":{"read":0,"write":0}}}"#,
                ),
            ],
        );

        let entries = load(&db_path);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].session_id.as_ref(), "sess-a");
        assert_eq!(entries[0].project.as_ref(), "ccusage");
        // The older message predates the model_usage prune and is backfilled.
        assert_eq!(entries[0].data.message.usage.input_tokens, 90);
        assert_eq!(entries[0].data.message.usage.output_tokens, 10);
        assert_eq!(entries[1].data.message.usage.input_tokens, 40);
        assert_eq!(entries[1].data.message.usage.cache_read_input_tokens, 16384);
    }

    #[test]
    fn skips_running_rows_and_falls_back_to_the_source_name() {
        let fixture = fs_fixture!({});
        let db_path = fixture.path("cli/db/db.sqlite");
        fixture_db(
            &db_path,
            &[
                model_usage_row(
                    "usage-running",
                    "sess-x",
                    "glm-5.2",
                    500,
                    10,
                    0,
                    1786089309138,
                    "running",
                    "",
                ),
                model_usage_row(
                    "usage-ok",
                    "sess-x",
                    "glm-5.2",
                    100,
                    20,
                    0,
                    1786089309140,
                    "completed",
                    "",
                ),
            ],
        );

        let entries = load(&db_path);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].data.message.usage.input_tokens, 100);
        // No session row exists for sess-x.
        assert_eq!(entries[0].project.as_ref(), "zcode");
    }

    #[test]
    fn tolerates_a_database_without_the_expected_tables() {
        let fixture = fs_fixture!({});
        let db_path = fixture.path("cli/db/db.sqlite");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        sqlite::open(&db_path)
            .unwrap()
            .execute("CREATE TABLE unrelated (id TEXT PRIMARY KEY)")
            .unwrap();

        assert!(load(&db_path).is_empty());
    }
}
