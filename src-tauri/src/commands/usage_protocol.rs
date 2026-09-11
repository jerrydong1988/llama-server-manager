//! Protocol-specific accounting. Missing values are never treated as measured zero.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UsageProtocol {
    Chat,
    Completion,
    Responses,
    Anthropic,
    Embedding,
    Rerank,
    Count,
}

impl UsageProtocol {
    pub fn from_path(path: &str) -> Option<Self> {
        Some(match path {
            "/v1/chat/completions" => Self::Chat,
            "/v1/completions" => Self::Completion,
            "/v1/responses" => Self::Responses,
            "/v1/messages" => Self::Anthropic,
            "/embedding" | "/embeddings" | "/v1/embeddings" => Self::Embedding,
            "/rerank" | "/reranking" | "/v1/rerank" | "/v1/reranking" => Self::Rerank,
            "/v1/messages/count_tokens"
            | "/v1/chat/completions/input_tokens"
            | "/v1/responses/input_tokens" => Self::Count,
            _ => return None,
        })
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::Embedding => "embedding",
            Self::Rerank => "rerank",
            Self::Count => "count",
            _ => "generation",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached: Option<u64>,
    pub cache_write: Option<u64>,
    pub reasoning: Option<u64>,
}

fn count(value: &Value, name: &str) -> Option<u64> {
    // Match SQLite's signed range and reject malformed, negative and fractional values.
    value
        .get(name)?
        .as_u64()
        .filter(|n| *n <= i64::MAX as u64 / 4)
}

#[derive(Debug)]
pub(crate) struct UsageAccumulator {
    pub protocol: UsageProtocol,
    pub tokens: TokenUsage,
    pub terminal: bool,
    pub failed: bool,
    pub malformed: bool,
    pub finish_reason: Option<String>,
    pub source: &'static str,
    event: String,
    data: String,
    // Anthropic input values are disjoint; output message_delta values are cumulative.
    anthropic_input: Option<u64>,
    anthropic_read: Option<u64>,
    anthropic_write: Option<u64>,
}

impl UsageAccumulator {
    pub fn new(protocol: UsageProtocol) -> Self {
        Self {
            protocol,
            tokens: TokenUsage::default(),
            terminal: false,
            failed: false,
            malformed: false,
            finish_reason: None,
            source: "none",
            event: String::new(),
            data: String::new(),
            anthropic_input: None,
            anthropic_read: None,
            anthropic_write: None,
        }
    }

    pub fn quality(&self, eligible: bool) -> &'static str {
        if !eligible || self.protocol == UsageProtocol::Count {
            return "not_applicable";
        }
        if !self.malformed && self.tokens.input.is_some() && self.tokens.output.is_some() {
            "complete"
        } else if self.tokens.input.is_some() || self.tokens.output.is_some() {
            "partial"
        } else {
            "unknown"
        }
    }

