//! Versioned local management contract shared by the desktop host and console client.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

pub fn data_identity(data: &Path) -> String {
    let canonical = std::fs::canonicalize(data).unwrap_or_else(|_| data.to_path_buf());
    #[cfg(windows)]
    {
        canonical
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase()
    }
    #[cfg(not(windows))]
    {
        canonical.to_string_lossy().into_owned()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: String,
    pub token: String,
    pub operation: String,
    #[serde(default = "empty_object")]
    pub arguments: Value,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

pub fn empty_object() -> Value {
    serde_json::json!({})
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl Failure {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        let code = if message.starts_with("CONFIG_CONFLICT:") {
            "CONFIG_CONFLICT"
        } else {
            "OPERATION_FAILED"
        };
        Self::new(code, message)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
}

impl Response {
    pub fn result(id: String, result: Result<Value, Failure>) -> Self {
        match result {
            Ok(data) => Self {
                version: VERSION,
                id,
                ok: true,
                data: Some(data),
                error: None,
            },
            Err(error) => Self {
                version: VERSION,
                id,
                ok: false,
                data: None,
                error: Some(error),
            },
        }
    }
}

pub fn default_data_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(root).join("LlamaServerManager");
    }
    #[cfg(target_os = "macos")]
    if let Some(root) = std::env::var_os("HOME") {
        return PathBuf::from(root).join("Library/Application Support/LlamaServerManager");
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(root) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(root).join("LlamaServerManager");
        }
        if let Some(root) = std::env::var_os("HOME") {
            return PathBuf::from(root).join(".local/share/LlamaServerManager");
        }
    }
    PathBuf::from(".").join("LlamaServerManager")
}

pub fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn token_path(data: &Path) -> PathBuf {
    data.join("management/control-token")
}

pub fn secret_equal(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    for (a, b) in left.as_bytes().iter().zip(right.as_bytes()) {
        difference |= usize::from(a ^ b);
    }
    difference == 0
}

/// Applied to responses and task records, never to the live configuration.
pub fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                let key = key.to_ascii_lowercase();
                if key == "key"
                    || key.contains("password")
                    || key.contains("secret")
                    || key == "api_key"
                    || key.ends_with("_api_key")
                    || key == "api_key_file"
                    || key == "hf_token"
                    || key == "token"
                    || key == "authorization"
                    || key == "command_display"
                    || key == "manual_command"
                    || key == "custom_args"
                {
                    if !item.is_null() && item != "" {
                        *item = Value::String("[REDACTED]".into());
                    }
                } else if key == "command" {
                    if let Value::Array(arguments) = item {
                        let mut sensitive = false;
                        for argument in arguments {
                            if sensitive {
                                *argument = Value::String("[REDACTED]".into());
                                sensitive = false;
                                continue;
                            }
                            if let Some(text) = argument.as_str() {
                                if [
                                    "--api-key",
                                    "--api-key-file",
                                    "--hf-token",
                                    "-hft",
                                    "--connect-code",
                                ]
                                .contains(&text)
                                {
                                    sensitive = true;
                                } else if [
                                    "--api-key=",
                                    "--api-key-file=",
                                    "--hf-token=",
                                    "-hft=",
                                    "--connect-code=",
                                ]
                                .iter()
                                .any(|prefix| text.starts_with(prefix))
                                {
                                    *argument = Value::String("[REDACTED]".into());
                                }
                            }
                        }
                    } else {
                        *item = Value::String("[REDACTED]".into());
                    }
                } else {
                    redact(item);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_redacted_recursively_without_changing_input() {
        let mut value = serde_json::json!({"api_key":"private","nested":[{"key":"private","tokens":42}],"empty":""});
        redact(&mut value);
        assert_eq!(value["api_key"], "[REDACTED]");
        assert_eq!(value["nested"][0]["key"], "[REDACTED]");
        assert_eq!(value["nested"][0]["tokens"], 42);
        assert!(secret_equal("abc", "abc"));
        assert!(!secret_equal("abc", "abcd"));
    }

    #[test]
    fn redaction_retains_key_metadata_and_masks_command_arguments() {
        let mut value = serde_json::json!({"api_keys":[{"id":"a","key":"private","tokens":10}],"command":["--api-key","private","--hf-token=private","--port","8080","--connect-code","private","--connect-code=private"]});
        redact(&mut value);
        assert_eq!(value["api_keys"][0]["id"], "a");
        assert_eq!(value["api_keys"][0]["key"], "[REDACTED]");
        assert_eq!(value["command"][4], "8080");
        assert!(!value.to_string().contains("private"));
    }
}
