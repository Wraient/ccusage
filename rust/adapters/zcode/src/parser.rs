use std::sync::Arc;

use jiff::tz::TimeZone as JiffTimeZone;
use serde_json::Value;

use crate::{
    LoadedEntry, PricingMap, TimestampMs, TokenUsageRaw, UsageEntry, UsageMessage,
    calculate_cost_for_usage, cli::CostMode, format_date_tz, format_rfc3339_millis,
    missing_pricing_model_for_candidates,
};

pub(super) struct ZcodeEntry {
    pub(super) timestamp: TimestampMs,
    timestamp_text: String,
    pub(super) session_id: String,
    pub(super) model: String,
    usage: TokenUsageRaw,
    /// Row identity for dedupe: `model_usage.id` or `message.id`. The two id
    /// spaces never collide, so one key covers both sources.
    pub(super) dedupe_id: String,
}

/// Converts one `model_usage` row into an entry. A request still running has
/// no final usage to report, and a row with zero tokens carries nothing.
pub(super) fn model_usage_entry(
    id: &str,
    session_id: &str,
    model_id: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_creation_input_tokens: i64,
    cache_read_input_tokens: i64,
    started_at_ms: i64,
    status: &str,
) -> Option<ZcodeEntry> {
    if status == "running" {
        return None;
    }
    if started_at_ms <= 0 {
        return None;
    }
    let usage = netted_usage(
        input_tokens,
        output_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
    );
    if total_usage_is_zero(&usage) {
        return None;
    }
    let model = model_id.trim().to_string();
    let timestamp = TimestampMs::from_millis(started_at_ms);
    Some(ZcodeEntry {
        timestamp,
        timestamp_text: format_rfc3339_millis(timestamp),
        session_id: session_id.to_string(),
        model,
        usage,
        dedupe_id: id.to_string(),
    })
}

