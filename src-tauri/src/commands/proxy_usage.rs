use super::usage_protocol::{TokenUsage, UsageAccumulator, UsageProtocol};
use axum::{
    body::{Body, Bytes},
    response::Response,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageRecord {
    pub request_id: String,
    pub key_id: String,
    pub key_name: String,
    pub model: String,
    pub instance_id: String,
    pub endpoint: String,
    pub kind: String,
    pub started_at: i64,
    pub completed_at: i64,
    pub http_status: u16,
    pub forwarded: bool,
    pub outcome: String,
    pub quality: String,
    pub source: String,
    pub tokens: TokenUsage,
    pub duration_ms: u64,
    pub queue_ms: u64,
    pub first_output_ms: Option<u64>,
    pub finish_reason: Option<String>,
    pub items: Option<u64>,
}

struct Observation {
    record: UsageRecord,
    parser: UsageAccumulator,
    started: Instant,
    streaming: bool,
    finished: bool,
    recorded: bool,
}

impl Observation {
    fn finalize(&mut self) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        self.parser.flush_event();
        self.record.completed_at = super::telemetry::current_time_ms();
        self.record.duration_ms = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.record.outcome = if !self.finished {
            "cancelled"
        } else if self.record.http_status >= 400 && !self.record.forwarded {
            "rejected"
        } else if self.record.http_status >= 400 || self.parser.failed {
            "failed"
        } else if self.streaming && !self.parser.terminal {
            "incomplete"
        } else {
            "success"
        }
        .into();
        self.record.tokens = self.parser.tokens.clone();
        self.record.quality = self.parser.quality(self.record.forwarded).into();
        if self.streaming && !self.parser.terminal && self.record.quality == "complete" {
            self.record.quality = "partial".into();
        }
        self.record.source = self.parser.source.into();
        self.record.finish_reason = self.parser.finish_reason.clone();
        super::usage_store::record(self.record.clone());
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        self.finalize();
    }
}

#[derive(Clone)]
pub(crate) struct UsageHandle(Arc<Mutex<Observation>>);

impl UsageHandle {
    pub fn new(endpoint: &str) -> Option<Self> {
        let protocol = UsageProtocol::from_path(endpoint)?;
        let started_at = super::telemetry::current_time_ms();
        Some(Self(Arc::new(Mutex::new(Observation {
            record: UsageRecord {
                request_id: format!("usage_{}", uuid::Uuid::new_v4().simple()),
                key_id: "unauthenticated".into(),
                key_name: "unauthenticated".into(),
                model: String::new(),
                instance_id: String::new(),
                endpoint: endpoint.into(),
                kind: protocol.kind().into(),
                started_at,
                completed_at: started_at,
                http_status: 0,
                forwarded: false,
                outcome: String::new(),
                quality: "unknown".into(),
                source: "none".into(),
                tokens: TokenUsage::default(),
                duration_ms: 0,
                queue_ms: 0,
                first_output_ms: None,
                finish_reason: None,
                items: None,
            },
            parser: UsageAccumulator::new(protocol),
            started: Instant::now(),
            streaming: false,
            finished: false,
            recorded: false,
        }))))
    }

    pub fn identity(&self, id: &str, name: &str) {
        let mut state = self.0.lock().unwrap();
        state.record.key_id = id.into();
        state.record.key_name = name.chars().take(256).collect();
    }
    pub fn request(&self, body: &[u8]) {
        if let Ok(value) = serde_json::from_slice::<Value>(body) {
            let mut s = self.0.lock().unwrap();
            s.record.model = value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(512)
                .collect();
            let items = if s.parser.protocol == UsageProtocol::Embedding {
                value.get("input").or_else(|| value.get("content"))
            } else if s.parser.protocol == UsageProtocol::Rerank {
                value.get("documents")
            } else {
                None
            };
            s.record.items = items.map(|v| match v.as_array() {
                Some(a)
                    if s.parser.protocol == UsageProtocol::Embedding
                        && a.first().is_some_and(Value::is_number) =>
                {
                    1
                }
                Some(a) => a.len() as u64,
                None => 1,
            });
        }
    }
    pub fn target(&self, instance: &str, public_model: &str) {
        let mut s = self.0.lock().unwrap();
        s.record.instance_id = instance.into();
        // Never persist upstream file paths or undeclared client model selectors.
        s.record.model = public_model.chars().take(512).collect();
    }
    pub fn queue(&self, ms: u64) {
        self.0.lock().unwrap().record.queue_ms = ms;
    }
    pub fn forwarded(&self) {
        self.0.lock().unwrap().record.forwarded = true;
    }
    pub fn start_stream(&self) {
        self.0.lock().unwrap().streaming = true;
    }
    pub fn json(&self, body: &[u8]) {
        let mut s = self.0.lock().unwrap();
        if let Ok(value) = serde_json::from_slice::<Value>(body) {
            s.parser.observe_json(&value);
        } else {
            s.parser.malformed = true;
        }
    }
    pub fn sse(&self, line: &str) {
        let mut s = self.0.lock().unwrap();
        s.streaming = true;
        if s.parser.sse_line(line) && s.record.first_output_ms.is_none() {
            s.record.first_output_ms =
                Some(s.started.elapsed().as_millis().min(u64::MAX as u128) as u64);
        }
    }
    pub fn finish(&self, status: u16, success: bool) {
        let mut s = self.0.lock().unwrap();
        s.record.http_status = status;
        s.finished = true;
        if !success {
            s.parser.failed = true;
        }
        s.finalize();
    }
    pub fn wrap(self, response: Response) -> Response {
        let (parts, body) = response.into_parts();
        let status = parts.status.as_u16();
        self.0.lock().unwrap().record.http_status = status;
        let stream = futures_util::stream::unfold(
            (body.into_data_stream(), self, false),
            move |(mut stream, usage, ended)| async move {
                if ended {
                    return None;
                }
                match stream.next().await {
                    Some(Ok(bytes)) => Some((Ok(bytes), (stream, usage, false))),
                    Some(Err(e)) => {
                        usage.finish(status, false);
                        Some((Err(e), (stream, usage, true)))
                    }
                    None => {
                        usage.finish(status, true);
                        None
                    }
                }
            },
        );
        Response::from_parts(parts, Body::from_stream(stream))
    }
}

