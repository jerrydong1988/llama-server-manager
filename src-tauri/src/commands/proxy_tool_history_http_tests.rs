#[tokio::test]
async fn tool_history_rejections_never_reach_upstream_and_count_once() {
    let received = Arc::new(Mutex::new(Vec::<(String, serde_json::Value)>::new()));
    let capture = received.clone();
    let upstream_router = Router::new()
        .route("/health", get(|| async { Json(json!({"status":"ok"})) }))
        .fallback(post(move |uri: Uri, Json(body): Json<serde_json::Value>| {
            let received = capture.clone();
            async move {
                received.lock().unwrap().push((uri.path().into(), body));
                Json(json!({"input_tokens":12,"model":"upstream-private","choices":[{"message":{"content":"OK"},"finish_reason":"stop"}]}))
            }
        }));
    let (upstream, upstream_task) = spawn_test_router(upstream_router).await;
    let snapshot = openai_proxy_snapshot(upstream, "history-key");
    let (router, runtime) =
        super::proxy_router_from_source_with_runtime(Arc::new(TestProxySource { snapshot }));
    let (address, router_task) = spawn_test_router(router).await;
    let client = reqwest::Client::new();
    let assistant = json!({"role":"assistant","content":null,"tool_calls":[
        {"id":"a","type":"function","function":{"name":"retired_tool","arguments":"{}"}},
        {"id":"b","type":"function","function":{"name":"retired_tool","arguments":"{}"}}
    ]});
    let result_a = json!({"role":"tool","tool_call_id":"a","content":"{\"error\":\"cancelled\"}"});
    let result_b = json!({"role":"tool","tool_call_id":"b","content":"result B"});
    let pending = json!({"model":"local-openai","messages":[assistant.clone(),result_a.clone()],"max_tokens":64});
    let url = format!("http://{address}/v1/chat/completions");
    let mut rejected = 0;
    for stream in [false, true] {
        for messages in [
            json!([assistant.clone()]),
            json!([assistant.clone(), result_a.clone(), {"role":"user","content":"Summarize"}]),
            json!([assistant.clone(), {"role":"tool","tool_call_id":"wrong","content":"private result"}]),
            json!([assistant.clone(), result_a.clone(), result_a.clone()]),
            json!([result_a.clone()]),
        ] {
            let response = client.post(&url).bearer_auth("history-key")
                .header("anthropic-version", "2023-06-01")
                .json(&json!({"model":"local-openai","messages":messages,"stream":stream,"max_tokens":64}))
                .send().await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response.headers()["content-type"], "application/json");
            assert!(response.headers().contains_key("x-request-id"));
            assert!(response.headers().contains_key("x-lsm-request-id"));
            let value: serde_json::Value = response.json().await.unwrap();
            assert_eq!(value["error"]["type"], "invalid_request_error");
            assert_eq!(value["error"]["code"], "invalid_tool_history");
            assert!(value["error"]["param"]
                .as_str()
                .unwrap()
                .starts_with("messages["));
            assert!(!value.to_string().contains("private result"));
            rejected += 1;
            assert_eq!(audit_rejections(&runtime), rejected);
            assert_eq!(runtime.in_flight_requests(), 0);
            assert_eq!(
                runtime.target_snapshot("openai-upstream").active_requests,
                0
            );
            assert!(
                received.lock().unwrap().is_empty(),
                "invalid history must not reach preflight or generation"
            );
        }
    }
    // A complete out-of-order batch is forwarded unchanged, even when its tools
    // have since been removed from the current tool declarations.
    let valid = json!({"model":"local-openai", "messages":[assistant.clone(),result_b,result_a],
        "tools":[],"max_tokens":64});
    let response = client
        .post(&url)
        .bearer_auth("history-key")
        .json(&valid)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.bytes().await.unwrap();
    let messages = received
        .lock()
        .unwrap()
        .iter()
        .find(|(path, _)| path == "/v1/chat/completions")
        .unwrap()
        .1["messages"]
        .clone();
    assert_eq!(messages, valid["messages"]);
    assert_eq!(audit_rejections(&runtime), rejected);

    // Counting pending history is useful before executing tools. Other protocols
    // may refer to server-side state, so this Chat-only guard must not intercept them.
    for path in [
        "/v1/chat/completions/input_tokens",
        "/v1/responses",
        "/v1/messages",
        "/v1/messages/count_tokens",
    ] {
        let response = client
            .post(format!("http://{address}{path}"))
            .bearer_auth("history-key")
            .header("anthropic-version", "2023-06-01")
            .json(&pending)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let _ = response.bytes().await.unwrap();
        assert!(received
            .lock()
            .unwrap()
            .iter()
            .any(|(actual, body)| actual == path && body["messages"] == pending["messages"]));
        assert_eq!(audit_rejections(&runtime), rejected);
    }
    router_task.abort();
    upstream_task.abort();
}
