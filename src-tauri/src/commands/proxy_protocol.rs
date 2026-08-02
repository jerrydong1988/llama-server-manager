use axum::body::Bytes;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

const ANTHROPIC_FORMAT_HEADER: &str = "x-llama-server-manager-api-format";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyApiFormat {
    OpenAi,
    Anthropic,
}

impl ProxyApiFormat {
    pub(crate) fn from_path(path: &str) -> Self {
        if matches!(path, "/v1/messages" | "/v1/messages/count_tokens") {
            Self::Anthropic
        } else {
            Self::OpenAi
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    pub(crate) fn is_anthropic(self) -> bool {
        self == Self::Anthropic
    }
}

pub(crate) fn request_format(path: &str, has_anthropic_version: bool) -> ProxyApiFormat {
    let path_format = ProxyApiFormat::from_path(path);
    if path_format.is_anthropic() || has_anthropic_version {
        ProxyApiFormat::Anthropic
    } else {
        ProxyApiFormat::OpenAi
    }
}

fn anthropic_error_type(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST | StatusCode::METHOD_NOT_ALLOWED => "invalid_request_error",
        StatusCode::PAYLOAD_TOO_LARGE => "request_too_large",
        StatusCode::UNAUTHORIZED => "authentication_error",
        StatusCode::FORBIDDEN => "permission_error",
        StatusCode::NOT_FOUND => "not_found_error",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        status if status.as_u16() == 529 => "overloaded_error",
        _ => "api_error",
    }
}

fn openai_error_type(status: StatusCode) -> &'static str {
    match status {
        StatusCode::UNAUTHORIZED => "authentication_error",
        StatusCode::FORBIDDEN => "permission_error",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        status if status.is_client_error() => "invalid_request_error",
        _ => "server_error",
    }
}

pub(crate) fn proxy_request_id() -> String {
    format!("req_{}", uuid::Uuid::new_v4().simple())
}

fn anthropic_error_value(status: StatusCode, message: &str, request_id: &str) -> Value {
    json!({
        "type": "error",
        "error": {
            "type": anthropic_error_type(status),
            "message": message,
        },
        "request_id": request_id,
    })
}

pub(crate) fn error_response(
    format: ProxyApiFormat,
    status: StatusCode,
    message: &str,
) -> Response {
    let request_id = proxy_request_id();
    let value = match format {
        ProxyApiFormat::OpenAi => json!({
            "error": {
                "message": message,
                "type": openai_error_type(status),
                "param": Value::Null,
                "code": Value::Null,
            }
        }),
        ProxyApiFormat::Anthropic => anthropic_error_value(status, message, &request_id),
    };
    let mut response = (status, Json(value)).into_response();
    if let Ok(header_value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(
            if format.is_anthropic() {
                "request-id"
            } else {
                "x-request-id"
            },
            header_value,
        );
    }
    add_format_header(&mut response, format);
    response
}

fn upstream_error_message(value: &Value) -> Option<&str> {
    value
        .get("error")
        .and_then(|error| match error {
            Value::String(message) => Some(message.as_str()),
            Value::Object(object) => object.get("message").and_then(Value::as_str),
            _ => None,
        })
        .or_else(|| value.get("message").and_then(Value::as_str))
}

fn is_anthropic_error(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("error")
        && value
            .get("error")
            .and_then(Value::as_object)
            .is_some_and(|error| {
                error.get("type").and_then(Value::as_str).is_some()
                    && error.get("message").and_then(Value::as_str).is_some()
            })
}

fn rewrite_known_model_fields(value: &mut Value, public_model_id: &str, format: ProxyApiFormat) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.contains_key("model") {
        object.insert(
            "model".to_string(),
            Value::String(public_model_id.to_string()),
        );
    }
    if format.is_anthropic() {
        if let Some(message) = object.get_mut("message").and_then(Value::as_object_mut) {
            if message.contains_key("model") {
                message.insert(
                    "model".to_string(),
                    Value::String(public_model_id.to_string()),
                );
            }
        }
    } else if let Some(response) = object.get_mut("response").and_then(Value::as_object_mut) {
        if response.contains_key("model") {
            response.insert(
                "model".to_string(),
                Value::String(public_model_id.to_string()),
            );
        }
    }
}

