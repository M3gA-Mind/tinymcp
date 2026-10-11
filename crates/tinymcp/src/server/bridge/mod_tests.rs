//! Server sessions preserve protocol ownership while host callbacks stay external.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use serde_json::{Value, json};
use tinymcp_bus::{
    HEADER_PROTOCOL_VERSION, HEADER_SESSION_ID, ServerHostCall, ServerHostReply,
    ServerHttpResponseMetadata, ServerHttpSessionConfig, ServerInput, ServerOperationState,
    ServerSessionConfig,
};

fn input(line: String) -> ServerInput {
    ServerInput {
        operation_id: next_id(),
        line,
    }
}
fn config() -> ServerSessionConfig {
    ServerSessionConfig {
        session_id: next_id(),
        info: json!({"name":"host", "version":"1"}),
        source_type_prefix: "mcp".into(),
        resources: vec![json!({"uri":"host://info", "name":"info"})],
    }
}
fn http_config(
    requested_session_id: Option<&str>,
    known_session: bool,
    expected_protocol_version: Option<&str>,
) -> ServerHttpSessionConfig {
    ServerHttpSessionConfig {
        server: config(),
        requested_session_id: requested_session_id.map(str::to_owned),
        known_session,
        expected_protocol_version: expected_protocol_version.map(str::to_owned),
    }
}
async fn ready(sessions: &ServerSessions, id: &str) -> ServerOperationState {
    for _ in 0..1000 {
        let state = sessions.poll(id).await.unwrap().state;
        if state != ServerOperationState::Pending {
            return state;
        }
        tokio::task::yield_now().await;
    }
    panic!("operation did not become observable");
}
async fn submit(sessions: &ServerSessions, id: &str, value: Value) {
    sessions
        .submit(id, serde_json::Map::new(), input(value.to_string()))
        .await
        .unwrap();
}
async fn response(sessions: &ServerSessions, id: &str) -> Value {
    let ServerOperationState::Complete {
        response: Some(line),
    } = ready(sessions, id).await
    else {
        panic!("expected response")
    };
    serde_json::from_str(&line).unwrap()
}
#[tokio::test]
async fn initialize_freezes_provenance_and_host_dispatches_normalized_calls() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    submit(&sessions, &id, json!({"jsonrpc":"2.0", "id":1,"method":"initialize","params":{"clientInfo":{"name":"Claude Desktop"}}})).await;
    assert_eq!(
        response(&sessions, &id).await["result"]["serverInfo"]["name"],
        "host"
    );
    submit(&sessions, &id, json!({"jsonrpc":"2.0", "id":2,"method":"tools/call","params":{"name":" echo ","arguments":"{\"x\":1}"}})).await;
    let ServerOperationState::Callback { callback } = ready(&sessions, &id).await else {
        panic!("callback missing")
    };
    assert!(
        matches!(&callback.call, ServerHostCall::CallTool { source_type, name, arguments, .. } if source_type == "mcp:claude-desktop" && name == "echo" && arguments["x"] == 1)
    );
    assert_eq!(
        sessions.poll(&id).await.unwrap().state,
        ServerOperationState::Callback {
            callback: callback.clone()
        }
    );
    assert!(
        sessions
            .complete(&id, "wrong", ServerHostReply::Success { value: json!({}) })
            .await
            .is_err()
    );
    sessions
        .complete(
            &id,
            &callback.id,
            ServerHostReply::Success {
                value: json!({"content":[]}),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        response(&sessions, &id).await["result"]["content"],
        json!([])
    );
    assert!(
        sessions
            .complete(
                &id,
                &callback.id,
                ServerHostReply::Success { value: json!({}) }
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn session_metadata_reports_the_module_negotiated_protocol_version() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    assert_eq!(
        sessions.metadata(&id).await.unwrap(),
        tinymcp_bus::ServerSessionMetadata {
            initialized: false,
            protocol_version: None,
        }
    );

    submit(
        &sessions,
        &id,
        json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"protocolVersion":"unsupported-version"}
        }),
    )
    .await;
    let initialize_response = response(&sessions, &id).await;
    assert_eq!(
        sessions.metadata(&id).await.unwrap(),
        tinymcp_bus::ServerSessionMetadata {
            initialized: true,
            protocol_version: Some(
                initialize_response["result"]["protocolVersion"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            ),
        }
    );
    assert_eq!(
        sessions
            .metadata(&id)
            .await
            .unwrap()
            .protocol_version
            .as_deref(),
        Some(tinymcp_bus::LATEST_PROTOCOL_VERSION)
    );
    submit(
        &sessions,
        &id,
        json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"initialize",
            "params":{"protocolVersion":tinymcp_bus::SUPPORTED_PROTOCOL_VERSIONS[0]}
        }),
    )
    .await;
    let _ = response(&sessions, &id).await;
    assert_eq!(
        sessions
            .metadata(&id)
            .await
            .unwrap()
            .protocol_version
            .as_deref(),
        Some(tinymcp_bus::SUPPORTED_PROTOCOL_VERSIONS[0])
    );
}

