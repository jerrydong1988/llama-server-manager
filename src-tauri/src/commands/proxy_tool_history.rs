//! Validate modern Chat tool-call/result pairing without interpreting tool payloads.
//! Count endpoints and other native protocols intentionally keep their own contracts.
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ToolHistoryError {
    pub param: String,
    pub message: &'static str,
}

fn error(param: String, message: &'static str) -> Option<ToolHistoryError> {
    Some(ToolHistoryError { param, message })
}

pub(super) fn validate(path: &str, body: &[u8]) -> Option<ToolHistoryError> {
    if path != "/v1/chat/completions" {
        return None;
    }
    // Generic JSON/messages validation remains with the existing parser/upstream.
    let value: Value = serde_json::from_slice(body).ok()?;
    let messages = value.get("messages")?.as_array()?;
    let mut pending = HashSet::new();
    let mut answered = HashSet::new();
    let mut batch_index = 0;

    for (index, message) in messages.iter().enumerate() {
        if message.get("role").and_then(Value::as_str) == Some("tool") {
            let param = format!("messages[{index}].tool_call_id");
            let Some(id) = message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
            else {
                return error(
                    param,
                    "tool results require a non-empty string tool_call_id",
                );
            };
            if !pending.remove(id) {
                return error(
                    param,
                    if answered.contains(id) {
                        "a tool call must not receive duplicate results"
                    } else {
                        "tool_call_id does not match an outstanding assistant tool call"
                    },
                );
            }
            answered.insert(id);
            continue;
        }

        if !pending.is_empty() {
            return error(format!("messages[{batch_index}].tool_calls"),
                "each assistant tool call requires a matching tool result before the next non-tool message");
        }
        answered.clear();
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(calls) = message.get("tool_calls").filter(|calls| !calls.is_null()) else {
            continue;
        };
        let Some(calls) = calls.as_array() else {
            return error(
                format!("messages[{index}].tool_calls"),
                "assistant tool_calls must be an array or null",
            );
        };
        batch_index = index;
        for (call_index, call) in calls.iter().enumerate() {
            let param = format!("messages[{index}].tool_calls[{call_index}].id");
            let Some(id) = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
            else {
                return error(param, "assistant tool calls require a non-empty string id");
            };
            if !pending.insert(id) {
                return error(
                    param,
                    "tool call ids must be unique within an assistant turn",
                );
            }
        }
    }
    if !pending.is_empty() {
        return error(format!("messages[{batch_index}].tool_calls"),
            "each assistant tool call requires a matching tool result before requesting another generation");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check(messages: Value) -> Option<ToolHistoryError> {
        validate(
            "/v1/chat/completions",
            &serde_json::to_vec(&json!({"messages": messages})).unwrap(),
        )
    }

    fn call(id: Value) -> Value {
        json!({"id": id, "type":"function", "function":{"name":"historical_tool", "arguments":"{}"}})
    }

    fn assistant(ids: &[&str]) -> Value {
        json!({"role":"assistant", "content":null, "tool_calls":ids.iter().map(|id| call(json!(id))).collect::<Vec<_>>()})
    }

    fn result(id: &str) -> Value {
        json!({"role":"tool", "tool_call_id":id, "content":"{}"})
    }

    #[test]
    fn valid_parallel_results_can_arrive_out_of_order_and_include_errors() {
        assert_eq!(
            check(json!([
                {"role":"user","content":"Run tools"}, assistant(&["a", "b"]), result("b"),
                {"role":"tool","tool_call_id":"a","content":"{\"error\":\"cancelled\"}"},
                {"role":"assistant","content":"One operation was cancelled"}
            ])),
            None
        );
        // IDs are matched per turn, not globally, and historical tool names do
        // not have to appear in the current request's declarations.
        assert_eq!(
            check(json!([
                assistant(&["a"]),
                result("a"),
                assistant(&["a"]),
                result("a")
            ])),
            None
        );
    }

    #[test]
    fn missing_results_are_rejected_at_turn_boundary_and_end() {
        for tail in [
            None,
            Some(json!({"role":"user","content":"Summarize"})),
            Some(json!({"role":"assistant","content":"Invented result"})),
            Some(json!({"role":"system","content":"Continue"})),
        ] {
            let mut messages = vec![assistant(&["a", "b"]), result("b")];
            if let Some(tail) = tail {
                messages.push(tail);
            }
            let failure = check(json!(messages)).unwrap();
            assert_eq!(failure.param, "messages[0].tool_calls");
            assert!(failure.message.contains("matching tool result"));
        }
    }

    #[test]
    fn orphan_mismatched_and_duplicate_results_are_rejected() {
        for (messages, index) in [
            (json!([result("a")]), 0),
            (json!([assistant(&["a"]), result("wrong")]), 1),
            (json!([assistant(&["a", "b"]), result("a"), result("a")]), 2),
            (json!([assistant(&["a"]), result("a"), result("a")]), 2),
        ] {
            assert_eq!(
                check(messages).unwrap().param,
                format!("messages[{index}].tool_call_id")
            );
        }
    }

    #[test]
    fn call_ids_must_be_nonempty_strings_unique_within_the_turn() {
        for id in [Value::Null, json!(42), json!(""), json!("  ")] {
            assert_eq!(
                check(json!([{"role":"assistant","tool_calls":[call(id.clone())]}]))
                    .unwrap()
                    .param,
                "messages[0].tool_calls[0].id"
            );
            assert_eq!(
                check(json!([assistant(&["a"]), {"role":"tool","tool_call_id":id,"content":""}]))
                    .unwrap()
                    .param,
                "messages[1].tool_call_id"
            );
        }
        assert_eq!(
            check(json!([assistant(&["a", "a"])])).unwrap().param,
            "messages[0].tool_calls[1].id"
        );
        assert_eq!(
            check(json!([{"role":"assistant","tool_calls":{}}]))
                .unwrap()
                .param,
            "messages[0].tool_calls"
        );
    }

    #[test]
    fn ordinary_prefill_legacy_functions_and_custom_tool_payloads_are_not_reinterpreted() {
        for messages in [
            json!([{"role":"user","content":"Hi"},{"role":"assistant","content":"Prefix"}]),
            json!([{"role":"assistant","tool_calls":null},{"role":"assistant","tool_calls":[]}]),
            json!([{"role":"assistant","function_call":{"name":"old_tool","arguments":"{}"}},
                {"role":"function","name":"old_tool","content":"legacy result"}]),
            json!([{"role":"assistant","tool_calls":[{"id":"custom-1","type":"custom","custom":{"name":"grammar_tool","input":"raw input"}}]}, result("custom-1")]),
        ] {
            assert_eq!(check(messages), None);
        }
        // Content that merely mentions the protocol is not treated as history.
        assert_eq!(
            check(json!([{"role":"user","content":"{\"tool_calls\":[{\"id\":\"unfinished\"}]}"}])),
            None
        );
    }

    #[test]
    fn counts_other_protocols_and_generic_json_errors_keep_their_existing_contracts() {
        let body = serde_json::to_vec(&json!({"messages":[assistant(&["pending"])]})).unwrap();
        for path in [
            "/v1/chat/completions/input_tokens",
            "/v1/responses",
            "/v1/responses/input_tokens",
            "/v1/messages",
            "/v1/messages/count_tokens",
            "/v1/completions",
            "/v1/embeddings",
        ] {
            assert_eq!(validate(path, &body), None);
        }
        for body in [b"not json".as_slice(), b"{}", b"{\"messages\":false}"] {
            assert_eq!(validate("/v1/chat/completions", body), None);
        }
    }

    #[test]
    fn diagnostics_never_echo_tool_payloads_or_arbitrary_ids() {
        let failure = check(json!([assistant(&["secret-id"]),
            {"role":"tool","tool_call_id":"private-other-id","content":"secret-result"}]))
        .unwrap();
        let diagnostic = format!("{failure:?}");
        for secret in ["secret-id", "private-other-id", "secret-result"] {
            assert!(!diagnostic.contains(secret));
        }
    }
}
