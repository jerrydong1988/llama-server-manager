use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, String>,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            context: BTreeMap::new(),
        }
    }

    pub fn from_legacy(message: impl Into<String>) -> Self {
        let message = message.into();
        let normalized = message.to_ascii_lowercase();
        let has_term = |term: &str| {
            normalized.match_indices(term).any(|(start, matched)| {
                let end = start + matched.len();
                let before = normalized[..start].chars().next_back();
                let after = normalized[end..].chars().next();
                before.map_or(true, |ch| !ch.is_ascii_alphanumeric() && ch != '_')
                    && after.map_or(true, |ch| !ch.is_ascii_alphanumeric() && ch != '_')
            })
        };
        let (code, retryable) = if has_term("already")
            || has_term("conflict")
            || message.contains("已在")
            || message.contains("冲突")
        {
            ("CONFLICT", false)
        } else if normalized.contains("not found") || message.contains("未找到") {
            ("NOT_FOUND", false)
        } else if has_term("timeout")
            || normalized.contains("timed out")
            || message.contains("超时")
        {
            ("TIMEOUT", true)
        } else if has_term("connect")
            || has_term("connection")
            || has_term("network")
            || message.contains("网络")
            || message.contains("连接")
        {
            ("NETWORK", true)
        } else if has_term("invalid")
            || has_term("required")
            || message.contains("无效")
            || message.contains("必须")
        {
            ("VALIDATION", false)
        } else if has_term("permission")
            || has_term("disk")
            || has_term("file")
            || message.contains("文件")
            || message.contains("磁盘")
        {
            ("IO", true)
        } else {
            ("INTERNAL", false)
        };
        Self::new(code, message, retryable)
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }
}

impl Display for AppError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::from_legacy(message)
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::new("IO", error.to_string(), true)
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_errors_receive_stable_codes_and_retry_policy() {
        let timeout = AppError::from_legacy("request timeout");
        assert_eq!(timeout.code, "TIMEOUT");
        assert!(timeout.retryable);

        let validation = AppError::from_legacy("invalid port");
        assert_eq!(validation.code, "VALIDATION");
        assert!(!validation.retryable);

        let unrelated = AppError::from_legacy("profile metadata could not be loaded");
        assert_eq!(unrelated.code, "INTERNAL");

        let file_error = AppError::from_legacy("file could not be opened");
        assert_eq!(file_error.code, "IO");
    }

    #[test]
    fn app_error_serializes_as_an_ipc_object() {
        let error =
            AppError::new("CONFLICT", "already running", false).with_context("instanceId", "demo");
        let value = serde_json::to_value(error).unwrap();
        assert_eq!(value["code"], "CONFLICT");
        assert_eq!(value["context"]["instanceId"], "demo");
    }
}