    pub fn observe_json(&mut self, value: &Value) -> bool {
        let ty = value.get("type").and_then(Value::as_str).unwrap_or("");
        if value.get("error").is_some_and(|v| !v.is_null())
            || ty == "error"
            || ty == "response.failed"
        {
            self.failed = true;
        }
        let response = value.get("response").unwrap_or(value);
        if matches!(
            ty,
            "response.completed" | "response.failed" | "response.incomplete" | "message_stop"
        ) {
            self.terminal = true;
        }
        if ty == "response.incomplete"
            || response.get("status").and_then(Value::as_str) == Some("incomplete")
        {
            self.finish_reason = Some("incomplete".into());
        }
        if response.get("status").and_then(Value::as_str) == Some("failed") {
            self.failed = true;
        }
        if let Some(reason) = value
            .pointer("/delta/stop_reason")
            .or_else(|| value.get("stop_reason"))
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(reason.chars().take(64).collect());
        }
        if let Some(choices) = value.get("choices").and_then(Value::as_array) {
            for choice in choices {
                if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                    self.finish_reason = Some(reason.chars().take(64).collect());
                }
            }
        }
        let usage = match self.protocol {
            UsageProtocol::Anthropic if ty == "message_start" => value.pointer("/message/usage"),
            UsageProtocol::Responses => response.get("usage"),
            _ => value.get("usage"),
        };
        if let Some(usage) = usage.filter(|v| v.is_object()) {
            self.observe_usage(usage);
        }
        // Heartbeats, roles, empty deltas and usage-only frames are not first output.
        matches!(
            ty,
            "response.output_text.delta"
                | "response.function_call_arguments.delta"
                | "response.reasoning_text.delta"
        ) && value
            .get("delta")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
            || value
                .get("choices")
                .and_then(Value::as_array)
                .is_some_and(|choices| {
                    choices.iter().any(|c| {
                        c.get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                            || c.get("delta").is_some_and(|d| {
                                ["content", "reasoning_content"].iter().any(|k| {
                                    d.get(k)
                                        .and_then(Value::as_str)
                                        .is_some_and(|s| !s.is_empty())
                                }) || d
                                    .get("tool_calls")
                                    .and_then(Value::as_array)
                                    .is_some_and(|a| !a.is_empty())
                            })
                    })
                })
            || ty == "content_block_delta"
                && value.get("delta").is_some_and(|d| {
                    ["text", "thinking", "partial_json"].iter().any(|k| {
                        d.get(k)
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                    })
                })
    }

    fn observe_usage(&mut self, usage: &Value) {
        if self.protocol == UsageProtocol::Count {
            return;
        }
        self.source = "upstream_usage";
        let (input_key, output_key) = match self.protocol {
            UsageProtocol::Responses | UsageProtocol::Anthropic => {
                ("input_tokens", "output_tokens")
            }
            _ => ("prompt_tokens", "completion_tokens"),
        };
        for name in [
            input_key,
            output_key,
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ] {
            if usage.get(name).is_some_and(|v| !v.is_null()) && count(usage, name).is_none() {
                self.malformed = true;
            }
        }
        if self.protocol == UsageProtocol::Anthropic {
            if let Some(n) = count(usage, "input_tokens") {
                self.anthropic_input = Some(n);
            }
            if let Some(n) = count(usage, "cache_read_input_tokens") {
                self.anthropic_read = Some(n);
            }
            if let Some(n) = count(usage, "cache_creation_input_tokens") {
                self.anthropic_write = Some(n);
            }
            // These optional Anthropic counters default to zero by protocol, unlike an absent input count.
            self.tokens.input = self
                .anthropic_input
                .map(|n| n + self.anthropic_read.unwrap_or(0) + self.anthropic_write.unwrap_or(0));
            self.tokens.cached = self
                .anthropic_input
                .map(|_| self.anthropic_read.unwrap_or(0));
            self.tokens.cache_write = self.anthropic_write;
        } else {
            if let Some(n) = count(usage, input_key) {
                self.tokens.input = Some(n);
            }
            let details = usage.get(if self.protocol == UsageProtocol::Responses {
                "input_tokens_details"
            } else {
                "prompt_tokens_details"
            });
            if let Some(details) = details {
                self.tokens.cached = count(details, "cached_tokens");
                self.tokens.cache_write = count(details, "cache_write_tokens");
            }
        }
        if let Some(n) = count(usage, output_key) {
            self.tokens.output = Some(n);
        }
        if matches!(
            self.protocol,
            UsageProtocol::Embedding | UsageProtocol::Rerank
        ) && self.tokens.input.is_some()
        {
            self.tokens.output = Some(0);
        }
        if let Some(details) = usage.get(if self.protocol == UsageProtocol::Responses {
            "output_tokens_details"
        } else {
            "completion_tokens_details"
        }) {
            self.tokens.reasoning = count(details, "reasoning_tokens");
        }
        if self
            .tokens
            .cached
            .zip(self.tokens.input)
            .is_some_and(|(c, i)| c > i)
        {
            self.tokens.cached = None;
            self.malformed = true;
        }
        if self
            .tokens
            .reasoning
            .zip(self.tokens.output)
            .is_some_and(|(r, o)| r > o)
        {
            self.tokens.reasoning = None;
            self.malformed = true;
        }
    }

    pub fn sse_line(&mut self, line: &str) -> bool {
        if line.is_empty() {
            return self.flush_event();
        }
        if let Some(event) = line.strip_prefix("event:") {
            self.event = event.trim().chars().take(80).collect();
        }
        if let Some(data) = line.strip_prefix("data:") {
            if self.data.len() + data.len() > 16 * 1024 * 1024 {
                self.malformed = true;
                self.data.clear();
                return false;
            }
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
        }
        false
    }

    pub fn flush_event(&mut self) -> bool {
        let data = std::mem::take(&mut self.data);
        let event = std::mem::take(&mut self.event);
        if data.trim() == "[DONE]" {
            self.terminal = true;
            return false;
        }
        if data.is_empty() {
            return false;
        }
        match serde_json::from_str::<Value>(&data) {
            Ok(mut value) => {
                if let Some(object) = value.as_object_mut() {
                    if !event.is_empty() {
                        object.entry("type").or_insert(Value::String(event));
                    }
                }
                self.observe_json(&value)
            }
            Err(_) => {
                self.malformed = true;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_semantics_match_across_protocols_without_double_counting() {
        let mut a = UsageAccumulator::new(UsageProtocol::Anthropic);
        a.observe_json(&json!({"type":"message_start","message":{"usage":{"input_tokens":2000,"cache_read_input_tokens":8000,"output_tokens":0}}}));
        a.observe_json(&json!({"type":"message_delta","usage":{"output_tokens":500}}));
        a.observe_json(&json!({"type":"message_delta","usage":{"output_tokens":1000}}));
        a.observe_json(&json!({"type":"message_delta","usage":{"output_tokens":1000}}));
        let mut c = UsageAccumulator::new(UsageProtocol::Chat);
        c.observe_json(&json!({"usage":{"prompt_tokens":10000,"completion_tokens":1000,"prompt_tokens_details":{"cached_tokens":8000}}}));
        assert_eq!(a.tokens.input, c.tokens.input);
        assert_eq!(a.tokens.output, Some(1000));
        assert_eq!(a.tokens.cached, Some(8000));
        assert_eq!(a.quality(true), "complete");
    }

    #[test]
    fn missing_malformed_and_zero_are_distinct() {
        let mut c = UsageAccumulator::new(UsageProtocol::Chat);
        assert_eq!(c.quality(true), "unknown");
        c.observe_json(&json!({"usage":{"prompt_tokens":0,"completion_tokens":0}}));
        assert_eq!(c.quality(true), "complete");
        c.observe_json(&json!({"usage":{"prompt_tokens":-1}}));
        assert_eq!(c.quality(true), "partial");
        let mut e = UsageAccumulator::new(UsageProtocol::Embedding);
        e.observe_json(&json!({"usage":{"prompt_tokens":10,"total_tokens":10}}));
        assert_eq!(e.tokens.output, Some(0));
        let mut r = UsageAccumulator::new(UsageProtocol::Rerank);
        r.observe_json(&json!({"results":[]}));
        assert_eq!(r.quality(true), "unknown");
    }

    #[test]
    fn responses_nested_usage_and_stream_errors() {
        let mut r = UsageAccumulator::new(UsageProtocol::Responses);
        r.sse_line("event: response.completed");
        r.sse_line("data: {\"response\": {\"usage\":");
        r.sse_line("data: {\"input_tokens\":9,\"output_tokens\":2}}}");
        r.sse_line("");
        assert!(r.terminal);
        assert_eq!(r.tokens.input, Some(9));
        assert_eq!(r.quality(true), "complete");
        r.observe_json(&json!({"type":"error","error":{"type":"overloaded_error"}}));
        assert!(r.failed);
    }

    #[test]
    fn first_output_excludes_roles_and_usage_and_counts_tools() {
        let mut c = UsageAccumulator::new(UsageProtocol::Chat);
        assert!(!c.observe_json(&json!({"choices":[{"delta":{"role":"assistant"}}]})));
        assert!(!c.sse_line(": ping"));
        assert!(c.observe_json(
            &json!({"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"{}"}}]}}]})
        ));
        assert!(!c.terminal);
    }
}
