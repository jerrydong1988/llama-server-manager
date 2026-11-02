// HTTP-level regressions for the Hermes and DeepSeek manual audit findings.
fn audit_rejections(runtime: &super::RouterRuntime) -> u64 {
    runtime
        .prometheus_metrics()
        .lines()
        .find_map(|line| line.strip_prefix("lsm_router_requests_rejected_total "))
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn audit_routing_failures_are_classified_and_counted_once() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let received = calls.clone();
    let (upstream, upstream_task) = spawn_test_router(Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/v1/chat/completions", post(move |Json(body): Json<serde_json::Value>| {
            let calls = received.clone();
            async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if body["messages"][0]["content"] == "upstream-error" {
                    return (StatusCode::BAD_REQUEST, Json(json!({"error":{"message":"bad upstream request"}}))).into_response();
                }
                Json(json!({"model":"upstream-private","choices":[{"message":{"content":"OK"},"finish_reason":"stop"}],
                    "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}})).into_response()
            }
        }))).await;
    let mut snapshot = openai_proxy_snapshot(upstream, "audit-key");
    snapshot.config.routes[0].max_concurrent_requests = 1;
    let (router, runtime) =
        super::proxy_router_from_source_with_runtime(Arc::new(TestProxySource { snapshot }));
    let (address, task) = spawn_test_router(router).await;
    let client = reqwest::Client::new();
    let url = format!("http://{address}/v1/chat/completions");
    let valid = json!({"model":"local-openai","messages":[{"role":"user","content":"hello"}],"max_tokens":1});
    let mut expected = 0;
    for (body, status, code) in [
        (json!({}), 400, Some("model_required")),
        (json!({"model":null}), 400, Some("model_required")),
        (json!({"model":42}), 400, None),
        (json!({"model":"  "}), 400, None),
        (json!({"model":"unknown"}), 404, Some("model_not_found")),
        (
            json!({"model":"openai-upstream"}),
            404,
            Some("model_not_found"),
        ),
    ] {
        let response = client
            .post(&url)
            .bearer_auth("audit-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert!(response.headers().contains_key("x-lsm-request-id"));
        assert!(!response.headers().contains_key("retry-after"));
        let value: serde_json::Value = response.json().await.unwrap();
        if let Some(code) = code {
            assert_eq!(value["error"]["code"], code);
        }
        expected += 1;
        assert_eq!(audit_rejections(&runtime), expected);
    }
    // A valid public chat model cannot be silently dispatched to a vector workload.
    let response = client
        .post(format!("http://{address}/v1/embeddings"))
        .bearer_auth("audit-key")
        .json(&json!({"model":"local-openai","input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "model_endpoint_mismatch"
    );
    expected += 1;

    let unauthorized = client.post(&url).json(&valid).send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let _ = unauthorized.bytes().await.unwrap();
    expected += 1;
    assert_eq!(audit_rejections(&runtime), expected);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    // Hold one target permit, as a live streamed response would. Both protocols
    // must fail fast with the same capacity code; no second upstream dispatch.
    let candidate = super::RoutingCandidate {
        instance_id: "openai-upstream".into(),
        priority: 0,
        weight: 1,
        max_concurrent_requests: 1,
    };
    let (_, held) = runtime
        .select_and_acquire_target(&[candidate], "priorityFailover", "audit")
        .unwrap();
    for path in ["/v1/chat/completions", "/v1/responses", "/v1/messages"] {
        let response = client
            .post(format!("http://{address}{path}"))
            .bearer_auth("audit-key")
            .header("anthropic-version", "2023-06-01")
            .json(&valid)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["retry-after"], "1");
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "route_capacity");
        if path == "/v1/messages" {
            assert_eq!(body["type"], "error");
        }
        expected += 1;
        assert_eq!(audit_rejections(&runtime), expected);
    }
    drop(held);
    let response = client
        .post(&url)
        .bearer_auth("audit-key")
        .json(&valid)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.bytes().await.unwrap();
    let response = client.post(&url).bearer_auth("audit-key")
        .json(&json!({"model":"local-openai","messages":[{"role":"user","content":"upstream-error"}]})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = response.bytes().await.unwrap();
    assert_eq!(
        audit_rejections(&runtime),
        expected,
        "forwarded success/error must not count as local rejection"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(runtime.in_flight_requests(), 0);
    task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn audit_defaults_offline_routes_and_protocol_errors_remain_distinct() {
    let address = "127.0.0.1:9".parse().unwrap();
    let mut snapshot = openai_proxy_snapshot(address, "");
    snapshot.running.clear();
    for (model, expected) in [
        (Some("local-openai"), super::RoutingFailure::Unavailable),
        (Some("unknown"), super::RoutingFailure::ModelNotFound),
        (None, super::RoutingFailure::ModelRequired),
    ] {
        let resolution = super::proxy_request_resolution_from(
            snapshot.config.clone(),
            &snapshot.instances,
            &snapshot.running,
            model,
            None,
        );
        assert!(resolution.candidates.is_empty());
        assert_eq!(resolution.empty_failure, expected);
    }
    snapshot.config.default_instance_id = "openai-upstream".into();
    let resolution = super::proxy_request_resolution_from(
        snapshot.config.clone(),
        &snapshot.instances,
        &snapshot.running,
        None,
        None,
    );
    assert_eq!(resolution.empty_failure, super::RoutingFailure::Unavailable);
    let (router, runtime) =
        super::proxy_router_from_source_with_runtime(Arc::new(TestProxySource { snapshot }));
    let (address, task) = spawn_test_router(router).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{address}/v1/chat/completions"))
        .json(&json!({"model":"local-openai"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "route_unavailable"
    );
    assert_eq!(audit_rejections(&runtime), 1);
    for (path, code, status) in [
        ("/not-an-api", "endpoint_not_found", 404),
        ("/v1/chat/completions", "method_not_allowed", 405),
    ] {
        for anthropic in [false, true] {
            let mut request = client.get(format!("http://{address}{path}"));
            if anthropic {
                request = request.header("anthropic-version", "2023-06-01");
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()["content-type"], "application/json");
            assert!(response.headers().contains_key(if anthropic {
                "request-id"
            } else {
                "x-request-id"
            }));
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["error"]["code"], code);
            if anthropic {
                assert_eq!(body["type"], "error");
            }
        }
    }
    task.abort();
    // Configured defaults and the established single implicit target fallback survive.
    let mut snapshot = openai_proxy_snapshot("127.0.0.1:9".parse().unwrap(), "");
    snapshot.config.default_instance_id = "openai-upstream".into();
    assert_eq!(
        super::proxy_request_resolution_from(
            snapshot.config.clone(),
            &snapshot.instances,
            &snapshot.running,
            None,
            None
        )
        .candidates
        .len(),
        1
    );
    snapshot.config.default_instance_id.clear();
    snapshot.config.routes.clear();
    assert_eq!(
        super::proxy_request_resolution_from(
            snapshot.config,
            &snapshot.instances,
            &snapshot.running,
            None,
            None
        )
        .candidates
        .len(),
        1
    );
}

#[tokio::test]
async fn audit_cors_policy_overrides_upstream_without_losing_request_ids() {
    let (upstream, upstream_task) = spawn_test_router(Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .route("/v1/chat/completions", post(|| async {
            Response::builder().header("content-type", "application/json")
                .header("access-control-allow-origin", "*").header("access-control-allow-credentials", "true")
                .header("access-control-expose-headers", "upstream-secret").header("x-request-id", "r123")
                .body(Body::from(r#"{"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":0,"total_tokens":1}}"#)).unwrap()
        }))).await;
    let mut snapshot = openai_proxy_snapshot(upstream, "");
    snapshot.config.cors_allowed_origins = vec!["https://allowed.test".into()];
    let (address, task) =
        spawn_test_router(super::proxy_router_from_source(Arc::new(TestProxySource {
            snapshot,
        })))
        .await;
    let client = reqwest::Client::new();
    for origin in [
        None,
        Some("https://allowed.test"),
        Some("https://denied.test"),
    ] {
        let mut request = client
            .post(format!("http://{address}/v1/chat/completions"))
            .json(&json!({"model":"local-openai"}));
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = request.send().await.unwrap();
        assert!(!response
            .headers()
            .contains_key("access-control-allow-credentials"));
        if origin == Some("https://denied.test") {
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        } else {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["x-request-id"], "r123");
            assert!(response.headers().contains_key("x-lsm-request-id"));
        }
        if origin == Some("https://allowed.test") {
            assert_eq!(
                response.headers()["access-control-allow-origin"],
                "https://allowed.test"
            );
            assert!(response.headers()["access-control-expose-headers"]
                .to_str()
                .unwrap()
                .contains("x-lsm-request-id"));
        } else {
            assert!(!response
                .headers()
                .contains_key("access-control-allow-origin"));
        }
        let _ = response.bytes().await.unwrap();
    }
    task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn audit_global_admission_and_rate_rejections_are_not_double_counted() {
    for rate_limit in [false, true] {
        let mut snapshot = openai_proxy_snapshot("127.0.0.1:9".parse().unwrap(), "");
        snapshot.config.max_concurrent_requests = 1;
        snapshot.config.queue_timeout_ms = 0;
        snapshot.config.requests_per_minute = if rate_limit { 1 } else { 0 };
        let (router, runtime) =
            super::proxy_router_from_source_with_runtime(Arc::new(TestProxySource { snapshot }));
        let held = if rate_limit {
            None
        } else {
            runtime.acquire_request("held", Duration::ZERO).await
        };
        let (address, task) = spawn_test_router(router).await;
        let client = reqwest::Client::new();
        for attempt in 1..=2 {
            let response = client
                .post(format!("http://{address}/v1/chat/completions"))
                .json(&json!({"model":"unknown"}))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if rate_limit && attempt == 1 {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::TOO_MANY_REQUESTS
                }
            );
            let _ = response.bytes().await.unwrap();
            assert_eq!(audit_rejections(&runtime), attempt);
        }
        drop(held);
        task.abort();
    }
}
