use bytes::Bytes;
use futures_util::StreamExt;
use std::time::{Duration, Instant};

/// Collect an HTTP response with independent size, idle-read, and total-time bounds.
///
/// Callers must still validate the response status before collecting the body.
pub async fn collect_response(
    response: reqwest::Response,
    limit: usize,
    idle_timeout: Duration,
    total_timeout: Duration,
) -> Result<(Bytes, Option<Instant>), String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("HTTP response exceeds {limit} bytes"));
    }

    let collect = async {
        let mut body = Vec::new();
        let mut first_chunk_at = None;
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::time::timeout(idle_timeout, stream.next())
                .await
                .map_err(|_| "HTTP response body idle timeout".to_string())?;
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk.map_err(|error| error.to_string())?;
            if first_chunk_at.is_none() && !chunk.is_empty() {
                first_chunk_at = Some(Instant::now());
            }
            let next_len = body
                .len()
                .checked_add(chunk.len())
                .ok_or_else(|| "HTTP response size overflow".to_string())?;
            if next_len > limit {
                return Err(format!("HTTP response exceeds {limit} bytes"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((Bytes::from(body), first_chunk_at))
    };

    tokio::time::timeout(total_timeout, collect)
        .await
        .map_err(|_| "HTTP response total timeout".to_string())?
}

#[cfg(test)]
mod tests {
    #[test]
    fn checked_size_boundary_is_explicit() {
        assert_eq!(usize::MAX.checked_add(1), None);
    }
}
