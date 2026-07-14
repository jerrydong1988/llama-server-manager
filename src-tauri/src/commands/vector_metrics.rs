use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorEventSource {
    Log,
    Proxy,
}

impl VectorEventSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Proxy => "proxy",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VectorTrendBucket {
    pub timestamp: i64,
    pub input_tokens_per_second: Option<f64>,
    pub items_per_second: f64,
}
