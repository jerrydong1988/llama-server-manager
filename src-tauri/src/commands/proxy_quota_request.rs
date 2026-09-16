//! Normalize only quota-enabled generation requests, before counting or forwarding.
use crate::commands::usage_quota::QuotaError;
use serde_json::{json, Value};

const OUTPUT_FIELDS: [&str; 5] = [
    "max_tokens",
    "max_completion_tokens",
    "max_output_tokens",
    "n_predict",
    "max_new_tokens",
];

pub(super) struct OutputLimit {
    pub tokens: u64,
    pub defaulted: bool,
    field: &'static str,
}

impl OutputLimit {
    pub fn fit_default(&mut self, input: u64, context: Option<u64>, value: &mut Value) {
        // Explicit limits retain the client's intent and receive the usual context error.
        // Only an injected default shrinks to fit the selected target's remaining context.
        if self.defaulted {
            if let Some(context) = context.filter(|n| *n > 0) {
                self.tokens = self.tokens.min(context.saturating_sub(input).max(1));
                value[self.field] = json!(self.tokens);
            }
        }
    }
}

fn integer(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return Some(n);
    }
    if let Some(s) = value.as_str() {
        return s.trim().parse().ok();
    }
    value
        .as_f64()
        .filter(|n| n.fract() == 0.0 && *n >= -2.0 && *n <= i32::MAX as f64)
        .map(|n| n as i64)
}

