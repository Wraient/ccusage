# ccusage-adapter-zcode

The ZCode adapter: it turns ZCode's local SQLite database into the usage
entries the reports render.

## Owns

- `paths.rs` — database location and `ZCODE_DATA_DIR` override.
- `parser.rs` — row-to-entry conversion, token mapping, and model naming.
- `loader.rs` — the read-only SQL queries, the two-source dedupe, and entry points.
- `report.rs` — the JSON and table shapes where they differ from the shared ones.

## Data source

- `~/.zcode/cli/db/db.sqlite` (SQLite; overridable via `ZCODE_DATA_DIR`)

ZCode stores one usage row per model request in `model_usage`, which is pruned
to the last ~30 days. Assistant messages keep the same token summary in the
durable `message` table, so the adapter reads both:

- `model_usage` — per-request tokens, model, status, timestamp, and the
  `assistant_message_id` linking the row to its message.
- `message` — assistant-message token summaries for requests whose
  `model_usage` row has been pruned.
- `session` — directory per session, from which the project name (basename)
  comes.

A message whose id a kept `model_usage` row already attributed is skipped, so
the two sources never double count the same request.

ZCode stores no cost, so every entry is priced from the shared pricing map by
model name; models without a published rate card surface the usual
missing-pricing warning.

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
- `sqlite`

See `src/README.md` for the table schemas and token rules.
