//! Persist only reviewed diagnostic categories, never upstream error bodies.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageFailure {
    pub stage: String,
    pub code: String,
    pub reason: String,
}

impl UsageFailure {
    pub fn new(stage: &str, code: &str) -> Self {
        let reason = match code {
            "context_length_exceeded" => "Input and requested output exceed the route context window.",
            "authentication_failed" => "The router could not authenticate this request.",
            "permission_denied" => "The request is not permitted by the router access policy.",
            "rate_limited" => "The API key request rate limit was reached.",
            "queue_timeout" => "No router concurrency slot became available before the queue deadline.",
            "route_unavailable" => "No matching route is ready to accept this request.",
            "route_capacity" => "All matching routes are at capacity or unavailable.",
            "body_capacity" => "The router request body memory budget was reached.",
            "upstream_timeout" => "The upstream response deadline was exceeded.",
            "upstream_connection_failed" => "The router could not complete the upstream connection.",
            "upstream_error" => "The upstream returned an error; inspect the instance using the correlated request ID.",
            "stream_timeout" => "The upstream stream exceeded the idle deadline.",
            "stream_interrupted" => "The stream ended without a complete protocol response.",
            "client_cancelled" => "The request was dropped before response delivery completed.",
            "response_error" => "The upstream response could not be read within the router limits.",
            "invalid_request" => "The request did not satisfy the endpoint contract.",
            _ => "The router could not complete this request.",
        };
        Self {
            stage: stage.into(),
            code: code.into(),
            reason: reason.into(),
        }
    }

    pub fn local(status: u16, message: &str) -> Self {
        let (stage, code) = match message {
            "router concurrency limit exceeded" => ("queue", "queue_timeout"),
            "router in-flight request body budget exceeded" => ("admission", "body_capacity"),
            "all matching routes are unavailable or at capacity"
            | "all matching targets are at capacity" => ("routing", "route_capacity"),
            "no public route matches the requested model" => ("routing", "route_unavailable"),
            _ => match status {
                401 => ("authentication", "authentication_failed"),
                403 => ("authentication", "permission_denied"),
                429 => ("admission", "rate_limited"),
                400 | 404 | 405 | 413 | 422 => ("validation", "invalid_request"),
                _ => ("routing", "internal_error"),
            },
        };
        Self::new(stage, code)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextBudget {
    pub input_tokens: Option<u64>,
    pub requested_output_tokens: u64,
    pub context_window: u64,
    pub excess_tokens: Option<u64>,
    // exact / output_only / not_needed / unavailable. None is never a measured zero.
    pub input_source: String,
}

impl ContextBudget {
    pub fn new(input: Option<u64>, output: u64, window: u64, source: &str) -> Self {
        Self {
            input_tokens: input,
            requested_output_tokens: output,
            context_window: window,
            excess_tokens: input
                .map(|n| n.saturating_add(output).saturating_sub(window))
                .or_else(|| (output > window).then_some(output.saturating_sub(window))),
            input_source: source.into(),
        }
    }
}

pub(crate) fn correlation_id(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:/".contains(&b)))
    .then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_separate_from_measured_consumption() {
        let b = ContextBudget::new(Some(91_675), 65_536, 131_072, "exact");
        assert_eq!(b.excess_tokens, Some(26_139));
        assert_eq!(
            ContextBudget::new(None, 16_384, 131_072, "unavailable").excess_tokens,
            None
        );
        assert_eq!(
            ContextBudget::new(None, 200, 100, "output_only").excess_tokens,
            Some(100)
        );
    }

    #[test]
    fn diagnostic_reasons_do_not_copy_untrusted_text() {
        let json = serde_json::to_string(&UsageFailure::local(
            400,
            "secret prompt /private/model sk-secret",
        ))
        .unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("/private"));
        assert!(correlation_id("req_123/456-ab").is_some());
        assert!(correlation_id("token\nsecret").is_none());
        assert!(correlation_id(&"x".repeat(129)).is_none());
    }
}
