//! Quota preflight runs only for explicitly limited keys and never infers unknown usage as zero.
use super::*;
use crate::commands::usage_quota::{self, QuotaError};

#[path = "proxy_quota_request.rs"]
mod request;

pub(super) struct Prepared {
    pub body: Bytes,
    pub context_check: Option<ContextLimitViolation>,
    pub reservation: Option<(crate::models::ProxyApiKey, u64)>,
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

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare(
    usage: &UsageHandle,
    config: &ProxyConfig,
    target: &ResolvedProxyTarget,
    client: &reqwest::Client,
    headers: &HeaderMap,
    path: &str,
    body: Bytes,
    context_window: Option<u64>,
) -> Result<Prepared, QuotaError> {
    let mut prepared = Prepared {
        body,
        context_check: None,
        reservation: None,
    };
    if matches!(
        path,
        "/v1/messages/count_tokens"
            | "/v1/chat/completions/input_tokens"
            | "/v1/responses/input_tokens"
    ) {
        return Ok(prepared);
    }
    let (key_id, _) = usage.quota_identity();
    let Some(key) = config
        .api_keys
        .iter()
        .find(|key| key.id == key_id && usage_quota::enabled(key))
    else {
        // Preserve bytes and avoid count requests entirely when hard quotas are off.
        return Ok(prepared);
    };
    let mut value: serde_json::Value =
        serde_json::from_slice(&prepared.body).map_err(|_| QuotaError::Unmetered)?;
    let amount = if let Some(spec) = context_preflight_spec(path, &prepared.body) {
        let mut output = request::normalize(path, &mut value, key.quota_default_output_tokens)?;
        let count_body =
            Bytes::from(serde_json::to_vec(&value).map_err(|_| QuotaError::Unmetered)?);
        let input =
            fetch_input_token_count(client, target, headers, &count_body, spec.counter, config)
                .await
                .ok_or(QuotaError::Unmetered)?;
        output.fit_default(input, context_window, &mut value);
        prepared.body = Bytes::from(serde_json::to_vec(&value).map_err(|_| QuotaError::Unmetered)?);
        // llama.cpp can report its first sampled token even with max_tokens=0.
        // Preserve the zero sent upstream, but cover that step in the reservation.
        let reserved_output = output.tokens.max(1);
        if let Some(context_window) = context_window {
            prepared.context_check = Some(ContextLimitViolation {
                error_param: spec.error_param,
                input_tokens: Some(input),
                requested_output_tokens: reserved_output,
                context_window,
                input_source: "exact",
            });
        }
        input
            .checked_add(reserved_output)
            .ok_or(QuotaError::Unmetered)?
    } else {
        let items = vector_items(path, &value)?;
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
    prepared.reservation = Some((key.clone(), amount));
    Ok(prepared)
}

pub(super) async fn admit(
    usage: &UsageHandle,
    reservation: Option<(crate::models::ProxyApiKey, u64)>,
) -> Result<(), QuotaError> {
    if let Some((key, amount)) = reservation {
        let (_, id) = usage.quota_identity();
        usage.quota(usage_quota::reserve(key, id, amount).await?);
    }
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
        &error.message(),
        error.param(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