pub(super) fn normalize(
    path: &str,
    value: &mut Value,
    default: u32,
) -> Result<OutputLimit, QuotaError> {
    let field = match path {
        "/v1/chat/completions" | "/v1/messages" | "/v1/completions" => "max_tokens",
        "/v1/responses" => "max_output_tokens",
        _ => return Err(QuotaError::Unmetered),
    };
    let object = value.as_object_mut().ok_or(QuotaError::Unmetered)?;
    for name in ["n", "best_of", "num_return_sequences"] {
        if let Some(v) = object.get(name).filter(|v| !v.is_null()) {
            if integer(v) != Some(1) {
                return Err(QuotaError::MultipleGenerations(name));
            }
        }
    }
    let mut finite: Option<u64> = None;
    for name in OUTPUT_FIELDS {
        if let Some(v) = object.get(name).filter(|v| !v.is_null()) {
            let n = integer(v)
                .filter(|n| (-2..=i32::MAX as i64).contains(n))
                .ok_or(QuotaError::InvalidLimit(name))?;
            if n >= 0 {
                // Honor every finite bound; never let another alias override it upward.
                finite = Some(finite.map_or(n as u64, |current| current.min(n as u64)));
            }
        }
    }
    if path == "/v1/completions" {
        let prompt = object.get_mut("prompt").ok_or(QuotaError::Unmetered)?;
        if let Some(items) = prompt.as_array() {
            if items.len() == 1 && (items[0].is_string() || items[0].is_array()) {
                *prompt = items[0].clone();
            }
        }
        if !prompt.is_string()
            && !prompt
                .as_array()
                .is_some_and(|a| !a.is_empty() && a.iter().all(|n| n.as_u64().is_some()))
        {
            return Err(QuotaError::UnsupportedBatch);
        }
    }
    for name in OUTPUT_FIELDS
        .into_iter()
        .chain(["n", "best_of", "num_return_sequences"])
    {
        object.remove(name);
    }
    let default = if default == 0 {
        crate::models::DEFAULT_QUOTA_OUTPUT_TOKENS
    } else {
        default.min(i32::MAX as u32)
    };
    let tokens = finite.unwrap_or(u64::from(default));
    object.insert(field.into(), json!(tokens));
    Ok(OutputLimit {
        tokens,
        defaulted: finite.is_none(),
        field,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_null_and_unbounded_values_use_default_for_every_protocol() {
        for (path, canonical) in [
            ("/v1/chat/completions", "max_tokens"),
            ("/v1/responses", "max_output_tokens"),
            ("/v1/messages", "max_tokens"),
            ("/v1/completions", "max_tokens"),
        ] {
            for raw in [
                json!({}),
                json!({"max_tokens":null}),
                json!({"max_tokens":-1}),
                json!({"n_predict":-2}),
            ] {
                let mut body = raw;
                body["prompt"] = json!("Hello");
                body["stream"] = json!(true);
                let limit = normalize(path, &mut body, 32768).unwrap();
                assert!(limit.defaulted);
                assert_eq!(body[canonical], 32768);
                assert_eq!(body["stream"], true);
                assert_eq!(
                    OUTPUT_FIELDS
                        .iter()
                        .filter(|f| body.get(**f).is_some())
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn aliases_numeric_clients_and_single_generation_are_normalized() {
        for alias in OUTPUT_FIELDS {
            for raw in [json!(48), json!(48.0), json!("48")] {
                let mut body =
                    json!({"prompt":["Hello"], "n":null,"best_of":"1","num_return_sequences":1});
                body[alias] = raw;
                let limit = normalize("/v1/completions", &mut body, 8).unwrap();
                assert!(!limit.defaulted);
                assert_eq!(limit.tokens, 48);
                assert_eq!(body, json!({"prompt":"Hello","max_tokens":48}));
            }
        }
        let mut body =
            json!({"max_tokens":64,"max_completion_tokens":32,"n_predict":-1,"max_new_tokens":128});
        assert_eq!(
            normalize("/v1/chat/completions", &mut body, 16)
                .unwrap()
                .tokens,
            32
        );
        assert_eq!(body, json!({"max_tokens":32}));
        let mut prefill = json!({"max_tokens":0,"n_predict":-1});
        let limit = normalize("/v1/chat/completions", &mut prefill, 32).unwrap();
        assert!(!limit.defaulted);
        assert_eq!(limit.tokens, 0);
        assert_eq!(prefill, json!({"max_tokens":0}));
    }

    #[test]
    fn malformed_limits_and_multiple_generations_remain_actionable_errors() {
        for raw in [
            json!(true),
            json!(1.5),
            json!("unlimited"),
            json!(-3),
            json!(2147483648_u64),
        ] {
            assert!(matches!(
                normalize("/v1/chat/completions", &mut json!({"max_tokens":raw}), 32),
                Err(QuotaError::InvalidLimit("max_tokens"))
            ));
        }
        for field in ["n", "best_of", "num_return_sequences"] {
            let mut body = json!({"max_tokens":16});
            body[field] = json!(2);
            assert!(matches!(
                normalize("/v1/chat/completions", &mut body, 32),
                Err(QuotaError::MultipleGenerations(_))
            ));
        }
        assert!(matches!(
            normalize("/v1/completions", &mut json!({"prompt":["a","b"]}), 32),
            Err(QuotaError::UnsupportedBatch)
        ));
        let mut body = json!({"prompt":[[1,2,3]]});
        normalize("/v1/completions", &mut body, 32).unwrap();
        assert_eq!(body["prompt"], json!([1, 2, 3]));
    }

    #[test]
    fn only_injected_defaults_shrink_to_context_and_never_to_zero() {
        for (input, expected) in [(90, 10), (100, 1), (101, 1)] {
            let mut body = json!({});
            let mut limit = normalize("/v1/messages", &mut body, 32).unwrap();
            limit.fit_default(input, Some(100), &mut body);
            assert_eq!(limit.tokens, expected);
            assert_eq!(body["max_tokens"], expected);
        }
        let mut body = json!({"max_tokens":32});
        let mut limit = normalize("/v1/messages", &mut body, 16).unwrap();
        limit.fit_default(90, Some(100), &mut body);
        assert_eq!(limit.tokens, 32);
        assert_eq!(
            normalize("/v1/messages", &mut json!({}), 0).unwrap().tokens,
            32768
        );
    }
}
