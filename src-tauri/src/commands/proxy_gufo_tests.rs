// Uses the same transport, target resolution and readiness gate as managed engines.
#[tokio::test]
async fn gufo_router_requires_ready_identity_and_detects_count_unavailability() {
    let readiness = Arc::new(Mutex::new(json!({"status":"loading"})));
    let count_status = Arc::new(Mutex::new(StatusCode::OK));
    let ready_state = readiness.clone();
    let count_state = count_status.clone();
    let (upstream, task) = spawn_test_router(
        Router::new()
            .route(
                "/health",
                axum::routing::get(|| async { Json(json!({"status":"ok"})) }),
            )
            .route(
                "/ready",
                axum::routing::get(move || {
                    let s = ready_state.clone();
                    async move { Json(s.lock().unwrap().clone()) }
                }),
            )
            .route(
                "/props",
                axum::routing::get(|| async {
                    Json(json!({"default_generation_settings":{"n_ctx":32768}}))
                }),
            )
            .route("/slots", axum::routing::get(|| async { Json(json!([])) }))
            .route(
                "/v1/chat/completions/input_tokens",
                post(move || {
                    let s = count_state.clone();
                    async move { (*s.lock().unwrap(), Json(json!({"input_tokens":37}))) }
                }),
            ),
    )
    .await;
    let mut snapshot = openai_proxy_snapshot(upstream, "");
    let launch = snapshot
        .running
        .get_mut("openai-upstream")
        .unwrap()
        .launch_config
        .as_mut()
        .unwrap();
    launch.engine_kind = crate::gufo::EngineKind::Gufo;
    // Persisted edits cannot override the active engine's identity.
    let targets = super::resolve_proxy_candidates_from(
        &snapshot.config,
        &snapshot.instances,
        &snapshot.running,
        Some("local-openai"),
        None,
    );
    assert_eq!(targets.len(), 1);
    let target = targets[0].clone();
    assert_eq!(target.engine_kind, crate::gufo::EngineKind::Gufo);
    let runtime = Arc::new(super::RouterRuntime::default());
    assert!(!super::model_health_snapshots(&runtime, &targets)[0].ready);
    for body in [
        json!({"status":"loading"}),
        json!({"status":"ready","model":"wrong"}),
    ] {
        *readiness.lock().unwrap() = body;
        super::probe_target(runtime.clone(), target.clone(), snapshot.config.clone()).await;
        assert!(!runtime.target_snapshot("openai-upstream").ready);
    }
    *readiness.lock().unwrap() = json!({"status":"ready","model":"upstream-private"});
    super::probe_target(runtime.clone(), target.clone(), snapshot.config.clone()).await;
    assert!(runtime.target_snapshot("openai-upstream").ready);
    let client = reqwest::Client::new();
    for (status, expected) in [
        (StatusCode::OK, Some(37)),
        (StatusCode::NOT_FOUND, None),
        (StatusCode::NOT_IMPLEMENTED, None),
    ] {
        *count_status.lock().unwrap() = status;
        assert_eq!(
            super::fetch_input_token_count(
                &client,
                &target,
                &HeaderMap::new(),
                &Bytes::from_static(b"{}"),
                super::InputTokenCounter::Native("/v1/chat/completions/input_tokens"),
                &snapshot.config
            )
            .await,
            expected
        );
    }
    task.abort();
}