pub(crate) fn rewrite_json_response(
    body: Bytes,
    public_model_id: &str,
    format: ProxyApiFormat,
    status: StatusCode,
) -> Bytes {
    let parsed = serde_json::from_slice::<Value>(&body);
    if format.is_anthropic() && !status.is_success() {
        let message = parsed
            .as_ref()
            .ok()
            .and_then(upstream_error_message)
            .map(str::to_string)
            .or_else(|| {
                let message = String::from_utf8_lossy(&body).trim().to_string();
                (!message.is_empty()).then_some(message)
            })
            .unwrap_or_else(|| {
                status
                    .canonical_reason()
                    .unwrap_or("upstream request failed")
                    .to_string()
            });
        if let Ok(mut value) = parsed {
            if is_anthropic_error(&value) {
                rewrite_known_model_fields(&mut value, public_model_id, format);
                if value.get("request_id").and_then(Value::as_str).is_none() {
                    if let Some(object) = value.as_object_mut() {
                        object.insert("request_id".to_string(), Value::String(proxy_request_id()));
                    }
                }
                return serde_json::to_vec(&value).map(Bytes::from).unwrap_or(body);
            }
        }
        let request_id = proxy_request_id();
        return serde_json::to_vec(&anthropic_error_value(status, &message, &request_id))
            .map(Bytes::from)
            .unwrap_or(body);
    }

    if !status.is_success() {
        let message = parsed
            .as_ref()
            .ok()
            .and_then(upstream_error_message)
            .map(str::to_string)
            .or_else(|| {
                let message = String::from_utf8_lossy(&body).trim().to_string();
                (!message.is_empty()).then_some(message)
            })
            .unwrap_or_else(|| {
                status
                    .canonical_reason()
                    .unwrap_or("upstream request failed")
                    .to_string()
            });
        if let Ok(mut value) = parsed {
            if let Some(error) = value.get_mut("error").and_then(Value::as_object_mut) {
                if error.get("message").and_then(Value::as_str).is_some() {
                    error
                        .entry("type")
                        .or_insert_with(|| Value::String(openai_error_type(status).to_string()));
                    error.entry("param").or_insert(Value::Null);
                    error.entry("code").or_insert(Value::Null);
                }
            }
            let normalized = value
                .get("error")
                .and_then(Value::as_object)
                .is_some_and(|error| error.get("message").and_then(Value::as_str).is_some());
            if normalized {
                rewrite_known_model_fields(&mut value, public_model_id, format);
                return serde_json::to_vec(&value).map(Bytes::from).unwrap_or(body);
            }
        }
        return serde_json::to_vec(&json!({
            "error": {
                "message": message,
                "type": openai_error_type(status),
                "param": Value::Null,
                "code": Value::Null,
            }
        }))
        .map(Bytes::from)
        .unwrap_or(body);
    }

    let Ok(mut value) = parsed else {
        return body;
    };
    rewrite_known_model_fields(&mut value, public_model_id, format);
    serde_json::to_vec(&value).map(Bytes::from).unwrap_or(body)
}

