use std::collections::HashSet;

const RUNTIME_PRIORITY: &[&str] = &[
    "ngram-simple",
    "ngram-map-k",
    "ngram-map-k4v",
    "ngram-mod",
    "ngram-cache",
    "draft-simple",
    "draft-eagle3",
    "draft-mtp",
    "draft-dflash",
    "draft-dspark",
];
const REBUILDABLE_NGRAM_TYPES: &[&str] = &[
    "ngram-simple",
    "ngram-map-k",
    "ngram-map-k4v",
    "ngram-mod",
    "ngram-cache",
];
const CHECKPOINTED_DRAFT_TYPES: &[&str] = &[
    "draft-simple",
    "draft-eagle3",
    "draft-mtp",
    "draft-dflash",
    "draft-dspark",
];

fn raw_types(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|candidate| candidate.trim().to_ascii_lowercase())
        .filter(|candidate| !candidate.is_empty())
        .collect()
}

pub(crate) fn parse_speculative_types(value: &str) -> Vec<String> {
    let values = raw_types(value);
    if values
        .iter()
        .any(|candidate| candidate == "none" || candidate == "off")
    {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect()
}

pub(crate) fn normalize_speculative_types(value: &str) -> String {
    let raw = raw_types(value);
    if raw.is_empty() {
        return String::new();
    }
    if raw
        .iter()
        .any(|candidate| candidate == "none" || candidate == "off")
    {
        return "none".to_string();
    }

    let values = parse_speculative_types(value);
    let mut normalized = Vec::new();
    for known in RUNTIME_PRIORITY {
        if values.iter().any(|candidate| candidate == known) {
            normalized.push((*known).to_string());
        }
    }
    for candidate in values {
        if !normalized.contains(&candidate) {
            normalized.push(candidate);
        }
    }
    normalized.join(",")
}

pub(crate) fn checkpoint_uses_draft_state(value: &str) -> bool {
    parse_speculative_types(value)
        .iter()
        .any(|candidate| CHECKPOINTED_DRAFT_TYPES.contains(&candidate.as_str()))
}

pub(crate) fn checkpoint_speculative_types_supported(
    value: &str,
    engine_types: &[String],
    context_checkpoint_persistence: bool,
) -> bool {
    let configured = parse_speculative_types(value);
    if configured.is_empty() {
        return true;
    }
    configured.iter().all(|candidate| {
        (REBUILDABLE_NGRAM_TYPES.contains(&candidate.as_str())
            || (context_checkpoint_persistence
                && CHECKPOINTED_DRAFT_TYPES.contains(&candidate.as_str())))
            && engine_types
                .iter()
                .any(|reported| reported.trim().eq_ignore_ascii_case(candidate))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_matches_llama_runtime_priority() {
        assert_eq!(
            normalize_speculative_types(" draft-mtp,ngram-mod,ngram-mod "),
            "ngram-mod,draft-mtp"
        );
        assert_eq!(normalize_speculative_types("none,ngram-mod"), "none");
        assert_eq!(normalize_speculative_types(""), "");
    }

    #[test]
    fn checkpoint_support_requires_engine_reported_types_and_full_draft_contexts() {
        let engine_types = vec![
            "ngram-mod".to_string(),
            "ngram-cache".to_string(),
            "draft-mtp".to_string(),
        ];
        assert!(checkpoint_speculative_types_supported("", &[], false));
        assert!(checkpoint_speculative_types_supported(
            "ngram-mod,ngram-cache",
            &engine_types,
            false,
        ));
        assert!(!checkpoint_speculative_types_supported(
            "ngram-mod,draft-mtp",
            &engine_types,
            false,
        ));
        assert!(checkpoint_speculative_types_supported(
            "ngram-mod,draft-mtp",
            &engine_types,
            true,
        ));
        assert!(!checkpoint_speculative_types_supported(
            "ngram-future",
            &["ngram-future".to_string()],
            true,
        ));
        assert!(!checkpoint_speculative_types_supported(
            "ngram-mod",
            &[],
            true,
        ));
        assert!(checkpoint_speculative_types_supported("none", &[], false));
        assert!(checkpoint_uses_draft_state("ngram-mod,draft-mtp"));
        assert!(!checkpoint_uses_draft_state("ngram-mod"));
    }
}
