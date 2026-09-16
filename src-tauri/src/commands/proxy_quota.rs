//! Quota preflight runs only for explicitly limited keys and never infers unknown usage as zero.
use super::*;
use crate::commands::usage_quota::{self, QuotaError};

fn output_limit(path: &str, value: &serde_json::Value) -> Result<u64, QuotaError> {
    for field in ["n_predict", "max_new_tokens", "best_of"] {
        if value.get(field).is_some() {
            return Err(QuotaError::InvalidLimit);
        }
    }
    if value.get("n").is_some_and(|n| n.as_u64() != Some(1)) {
        return Err(QuotaError::InvalidLimit);
    }
    let fields: &[&str] = match path {
        "/v1/chat/completions" => &["max_completion_tokens", "max_tokens"],
        "/v1/responses" => &["max_output_tokens"],
        "/v1/messages" | "/v1/completions" => &["max_tokens"],
        _ => return Err(QuotaError::Unmetered),
    };
    if !fields.iter().any(|name| value.get(*name).is_some()) {
        return Err(QuotaError::InvalidLimit);
    }
    // Reserve the largest alias as endpoint conversions may preserve both names.
    let mut maximum = 0;
    for name in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
        if let Some(limit) = value.get(name) {
            let n = limit
                .as_u64()
                .filter(|n| *n > 0 && *n <= i32::MAX as u64)
                .ok_or(QuotaError::InvalidLimit)?;
            maximum = maximum.max(n);
        }
    }
    if path == "/v1/completions" {
        let prompt = value.get("prompt").ok_or(QuotaError::Unmetered)?;
        if !prompt.is_string()
            && !prompt
                .as_array()
                .is_some_and(|a| !a.is_empty() && a.iter().all(|n| n.as_u64().is_some()))
        {
            return Err(QuotaError::Unmetered);
        }
    }
    Ok(maximum)
}

fn vector_items(path: &str, value: &serde_json::Value) -> Result<u64, QuotaError> {
    let rerank = matches!(
        path,
        "/rerank" | "/reranking" | "/v1/rerank" | "/v1/reranking"
    );
    let input = if rerank {
        value.get("documents").or_else(|| value.get("texts"))
    } else {
        value.get("input").or_else(|| value.get("content"))
    }
    .ok_or(QuotaError::Unmetered)?;
    let count = match input.as_array() {
        Some(a) if !rerank && a.first().is_some_and(serde_json::Value::is_number) => 1,
        Some(a) => a.len(),
        None if !rerank && input.is_string() => 1,
        _ => return Err(QuotaError::Unmetered),
    };
    if count == 0 || count > 4096 {
        return Err(QuotaError::Unmetered);
    }
    Ok(count as u64)
}

pub(super) async fn admit(
    usage: &UsageHandle,
    config: &ProxyConfig,
    target: &ResolvedProxyTarget,
    client: &reqwest::Client,
    headers: &HeaderMap,
    path: &str,
    body: &Bytes,
) -> Result<(), QuotaError> {
    if matches!(
        path,
        "/v1/messages/count_tokens"
            | "/v1/chat/completions/input_tokens"
            | "/v1/responses/input_tokens"
    ) {
        return Ok(());
    }
    let (key_id, id) = usage.quota_identity();
    let Some(key) = config
        .api_keys
        .iter()
        .find(|key| key.id == key_id && usage_quota::enabled(key))
    else {
        return Ok(());
    };
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| QuotaError::Unmetered)?;
    let amount = if let Some(spec) = context_preflight_spec(path, body) {
        let output = output_limit(path, &value)?;
        let input = fetch_input_token_count(client, target, headers, body, spec.counter, config)
            .await
            .ok_or(QuotaError::Unmetered)?;
        input.checked_add(output).ok_or(QuotaError::Unmetered)?
    } else {
        let items = vector_items(path, &value)?;
        // A fresh engine property, not a configured guess or the smallest cached slot.
        let props = fetch_target_json(target, "/props", config)
            .await
            .map_err(|_| QuotaError::Unmetered)?;
        let context = props
            .pointer("/default_generation_settings/n_ctx")
            .and_then(serde_json::Value::as_u64)
            .filter(|n| *n > 0)
            .ok_or(QuotaError::Unmetered)?;
        context.checked_mul(items).ok_or(QuotaError::Unmetered)?
    };
    usage.quota(usage_quota::reserve(key.clone(), id, amount).await?);
    Ok(())
}

pub(super) fn response(format: ProxyApiFormat, error: QuotaError) -> Response {
    let status = match error {
        QuotaError::Exceeded => StatusCode::TOO_MANY_REQUESTS,
        QuotaError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_REQUEST,
    };
    super::super::proxy_protocol::quota_error_response(
        format,
        status,
        error.code(),
        error.message(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overrides_unbounded_and_multiple_generations_fail_closed() {
        for body in [
            json!({}),
            json!({"max_tokens":0}),
            json!({"max_tokens":-1}),
            json!({"max_tokens":1.5}),
            json!({"max_tokens":10,"n_predict":-1}),
            json!({"max_tokens":10,"n":2}),
            json!({"max_tokens":10,"best_of":3}),
        ] {
            assert!(output_limit("/v1/chat/completions", &body).is_err());
        }
        assert_eq!(
            output_limit(
                "/v1/chat/completions",
                &json!({"max_tokens":10,"max_completion_tokens":30})
            )
            .unwrap(),
            30
        );
        assert!(output_limit(
            "/v1/completions",
            &json!({"max_tokens":10,"prompt":["a","b"]})
        )
        .is_err());
    }
    #[test]
    fn vector_batches_count_all_work_not_top_n() {
        assert_eq!(
            vector_items("/v1/embeddings", &json!({"input":[1,2,3]})).unwrap(),
            1
        );
        assert_eq!(
            vector_items("/v1/embeddings", &json!({"input":["a","b"]})).unwrap(),
            2
        );
        assert_eq!(
            vector_items("/v1/rerank", &json!({"documents":["a","b","c"],"top_n":1})).unwrap(),
            3
        );
        assert_eq!(
            vector_items("/rerank", &json!({"texts":["a","b"]})).unwrap(),
            2
        );
    }
}