pub(crate) fn rewrite_sse_line(
    line: &str,
    public_model_id: &str,
    format: ProxyApiFormat,
) -> String {
    let Some(payload) = line.strip_prefix("data:") else {
        return line.to_string();
    };
    let payload = payload.trim_start();
    if payload.is_empty() || payload == "[DONE]" {
        return line.to_string();
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(payload.as_bytes()) else {
        return line.to_string();
    };
    rewrite_known_model_fields(&mut value, public_model_id, format);
    let Ok(rewritten) = serde_json::to_string(&value) else {
        return line.to_string();
    };
    format!("data: {rewritten}")
}

pub(crate) fn add_format_header(response: &mut Response, format: ProxyApiFormat) {
    if format.is_anthropic() {
        response.headers_mut().insert(
            ANTHROPIC_FORMAT_HEADER,
            HeaderValue::from_static("anthropic"),
        );
    }
}

pub(crate) fn ensure_request_id_header(response: &mut Response, format: ProxyApiFormat) {
    let name = if format.is_anthropic() {
        "request-id"
    } else {
        "x-request-id"
    };
    if response.headers().contains_key(name) {
        return;
    }
    if let Ok(value) = HeaderValue::from_str(&proxy_request_id()) {
        response.headers_mut().insert(name, value);
    }
}

pub(crate) fn response_request_id(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("request_id")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_errors_use_the_documented_envelope() {
        let response = error_response(
            ProxyApiFormat::Anthropic,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        );
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(ANTHROPIC_FORMAT_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some("anthropic")
        );
        assert!(response.headers().contains_key("request-id"));
    }

    #[test]
    fn anthropic_message_start_rewrites_only_protocol_model_fields() {
        let line = rewrite_sse_line(
            r#"data: {"type":"message_start","message":{"model":"private-model"},"input":{"model":"tool-input"}}"#,
            "public-model",
            ProxyApiFormat::Anthropic,
        );
        let value: Value = serde_json::from_str(line.strip_prefix("data: ").unwrap()).unwrap();
        assert_eq!(value["message"]["model"], "public-model");
        assert_eq!(value["input"]["model"], "tool-input");
    }

    #[test]
    fn upstream_openai_error_is_normalized_for_anthropic_clients() {
        let body = rewrite_json_response(
            Bytes::from_static(br#"{"error":{"message":"unsupported endpoint"}}"#),
            "public-model",
            ProxyApiFormat::Anthropic,
            StatusCode::NOT_FOUND,
        );
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["type"], "error");
        assert_eq!(value["error"]["type"], "not_found_error");
        assert_eq!(value["error"]["message"], "unsupported endpoint");
        assert!(value["request_id"]
            .as_str()
            .is_some_and(|request_id| request_id.starts_with("req_")));
    }

    #[test]
    fn responses_stream_rewrites_nested_response_model_only() {
        let line = rewrite_sse_line(
            r#"data: {"type":"response.created","response":{"model":"private-model"},"input":{"model":"tool-input"}}"#,
            "public-model",
            ProxyApiFormat::OpenAi,
        );
        let value: Value = serde_json::from_str(line.strip_prefix("data: ").unwrap()).unwrap();
        assert_eq!(value["response"]["model"], "public-model");
        assert_eq!(value["input"]["model"], "tool-input");
    }

    #[test]
    fn nonstandard_openai_upstream_errors_use_the_sdk_envelope() {
        let body = rewrite_json_response(
            Bytes::from_static(br#"{"message":"backend overloaded"}"#),
            "public-model",
            ProxyApiFormat::OpenAi,
            StatusCode::SERVICE_UNAVAILABLE,
        );
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["message"], "backend overloaded");
        assert_eq!(value["error"]["type"], "server_error");
        assert!(value["error"]["param"].is_null());
        assert!(value["error"]["code"].is_null());
    }

    #[test]
    fn partial_openai_error_objects_receive_all_sdk_fields() {
        let body = rewrite_json_response(
            Bytes::from_static(br#"{"error":{"message":"bad request"}}"#),
            "public-model",
            ProxyApiFormat::OpenAi,
            StatusCode::BAD_REQUEST,
        );
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["message"], "bad request");
        assert_eq!(value["error"]["type"], "invalid_request_error");
        assert!(value["error"]["param"].is_null());
        assert!(value["error"]["code"].is_null());
    }

    #[test]
    fn openai_errors_always_include_a_request_id_header() {
        let response = error_response(
            ProxyApiFormat::OpenAi,
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded",
        );
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().contains_key("x-request-id"));
    }

    #[tokio::test]
    async fn payload_limit_uses_anthropic_request_too_large_error() {
        let response = error_response(
            ProxyApiFormat::Anthropic,
            StatusCode::PAYLOAD_TOO_LARGE,
            "request too large",
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["type"], "request_too_large");
        assert!(response_request_id(&body).is_some());
    }
}