/// Request usage only on llama.cpp's supported Chat streaming contract. Filtering
/// the added usage-only frame preserves clients that did not opt in to it.
pub(crate) fn request_stream_usage(body: Bytes, path: &str) -> (Bytes, bool) {
    if path != "/v1/chat/completions" {
        return (body, false);
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&body) else {
        return (body, false);
    };
    if value.get("stream").and_then(Value::as_bool) != Some(true) {
        return (body, false);
    }
    if value
        .pointer("/stream_options/include_usage")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return (body, false);
    }
    let Some(object) = value.as_object_mut() else {
        return (body, false);
    };
    let options = object
        .entry("stream_options")
        .or_insert_with(|| serde_json::json!({}));
    if options.is_null() {
        *options = serde_json::json!({});
    }
    let Some(options) = options.as_object_mut() else {
        return (body, false);
    };
    options.insert("include_usage".into(), Value::Bool(true));
    (
        serde_json::to_vec(&value).map(Bytes::from).unwrap_or(body),
        true,
    )
}

pub(crate) fn is_usage_only_line(line: &str) -> bool {
    let Some(data) = line.strip_prefix("data:") else {
        return false;
    };
    serde_json::from_str::<Value>(data.trim())
        .ok()
        .is_some_and(|v| {
            v.get("usage").is_some_and(Value::is_object)
                && v.get("choices")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_retains_partial_usage_and_identity_once() {
        let key = uuid::Uuid::new_v4().to_string();
        let h = UsageHandle::new("/v1/messages").unwrap();
        h.identity(&key, "Application");
        h.forwarded();
        h.sse("data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}");
        h.sse("");
        let second = h.clone();
        drop(h);
        drop(second);
        let records = super::super::usage_store::TEST_RECORDS.lock().unwrap();
        let matching = records
            .iter()
            .filter(|r| r.key_id == key)
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].outcome, "cancelled");
        assert_eq!(matching[0].quality, "partial");
        assert_eq!(matching[0].tokens.input, Some(5));
    }
    #[test]
    fn injected_usage_preserves_other_stream_options() {
        let (body, hide) = request_stream_usage(
            Bytes::from_static(
                br#"{"stream":true,"stream_options":{"include_usage":false,"other":42}}"#,
            ),
            "/v1/chat/completions",
        );
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert!(hide);
        assert_eq!(value["stream_options"]["other"], 42);
        assert_eq!(value["stream_options"]["include_usage"], true);
        assert!(is_usage_only_line(
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5}}"
        ));
        assert!(!is_usage_only_line("data: [DONE]"));
    }
    #[test]
    fn explicit_terminal_and_transport_outcome_are_independent() {
        let key = uuid::Uuid::new_v4().to_string();
        let h = UsageHandle::new("/v1/chat/completions").unwrap();
        h.identity(&key, "app");
        h.forwarded();
        h.sse("data: {\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}");
        h.sse("");
        h.sse("data: [DONE]");
        h.sse("");
        h.finish(200, true);
        h.finish(200, true);
        drop(h);
        let records = super::super::usage_store::TEST_RECORDS.lock().unwrap();
        let found = records
            .iter()
            .filter(|r| r.key_id == key)
            .collect::<Vec<_>>();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].outcome, "success");
        assert_eq!(found[0].quality, "complete");
    }
}
