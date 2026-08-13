# ccusage-adapter-cline

The Cline adapter: it turns the Cline SQLite sessions database
into the usage entries the reports render.

## Owns

- `loader.rs` — reading the source, dedupe, and date filtering.
- `parser.rs` — raw record parsing, token mapping, and model naming.
- `paths.rs` — environment variables, default directories, and file discovery.
- `report.rs` — the JSON and table shapes where they differ from the shared ones.

## Data source

- `${CLINE_HOME:-~/.cline}/data/sessions/<session_id>/<session_id>.messages.json`

Reads per-message transcripts. Each assistant message carries its own
`modelInfo.id` and `metrics` (token counts + cost), so sessions that switch
models mid-conversation show up as separate per-model rows in the report.

## Public surface

- `loader::load_entries`
- `report::report_from_rows`
- `report::summarize_entries`
- `run`

## Depends on

- `ccusage-adapter-common`
- `ccusage-core`
- `jiff`
- `serde_json`