#[tokio::test]
async fn http_admission_rejects_preinitialize_tools_before_host_callbacks() {
    let sessions = ServerSessions::default();
    let id = sessions
        .open_http(http_config(None, false, None))
        .await
        .unwrap();
    submit(
        &sessions,
        &id,
        json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/call",
            "params":{"name":"must-not-run"}
        }),
    )
    .await;

    assert_eq!(
        ready(&sessions, &id).await,
        ServerOperationState::Complete { response: None }
    );
    assert_eq!(
        sessions.poll(&id).await.unwrap().http_response,
        Some(ServerHttpResponseMetadata {
            status_code: 400,
            headers: std::collections::BTreeMap::default(),
            content_type: Some("text/plain; charset=utf-8".into()),
            error_body: Some("missing or invalid Mcp-Session-Id header".into()),
        })
    );
}

#[tokio::test]
async fn http_initialize_returns_module_negotiated_session_headers() {
    let sessions = ServerSessions::default();
    let id = sessions
        .open_http(http_config(None, false, None))
        .await
        .unwrap();
    submit(
        &sessions,
        &id,
        json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"protocolVersion":"unsupported-version"}
        }),
    )
    .await;
    let initialize_response = response(&sessions, &id).await;
    assert_eq!(
        sessions
            .metadata(&id)
            .await
            .unwrap()
            .protocol_version
            .as_deref(),
        Some(tinymcp_bus::LATEST_PROTOCOL_VERSION)
    );
    assert_eq!(
        initialize_response["result"]["protocolVersion"],
        tinymcp_bus::LATEST_PROTOCOL_VERSION
    );
    let http_response = sessions.poll(&id).await.unwrap().http_response.unwrap();
    assert_eq!(http_response.status_code, 200);
    assert_eq!(
        http_response.content_type.as_deref(),
        Some("application/json")
    );
    assert!(!http_response.headers.contains_key(HEADER_PROTOCOL_VERSION));
    let external_id = http_response.headers[HEADER_SESSION_ID].clone();

    sessions
        .submit(
            &id,
            serde_json::Map::from_iter([
                (HEADER_SESSION_ID.into(), json!(external_id)),
                (
                    HEADER_PROTOCOL_VERSION.into(),
                    json!(tinymcp_bus::LATEST_PROTOCOL_VERSION),
                ),
            ]),
            input(json!({"jsonrpc":"2.0","id":2,"method":"ping"}).to_string()),
        )
        .await
        .unwrap();
    let response = response(&sessions, &id).await;
    assert_eq!(response["result"], json!({}));
    let http_response = sessions.poll(&id).await.unwrap().http_response.unwrap();
    assert_eq!(http_response.status_code, 200);
    assert!(http_response.headers.is_empty());
}

