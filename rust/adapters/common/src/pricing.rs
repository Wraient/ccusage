/// Vendor-qualified pricing lookup candidates for a bare model name.
///
/// Pricing snapshots index many models under provider namespaces (`xai/`,
/// `anthropic/`, `dashscope/`, ...) while agent logs record bare names, so a
/// lookup has to try the namespaces each model family is published under.
/// Qualified keys are lower-cased because the pricing tables are lowercase
/// while some sources record mixed-case ids (ZCode stores `GLM-5.2`).
///
/// Callers append these after their source-specific candidates; cost
/// resolution takes the first candidate that prices, so the order here is
/// most-canonical first.
pub fn vendor_namespaces(model: &str) -> Vec<String> {
    let model = model.to_ascii_lowercase();
    let qualified = |namespace: &str| format!("{namespace}/{model}");
    if model.starts_with("gemini") {
        ["gemini", "vertex_ai", "google", "openrouter/google"]
            .into_iter()
            .map(qualified)
            .collect()
    } else if model.starts_with("claude") {
        ["anthropic", "vertex_ai", "bedrock", "openrouter/anthropic"]
            .into_iter()
            .map(qualified)
            .collect()
    } else if model.starts_with("gpt")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
    {
        ["openai", "azure", "openrouter/openai"]
            .into_iter()
            .map(qualified)
            .collect()
    } else if model.starts_with("deepseek") {
        ["deepseek", "dashscope", "openrouter/deepseek"]
            .into_iter()
            .map(qualified)
            .collect()
    } else if model.starts_with("qwen") {
        ["qwen", "dashscope", "openrouter/qwen"]
            .into_iter()
            .map(qualified)
            .collect()
    } else if model.starts_with("glm") {
        ["zai", "zhipuai", "dashscope"]
            .into_iter()
            .map(qualified)
            .collect()
    } else if model.starts_with("grok") {
        ["xai", "x-ai"].into_iter().map(qualified).collect()
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::vendor_namespaces;

    #[test]
    fn qualifies_each_model_family_with_its_vendor_namespaces() {
        assert_eq!(
            vendor_namespaces("gemini-3.7-flash"),
            vec![
                "gemini/gemini-3.7-flash",
                "vertex_ai/gemini-3.7-flash",
                "google/gemini-3.7-flash",
                "openrouter/google/gemini-3.7-flash",
            ]
        );
        assert_eq!(
            vendor_namespaces("claude-sonnet-4-5"),
            vec![
                "anthropic/claude-sonnet-4-5",
                "vertex_ai/claude-sonnet-4-5",
                "bedrock/claude-sonnet-4-5",
                "openrouter/anthropic/claude-sonnet-4-5",
            ]
        );
        assert_eq!(
            vendor_namespaces("deepseek-v4-flash"),
            vec![
                "deepseek/deepseek-v4-flash",
                "dashscope/deepseek-v4-flash",
                "openrouter/deepseek/deepseek-v4-flash",
            ]
        );
    }

    #[test]
    fn lowercases_mixed_case_ids_for_the_qualified_keys() {
        assert_eq!(
            vendor_namespaces("GLM-5.2"),
            vec!["zai/glm-5.2", "zhipuai/glm-5.2", "dashscope/glm-5.2"]
        );
    }

    #[test]
    fn leaves_unknown_families_without_namespaces() {
        assert!(vendor_namespaces("muse-spark-1.2").is_empty());
        assert!(vendor_namespaces("antigravity-model-1050").is_empty());
    }
}
