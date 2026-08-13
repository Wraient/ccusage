# ZCode record shapes

ZCode is Zhipu AI's agentic development environment. It keeps a local SQLite
database at `~/.zcode/cli/db/db.sqlite` (WAL mode), overridable with the
`ZCODE_DATA_DIR` environment variable. The adapter opens it read-only. There
is no committed schema from Zhipu; the tables below are what the adapter
reads.

## `model_usage` — one row per model request

Pruned to the last ~30 days.

| column | meaning |
|---|---|
| `id` | row id (`usage_model_…`) |
| `session_id` | owning session (`sess_…`); subagents have their own sessions |
| `model_id` | provider model name, e.g. `glm-5.2` |
| `status` | `completed` / `error` / `cancelled` / `running`; `running` rows are skipped |
| `started_at` | milliseconds since the Unix epoch |
| `input_tokens` | **gross, includes `cache_read_input_tokens`** (netted before storing) |
| `output_tokens` | gross; `reasoning_tokens` is a subset of it |
| `cache_creation_input_tokens` | cache-creation tokens |
| `cache_read_input_tokens` | cached prefix replayed this request |
| `assistant_message_id` | the `message.id` this request produced (`msg_…`) |

## `message` — durable per-message records

| column | meaning |
|---|---|
| `id` | `msg_…`; assistant messages referenced from `model_usage` match here |
| `session_id` | owning session |
| `time_created` | milliseconds since the Unix epoch |
| `data` | JSON with `role`, `modelID`, and `tokens` |

`tokens` uses `{"total", "input", "output", "reasoning", "cache": {"read", "write"}}`
where `input` is gross of `cache.read` and `cache.write` counts cache-creation
tokens. Only `role == "assistant"` messages carry summaries; per-part rows
(`msg_part_…_message`) hold zero tokens and are skipped.

## `session`

| column | meaning |
|---|---|
| `id` | `sess_…` |
| `directory` | project directory; the basename is the project name, falling back to `zcode` |

## Dedupe

A kept `model_usage` row and its assistant message describe the same request.
The loader skips any message whose id equals a kept row's
`assistant_message_id`, so the pruned-history backfill from `message` never
double counts. Entry identity for reports is the `(session_id, row id)` pair.

## Cost

ZCode stores no cost, so entries are priced from the shared pricing map by
model name. Unknown models report $0 and the standard missing-pricing warning;
users can add pricing overrides in ccusage config.