/// Converts one durable `message` row into an entry. Only assistant messages
/// carry token summaries; every other role is skipped. The `data` JSON uses
/// `tokens.input`/`tokens.output` with `tokens.cache.read`/`tokens.cache.write`.
pub(super) fn message_entry(
    id: &str,
    session_id: &str,
    time_created_ms: i64,
    data: &str,
) -> Option<ZcodeEntry> {
    if time_created_ms <= 0 {
        return None;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return None;
    };
    if value.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return None;
    }
    let model = value
        .get("modelID")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if model.is_empty() {
        return None;
    }
    let Some(tokens) = value.get("tokens") else {
        return None;
    };
    let input_tokens = tokens.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
    let output_tokens = tokens.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
    let cache_read = tokens
        .get("cache")
        .and_then(|c| c.get("read"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cache_write = tokens
        .get("cache")
        .and_then(|c| c.get("write"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let usage = netted_usage(input_tokens, output_tokens, cache_write, cache_read);
    if total_usage_is_zero(&usage) {
        return None;
    }
    let timestamp = TimestampMs::from_millis(time_created_ms);
    Some(ZcodeEntry {
        timestamp,
        timestamp_text: format_rfc3339_millis(timestamp),
        session_id: session_id.to_string(),
        model,
        usage,
        dedupe_id: format!("msg:{id}"),
    })
}

/// ZCode reports `input_tokens` gross — the cached prefix is part of it — so
/// it must be netted before storing or the cached tokens would be counted
/// twice against the input rate. `output_tokens` is gross too, with
/// `reasoning_tokens` a subset of it, which is how every other adapter treats
/// reasoning and costs it at the output rate.
fn netted_usage(
    input_tokens: i64,
    output_tokens: i64,
    cache_creation_input_tokens: i64,
    cache_read_input_tokens: i64,
) -> TokenUsageRaw {
    let input_tokens = input_tokens.max(0) as u64;
    let cache_read_input_tokens = cache_read_input_tokens.max(0) as u64;
    TokenUsageRaw {
        input_tokens: input_tokens.saturating_sub(cache_read_input_tokens),
        output_tokens: output_tokens.max(0) as u64,
        cache_read_input_tokens,
        cache_creation_input_tokens: cache_creation_input_tokens.max(0) as u64,
        speed: None,
        cache_creation: None,
    }
}

fn total_usage_is_zero(usage: &TokenUsageRaw) -> bool {
    usage.input_tokens == 0
        && usage.output_tokens == 0
        && usage.cache_read_input_tokens == 0
        && usage.cache_creation_input_tokens == 0
}

pub(super) fn to_loaded_entry(
    entry: ZcodeEntry,
    tz: Option<&JiffTimeZone>,
    pricing: &PricingMap,
    project: &str,
) -> LoadedEntry {
    let cost = calculate_zcode_cost(&entry, pricing);
    let missing_pricing_model = missing_zcode_pricing(&entry, pricing);
    let data = UsageEntry {
        session_id: Some(entry.session_id.clone()),
        timestamp: entry.timestamp_text.clone(),
        version: None,
        message: UsageMessage {
            usage: entry.usage,
            model: Some(entry.model.clone()),
            id: Some(format!("zcode:{}", entry.dedupe_id)),
        },
        cost_usd: None,
        request_id: None,
        is_api_error_message: None,
        is_sidechain: None,
    };
    LoadedEntry {
        date: format_date_tz(entry.timestamp, tz),
        timestamp: entry.timestamp,
        project: Arc::from(project),
        session_id: Arc::from(entry.session_id.as_str()),
        project_path: Arc::from(project),
        cost,
        credits: None,
        extra_total_tokens: 0,
        message_count: None,
        model: Some(entry.model),
        usage_limit_reset_time: None,
        missing_pricing_model,
        data,
    }
}

/// ZCode stores no cost, so every entry is priced from the pricing map by
/// model id.
fn calculate_zcode_cost(entry: &ZcodeEntry, pricing: &PricingMap) -> f64 {
    let cost = calculate_cost_for_usage(
        Some(&entry.model),
        entry.usage,
        None,
        CostMode::Calculate,
        Some(pricing),
    );
    if cost.is_finite() && cost > 0.0 {
        cost
    } else {
        0.0
    }
}

fn missing_zcode_pricing(entry: &ZcodeEntry, pricing: &PricingMap) -> Option<String> {
    missing_pricing_model_for_candidates(
        &entry.model,
        std::iter::once(entry.model.clone()),
        crate::total_usage_tokens(entry.usage),
        Some(pricing),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nets_gross_input_and_keeps_cache_buckets() {
        let entry = model_usage_entry(
            "usage-1",
            "sess-a",
            "glm-5.2",
            16424,
            199,
            0,
            16384,
            1_786_089_169_225,
            "completed",
        )
        .unwrap();

        assert_eq!(entry.session_id, "sess-a");
        assert_eq!(entry.model, "glm-5.2");
        assert_eq!(entry.usage.input_tokens, 40);
        assert_eq!(entry.usage.cache_read_input_tokens, 16384);
        assert_eq!(entry.usage.output_tokens, 199);
        assert_eq!(entry.timestamp.as_millis(), 1_786_089_169_225);
    }

    #[test]
    fn skips_running_and_zero_token_requests() {
        assert!(model_usage_entry(
            "usage-1",
            "sess-a",
            "glm-5.2",
            10,
            0,
            0,
            0,
            1_786_089_169_225,
            "running",
        )
        .is_none());
        assert!(model_usage_entry(
            "usage-2",
            "sess-a",
            "glm-5.2",
            0,
            0,
            0,
            0,
            1_786_089_169_225,
            "error",
        )
        .is_none());
    }

    #[test]
    fn parses_message_token_summaries_for_backfill() {
        let data = r#"{"role":"assistant","modelID":"glm-5.2","cost":0,"tokens":{"total":16917,"input":16754,"output":163,"reasoning":0,"cache":{"read":7296,"write":0}},"time":{"created":1781978391174}}"#;

        let entry = message_entry("msg-1", "sess-old", 1781978391174, data).unwrap();

        assert_eq!(entry.session_id, "sess-old");
        assert_eq!(entry.model, "glm-5.2");
        assert_eq!(entry.usage.input_tokens, 9458);
        assert_eq!(entry.usage.cache_read_input_tokens, 7296);
        assert_eq!(entry.usage.output_tokens, 163);
        assert_eq!(entry.usage.cache_creation_input_tokens, 0);
        assert_eq!(entry.timestamp.as_millis(), 1781978391174);
    }

    #[test]
    fn skips_non_assistant_messages_and_token_less_blobs() {
        let user_data = r#"{"role":"user","tokens":{"input":10,"output":0}}"#;
        assert!(message_entry("msg-1", "sess-a", 1781978391174, user_data).is_none());

        let error_data = r#"{"role":"assistant","modelID":"glm-5.2","error":{"name":"AiSdkModelAdapterError"}}"#;
        assert!(message_entry("msg-2", "sess-a", 1781978391174, error_data).is_none());

        let zero_tokens = r#"{"role":"assistant","modelID":"glm-5.2","tokens":{"total":0,"input":0,"output":0,"cache":{"read":0,"write":0}}}"#;
        assert!(message_entry("msg-3", "sess-a", 1781978391174, zero_tokens).is_none());
    }
}