#[tokio::test]
async fn http_stale_and_mismatched_sessions_have_transport_statuses() {
    for (context, headers, expected_status, expected_body) in [
        (
            http_config(Some("expired-session"), false, None),
            serde_json::Map::from_iter([(HEADER_SESSION_ID.into(), json!("expired-session"))]),
            404,
            "unknown or expired MCP session",
        ),
        (
            http_config(Some("known-session"), true, Some("2025-11-25")),
            serde_json::Map::from_iter([
                (HEADER_SESSION_ID.into(), json!("known-session")),
                (HEADER_PROTOCOL_VERSION.into(), json!("wrong-version")),
            ]),
            400,
            "missing or invalid MCP-Protocol-Version header",
        ),
    ] {
        let sessions = ServerSessions::default();
        let id = sessions.open_http(context).await.unwrap();
        sessions
            .submit(
                &id,
                headers,
                input(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string()),
            )
            .await
            .unwrap();
        assert_eq!(
            ready(&sessions, &id).await,
            ServerOperationState::Complete { response: None }
        );
        let response = sessions.poll(&id).await.unwrap().http_response.unwrap();
        assert_eq!(response.status_code, expected_status);
        assert_eq!(response.error_body.as_deref(), Some(expected_body));
    }
}
#[tokio::test]
async fn callback_failures_are_jsonrpc_errors_and_cancellation_releases_waiters() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    for (method, params, reply, code) in [
        (
            "tools/list",
            json!({}),
            ServerHostReply::Internal {
                message: "unavailable".into(),
            },
            -32603,
        ),
        (
            "tools/call",
            json!({"name":"echo"}),
            ServerHostReply::InvalidParams {
                message: "denied".into(),
            },
            -32602,
        ),
        (
            "resources/read",
            json!({"uri":"host://missing"}),
            ServerHostReply::ResourceNotFound {
                message: "missing".into(),
            },
            -32002,
        ),
    ] {
        submit(
            &sessions,
            &id,
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
        )
        .await;
        let ServerOperationState::Callback { callback } = ready(&sessions, &id).await else {
            panic!("callback missing")
        };
        sessions.complete(&id, &callback.id, reply).await.unwrap();
        assert_eq!(response(&sessions, &id).await["error"]["code"], code);
    }
    submit(
        &sessions,
        &id,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    let ServerOperationState::Callback { callback } = ready(&sessions, &id).await else {
        panic!("callback missing")
    };
    cancel(&sessions, &id).await.unwrap();
    assert!(
        sessions
            .complete(
                &id,
                &callback.id,
                ServerHostReply::Success { value: json!([]) }
            )
            .await
            .is_err()
    );
    assert_eq!(
        sessions.poll(&id).await.unwrap().state,
        ServerOperationState::Cancelled
    );
    submit(
        &sessions,
        &id,
        json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
    )
    .await;
    assert_eq!(response(&sessions, &id).await["result"], json!({}));
    sessions.close(&id).await.unwrap();
    assert!(sessions.close(&id).await.is_ok());
    assert!(sessions.poll(&id).await.is_err());
}
#[tokio::test]
async fn rejects_oversized_inputs_and_closed_handles_and_shutdown_aborts_queued_work() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    assert!(
        sessions
            .submit(
                &id,
                serde_json::Map::new(),
                input("x".repeat(MAX_BYTES + 1))
            )
            .await
            .is_err()
    );
    assert!(
        sessions
            .submit(
                &id,
                serde_json::from_value(json!({"bad":1})).unwrap(),
                input("{}".into())
            )
            .await
            .is_err()
    );
    submit(
        &sessions,
        &id,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    assert!(
        sessions
            .submit(&id, serde_json::Map::new(), input("{}".into()))
            .await
            .is_err()
    );
    assert_eq!(sessions.shutdown().await, 1);
    assert_eq!(sessions.shutdown().await, 0);
    assert!(
        sessions
            .submit(&id, serde_json::Map::new(), input("{}".into()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_after_initialize_does_not_unlock_session_provenance() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    submit(&sessions,&id,json!([
        {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"original"}}},
        {"jsonrpc":"2.0","id":2,"method":"tools/list"}
    ])).await;
    assert!(matches!(
        ready(&sessions, &id).await,
        ServerOperationState::Callback { .. }
    ));
    cancel(&sessions, &id).await.unwrap();
    assert_eq!(
        sessions.poll(&id).await.unwrap().state,
        ServerOperationState::Cancelled
    );
    submit(&sessions,&id,json!([
        {"jsonrpc":"2.0","id":3,"method":"initialize","params":{"clientInfo":{"name":"replacement"}}},
        {"jsonrpc":"2.0","id":4,"method":"tools/list"}
    ])).await;
    let ServerOperationState::Callback { callback } = ready(&sessions, &id).await else {
        panic!("callback missing")
    };
    assert!(
        matches!(callback.call, ServerHostCall::ListTools {source_type,..} if source_type == "mcp:original")
    );
}

#[tokio::test]
async fn tools_resources_and_prompts_are_host_owned_and_validation_stays_in_module() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    for (method, params, reply) in [
        (
            "tools/list",
            json!({}),
            json!([{ "name":"echo", "description":"Echo", "inputSchema":{} }]),
        ),
        ("prompts/list", json!({}), json!({"prompts":[]})),
        (
            "prompts/get",
            json!({"name":"guide","arguments":{"topic":"local"}}),
            json!({"messages":[]}),
        ),
        (
            "resources/read",
            json!({"uri":"host://info"}),
            json!({"contents":[]}),
        ),
    ] {
        sessions
            .submit(
                &id,
                serde_json::from_value(json!({"Authorization":"fixture-only"})).unwrap(),
                input(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string()),
            )
            .await
            .unwrap();
        let ServerOperationState::Callback { callback } = ready(&sessions, &id).await else {
            panic!("callback missing")
        };
        let wire = serde_json::to_value(&callback.call).unwrap();
        assert_eq!(wire["source_type"], "mcp");
        assert_eq!(wire["headers"]["authorization"], "fixture-only");
        sessions
            .complete(&id, &callback.id, ServerHostReply::Success { value: reply })
            .await
            .unwrap();
        assert!(response(&sessions, &id).await.get("result").is_some());
    }
    for params in [
        json!({}),
        json!({"name":""}),
        json!({"name":"guide","arguments":null}),
        json!({"name":"guide","arguments":{"topic":3}}),
    ] {
        submit(
            &sessions,
            &id,
            json!({"jsonrpc":"2.0","id":1,"method":"prompts/get","params":params}),
        )
        .await;
        assert_eq!(response(&sessions, &id).await["error"]["code"], -32602);
    }
    submit(
        &sessions,
        &id,
        json!({"jsonrpc":"2.0","id":1,"method":"resources/list"}),
    )
    .await;
    assert_eq!(
        response(&sessions, &id).await["result"]["resources"][0]["uri"],
        "host://info"
    );
    submit(
        &sessions,
        &id,
        json!({"method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(
        ready(&sessions, &id).await,
        ServerOperationState::Complete { response: None }
    );
    sessions
        .submit(&id, serde_json::Map::new(), input("{".into()))
        .await
        .unwrap();
    assert_eq!(response(&sessions, &id).await["error"]["code"], -32700);
}

#[tokio::test]
async fn rejects_bad_declarations_idle_calls_limits_and_duplicate_completions() {
    let sessions = ServerSessions::default();
    for config in [
        ServerSessionConfig {
            info: json!({}),
            ..config()
        },
        ServerSessionConfig {
            resources: vec![json!({})],
            ..config()
        },
        ServerSessionConfig {
            source_type_prefix: " ".into(),
            ..config()
        },
        ServerSessionConfig {
            info: json!("x".repeat(MAX_BYTES)),
            ..config()
        },
    ] {
        assert!(sessions.open(config).await.is_err());
    }
    for _ in 0..32 {
        sessions.open(config()).await.unwrap();
    }
    assert!(sessions.open(config()).await.is_err());
    assert_eq!(sessions.shutdown().await, 32);
    let id = sessions.open(config()).await.unwrap();
    assert!(sessions.poll(&id).await.is_err());
    assert!(cancel(&sessions, &id).await.is_err());
    assert!(cancel(&sessions, "closed").await.is_err());
    assert!(
        sessions
            .complete(
                "closed",
                "none",
                ServerHostReply::Success { value: json!({}) }
            )
            .await
            .is_err()
    );
    assert!(
        sessions
            .complete(&id, "none", ServerHostReply::Success { value: json!({}) })
            .await
            .is_err()
    );
    assert!(
        sessions
            .submit(
                &id,
                serde_json::from_value(json!({"large":"x".repeat(MAX_BYTES)})).unwrap(),
                input("{}".into())
            )
            .await
            .is_err()
    );
    submit(
        &sessions,
        &id,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    assert!(
        sessions
            .complete(&id, "none", ServerHostReply::Success { value: json!({}) })
            .await
            .is_err()
    );
    let ServerOperationState::Callback { callback } = ready(&sessions, &id).await else {
        panic!("callback missing")
    };
    assert!(
        sessions
            .complete(
                &id,
                &callback.id,
                ServerHostReply::Success {
                    value: json!("x".repeat(MAX_BYTES))
                }
            )
            .await
            .is_err()
    );
    sessions
        .complete(
            &id,
            &callback.id,
            ServerHostReply::Success {
                value: json!({"invalid":"tool list"}),
            },
        )
        .await
        .unwrap();
    assert_eq!(response(&sessions, &id).await["error"]["code"], -32603);
    assert!(format!("{sessions:?}").contains("ServerSessions"));
}

#[tokio::test]
async fn known_reservations_recover_lost_open_replies_and_close_before_late_open() {
    let sessions = ServerSessions::default();
    let first = config();
    let id = first.session_id.clone();
    assert_eq!(sessions.open(first.clone()).await.unwrap(), id);
    assert_eq!(sessions.open(first.clone()).await.unwrap(), id);
    assert!(
        sessions
            .open(ServerSessionConfig {
                source_type_prefix: "different".into(),
                ..first.clone()
            })
            .await
            .is_err()
    );
    sessions.close(&id).await.unwrap();
    assert!(sessions.open(first).await.is_err());
    let late = config();
    sessions.close(&late.session_id).await.unwrap();
    assert!(sessions.open(late).await.is_err());
    assert!(sessions.close("malformed").await.is_err());
    assert!(
        sessions
            .open(ServerSessionConfig {
                session_id: "malformed".into(),
                ..config()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_and_close_join_workers_and_cancellation_is_idempotent() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    submit(
        &sessions,
        &id,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    let ServerOperationState::Callback { callback } = ready(&sessions, &id).await else {
        panic!("callback missing")
    };
    assert_eq!(
        ready(&sessions, &id).await,
        ServerOperationState::Callback { callback }
    );
    cancel(&sessions, &id).await.unwrap();
    cancel(&sessions, &id).await.unwrap();
    sessions.close(&id).await.unwrap();
    let id = sessions.open(config()).await.unwrap();
    submit(
        &sessions,
        &id,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    sessions.close(&id).await.unwrap();
    assert!(sessions.poll(&id).await.is_err());
    let id = sessions.open(config()).await.unwrap();
    submit(
        &sessions,
        &id,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    cancel(&sessions, &id).await.unwrap();
    assert_eq!(sessions.shutdown().await, 1);
}

async fn cancel(sessions: &ServerSessions, id: &str) -> crate::Result<()> {
    let operation_id = sessions
        .poll(id)
        .await
        .map_or_else(|_| String::new(), |state| state.operation_id);
    sessions.cancel(id, &operation_id).await
}

fn next_id() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    uuid::Uuid::from_u128(u128::from(
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
    ))
    .to_string()
}
