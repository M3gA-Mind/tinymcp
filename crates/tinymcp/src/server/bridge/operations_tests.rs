//! Cancellation and teardown retain ownership until protocol workers finish.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use serde_json::json;

fn config() -> ServerSessionConfig {
    ServerSessionConfig {
        session_id: next_id(),
        info: json!({"name":"fixture","version":"1"}),
        source_type_prefix: "mcp".into(),
        resources: vec![],
    }
}
#[tokio::test]
async fn abort_is_joined_before_cancel_returns_and_task_failure_releases_operation() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    sessions
        .submit(
            &id,
            Map::new(),
            ServerInput {
                operation_id: next_id(),
                line: json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
            },
        )
        .await
        .unwrap();
    cancel(&sessions, &id).await.unwrap();
    {
        let state = sessions.sessions.lock().await;
        let op = state.active[&id].operation.as_ref().unwrap();
        assert!(op.task.is_none());
        assert!(op.waiting.is_none());
        assert!(op.receiver.is_closed());
    }
    assert_eq!(
        sessions.poll(&id).await.unwrap().state,
        ServerOperationState::Cancelled
    );
    sessions
        .submit(
            &id,
            Map::new(),
            ServerInput {
                operation_id: next_id(),
                line: "{}".into(),
            },
        )
        .await
        .unwrap();
    {
        let mut state = sessions.sessions.lock().await;
        let op = state
            .active
            .get_mut(&id)
            .unwrap()
            .operation
            .as_mut()
            .unwrap();
        op.task.as_ref().unwrap().abort();
    }
    tokio::task::yield_now().await;
    assert!(matches!(
        sessions.poll(&id).await.unwrap().state,
        ServerOperationState::Failed { .. }
    ));
    assert!(matches!(
        sessions.poll(&id).await.unwrap().state,
        ServerOperationState::Failed { .. }
    ));
}
#[tokio::test]
async fn closed_session_slots_are_reclaimed_beyond_previous_lifetime_limit() {
    let sessions = ServerSessions::default();
    let mut first = None;
    for sequence in 1..=4100 {
        let cfg = ServerSessionConfig {
            session_id: uuid::Uuid::from_u128(sequence).to_string(),
            ..config()
        };
        first.get_or_insert_with(|| cfg.clone());
        let id = sessions.open(cfg).await.unwrap();
        sessions.close(&id).await.unwrap();
    }
    assert!(sessions.open(first.unwrap()).await.is_err());
    let state = sessions.sessions.lock().await;
    assert!(state.active.is_empty());
    assert_eq!(state.reserved.len(), ADMISSION_WINDOW as usize);
}

#[tokio::test]
async fn operations_reclaim_history_beyond_previous_lifetime_limit_without_replaying() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    let first = ServerInput {
        operation_id: uuid::Uuid::from_u128(1).to_string(),
        line: "{}".into(),
    };
    for sequence in 1..=4100 {
        let input = ServerInput {
            operation_id: uuid::Uuid::from_u128(sequence).to_string(),
            line: "{}".into(),
        };
        sessions
            .submit(&id, Map::new(), input.clone())
            .await
            .unwrap();
        sessions.cancel(&id, &input.operation_id).await.unwrap();
        let discarded = sessions.poll(&id).await.unwrap();
        sessions.submit(&id, Map::new(), input).await.unwrap();
        assert_eq!(sessions.poll(&id).await.unwrap(), discarded);
    }
    assert!(
        sessions
            .submit(&id, Map::new(), first.clone())
            .await
            .is_err()
    );
    assert!(sessions.cancel(&id, &first.operation_id).await.is_err());
}

#[tokio::test]
async fn static_batch_budget_stops_before_later_host_dispatch() {
    let sessions = ServerSessions::default();
    let cfg = ServerSessionConfig {
        resources: vec![
            json!({"uri":"fixture://large","name":"large","description":"x".repeat(40_000)}),
        ],
        ..config()
    };
    let id = sessions.open(cfg).await.unwrap();
    let mut batch = vec![json!({"jsonrpc":"2.0","id":1,"method":"resources/list"}); 256];
    batch.push(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    batch.remove(0);
    sessions
        .submit(
            &id,
            Map::new(),
            ServerInput {
                operation_id: next_id(),
                line: json!(batch).to_string(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        visible(&sessions, &id).await.state,
        ServerOperationState::Failed { .. }
    ));
}

async fn visible(sessions: &ServerSessions, id: &str) -> ServerOperationSnapshot {
    loop {
        let state = sessions.poll(id).await.unwrap();
        if state.state != ServerOperationState::Pending {
            return state;
        }
        tokio::task::yield_now().await;
    }
}
#[tokio::test]
async fn discarded_terminal_replies_and_retry_submit_never_repeat_host_side_effects() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    let input = ServerInput {
        operation_id: next_id(),
        line: json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write"}})
            .to_string(),
    };
    sessions
        .submit(&id, Map::new(), input.clone())
        .await
        .unwrap();
    sessions
        .submit(&id, Map::new(), input.clone())
        .await
        .unwrap();
    let ServerOperationState::Callback { callback } = visible(&sessions, &id).await.state else {
        panic!("callback missing")
    };
    assert!(
        sessions
            .submit(
                &id,
                Map::new(),
                ServerInput {
                    operation_id: input.operation_id.clone(),
                    line: "different".into()
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
                value: json!({"content":[],"write_count":1}),
            },
        )
        .await
        .unwrap();
    let discarded = visible(&sessions, &id).await;
    assert!(matches!(
        discarded.state,
        ServerOperationState::Complete { .. }
    ));
    assert_eq!(sessions.poll(&id).await.unwrap(), discarded);
    sessions
        .submit(&id, Map::new(), input.clone())
        .await
        .unwrap();
    assert_eq!(sessions.poll(&id).await.unwrap(), discarded);
    let next = ServerInput {
        operation_id: next_id(),
        line: "{}".into(),
    };
    sessions
        .submit(&id, Map::new(), next.clone())
        .await
        .unwrap();
    cancel(&sessions, &id).await.unwrap();
    let cancelled = sessions.poll(&id).await.unwrap();
    assert_eq!(cancelled.state, ServerOperationState::Cancelled);
    assert_eq!(sessions.poll(&id).await.unwrap(), cancelled);
    sessions.submit(&id, Map::new(), next).await.unwrap();
    assert_eq!(sessions.poll(&id).await.unwrap(), cancelled);
    assert!(sessions.submit(&id, Map::new(), input).await.is_err());
}
#[tokio::test]
async fn aggregate_callback_replies_and_batch_counts_are_bounded() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    let batch =
        vec![json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read"}}); 2];
    sessions
        .submit(
            &id,
            Map::new(),
            ServerInput {
                operation_id: next_id(),
                line: json!(batch).to_string(),
            },
        )
        .await
        .unwrap();
    for index in 0..2 {
        let ServerOperationState::Callback { callback } = visible(&sessions, &id).await.state
        else {
            panic!("callback missing")
        };
        let result = sessions
            .complete(
                &id,
                &callback.id,
                ServerHostReply::Success {
                    value: json!({"text":"x".repeat(600_000)}),
                },
            )
            .await;
        if index == 0 {
            result.unwrap();
        } else {
            assert!(result.is_err());
        }
    }
    cancel(&sessions, &id).await.unwrap();
    assert!(
        sessions
            .submit(
                &id,
                Map::new(),
                ServerInput {
                    operation_id: next_id(),
                    line: json!(vec![json!({}); 257]).to_string()
                }
            )
            .await
            .is_err()
    );
    assert!(
        sessions
            .submit(
                &id,
                Map::new(),
                ServerInput {
                    operation_id: "invalid".into(),
                    line: "{}".into()
                }
            )
            .await
            .is_err()
    );
}

async fn cancel(sessions: &ServerSessions, id: &str) -> crate::Result<()> {
    let operation_id = sessions
        .poll(id)
        .await
        .map_or_else(|_| String::new(), |state| state.operation_id);
    sessions.cancel(id, &operation_id).await
}

#[tokio::test]
async fn stale_cancel_cannot_abort_a_successor_operation() {
    let sessions = ServerSessions::default();
    let id = sessions.open(config()).await.unwrap();
    let first = ServerInput {
        operation_id: next_id(),
        line: "{}".into(),
    };
    sessions
        .submit(&id, Map::new(), first.clone())
        .await
        .unwrap();
    sessions.cancel(&id, &first.operation_id).await.unwrap();
    let next = ServerInput {
        operation_id: next_id(),
        line: json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
    };
    sessions
        .submit(&id, Map::new(), next.clone())
        .await
        .unwrap();
    assert!(sessions.cancel(&id, &first.operation_id).await.is_err());
    let current = visible(&sessions, &id).await;
    assert_eq!(current.operation_id, next.operation_id);
    assert!(matches!(
        current.state,
        ServerOperationState::Callback { .. }
    ));
    sessions.cancel(&id, &next.operation_id).await.unwrap();
}

fn next_id() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    uuid::Uuid::from_u128(u128::from(
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
    ))
    .to_string()
}

#[tokio::test]
async fn http_get_admission_is_module_owned() {
    use tinymcp_bus::{
        HEADER_PROTOCOL_VERSION, HEADER_SESSION_ID, ServerHttpSessionMethod,
        ServerHttpSessionRequest,
    };

    let sessions = ServerSessions::default();
    let missing = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Get,
            requested_session_id: None,
            internal_session_id: None,
            headers: Map::new(),
        })
        .await;
    assert_eq!(missing.status_code, 400);
    assert_eq!(
        missing.error_body.as_deref(),
        Some("missing Mcp-Session-Id header")
    );
    let (id, wire_id) = initialized_http_session(&sessions).await;
    let unknown = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Get,
            requested_session_id: Some("wire-stale".into()),
            internal_session_id: None,
            headers: serde_json::from_value(json!({HEADER_SESSION_ID:"wire-stale"})).unwrap(),
        })
        .await;
    assert_eq!(unknown.status_code, 404);

    let mismatch = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Get,
            requested_session_id: Some(wire_id.clone()),
            internal_session_id: Some(id.clone()),
            headers: serde_json::from_value(json!({HEADER_SESSION_ID:wire_id})).unwrap(),
        })
        .await;
    assert_eq!(mismatch.status_code, 400);
    assert_eq!(
        mismatch.error_body.as_deref(),
        Some("missing or invalid MCP-Protocol-Version header")
    );

    let misrouted = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Get,
            requested_session_id: Some("wire-alias".into()),
            internal_session_id: Some(id.clone()),
            headers: serde_json::from_value(json!({
                HEADER_SESSION_ID:"wire-alias",
                HEADER_PROTOCOL_VERSION:tinymcp_bus::LATEST_PROTOCOL_VERSION
            }))
            .unwrap(),
        })
        .await;
    assert_eq!(misrouted.status_code, 404);

    let valid = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Get,
            requested_session_id: Some(wire_id.clone()),
            internal_session_id: Some(id.clone()),
            headers: serde_json::from_value(json!({
                HEADER_SESSION_ID:wire_id,
                HEADER_PROTOCOL_VERSION:tinymcp_bus::LATEST_PROTOCOL_VERSION
            }))
            .unwrap(),
        })
        .await;
    assert_eq!(valid.status_code, 200);
    assert_eq!(valid.content_type.as_deref(), Some("text/event-stream"));
}

#[tokio::test]
async fn http_delete_is_idempotent_and_joins_known_session_cleanup() {
    use tinymcp_bus::{HEADER_SESSION_ID, ServerHttpSessionMethod, ServerHttpSessionRequest};

    let sessions = ServerSessions::default();
    let missing = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Delete,
            requested_session_id: None,
            internal_session_id: None,
            headers: Map::new(),
        })
        .await;
    assert_eq!(missing.status_code, 400);

    let (id, wire_id) = initialized_http_session(&sessions).await;
    let misrouted_delete = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Delete,
            requested_session_id: Some("wire-alias".into()),
            internal_session_id: Some(id.clone()),
            headers: serde_json::from_value(json!({HEADER_SESSION_ID:"wire-alias"})).unwrap(),
        })
        .await;
    assert_eq!(misrouted_delete.status_code, 204);
    assert!(sessions.metadata(&id).await.is_ok());

    let deleted = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Delete,
            requested_session_id: Some(wire_id.clone()),
            internal_session_id: Some(id.clone()),
            headers: serde_json::from_value(json!({HEADER_SESSION_ID:wire_id})).unwrap(),
        })
        .await;
    assert_eq!(deleted.status_code, 204);
    assert!(sessions.metadata(&id).await.is_err());

    let failed_cleanup = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Delete,
            requested_session_id: Some("wire-current".into()),
            internal_session_id: Some("not-a-session-uuid".into()),
            headers: serde_json::from_value(json!({HEADER_SESSION_ID:"wire-current"})).unwrap(),
        })
        .await;
    assert_eq!(failed_cleanup.status_code, 500);

    let stale_delete = sessions
        .http_session(ServerHttpSessionRequest {
            method: ServerHttpSessionMethod::Delete,
            requested_session_id: Some("wire-stale".into()),
            internal_session_id: None,
            headers: serde_json::from_value(json!({HEADER_SESSION_ID:"wire-stale"})).unwrap(),
        })
        .await;
    assert_eq!(stale_delete.status_code, 204);
}

async fn initialized_http_session(sessions: &ServerSessions) -> (String, String) {
    let id = sessions
        .open_http(tinymcp_bus::ServerHttpSessionConfig {
            server: config(),
            requested_session_id: None,
            known_session: false,
            expected_protocol_version: None,
        })
        .await
        .unwrap();
    sessions
        .submit(
            &id,
            Map::new(),
            ServerInput {
                operation_id: next_id(),
                line: json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "method":"initialize",
                    "params":{"protocolVersion":tinymcp_bus::LATEST_PROTOCOL_VERSION}
                })
                .to_string(),
            },
        )
        .await
        .unwrap();
    let initialized = visible(sessions, &id).await;
    assert!(matches!(
        initialized.state,
        ServerOperationState::Complete { .. }
    ));
    let wire_id = initialized
        .http_response
        .expect("HTTP initialize metadata")
        .headers[HEADER_SESSION_ID]
        .clone();
    (id, wire_id)
}

#[tokio::test]
async fn http_response_metadata_is_bound_to_the_polled_operation() {
    use tinymcp_bus::{ServerHttpSessionConfig, ServerOperationState};

    let sessions = ServerSessions::default();
    let id = sessions
        .open_http(ServerHttpSessionConfig {
            server: config(),
            requested_session_id: Some("wire-session".into()),
            known_session: true,
            expected_protocol_version: Some(tinymcp_bus::LATEST_PROTOCOL_VERSION.into()),
        })
        .await
        .unwrap();
    let first = ServerInput {
        operation_id: next_id(),
        line: json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
    };
    let headers = serde_json::from_value(json!({HEADER_SESSION_ID:"wrong"})).unwrap();
    sessions.submit(&id, headers, first.clone()).await.unwrap();
    let first_snapshot = sessions.poll(&id).await.unwrap();
    assert!(matches!(
        first_snapshot.state,
        ServerOperationState::Complete { .. }
    ));
    assert_eq!(
        first_snapshot.http_response.as_ref().unwrap().status_code,
        404
    );

    let second = ServerInput {
        operation_id: next_id(),
        line: json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
    };
    let headers = serde_json::from_value(json!({HEADER_SESSION_ID:"wire-session"})).unwrap();
    sessions.submit(&id, headers, second).await.unwrap();
    let second_snapshot = sessions.poll(&id).await.unwrap();
    assert_eq!(
        second_snapshot.http_response.as_ref().unwrap().status_code,
        400
    );
    assert_eq!(
        first_snapshot.http_response.as_ref().unwrap().status_code,
        404
    );
}

#[tokio::test]
async fn admission_window_allows_reordered_opens_and_close_only_retires_its_identifier() {
    let sessions = ServerSessions::default();
    let lower = ServerSessionConfig {
        session_id: uuid::Uuid::from_u128(1).to_string(),
        ..config()
    };
    let cancelled = ServerSessionConfig {
        session_id: uuid::Uuid::from_u128(2).to_string(),
        ..config()
    };
    let higher = ServerSessionConfig {
        session_id: uuid::Uuid::from_u128(3).to_string(),
        ..config()
    };
    sessions.open(higher.clone()).await.unwrap();
    sessions.close(&cancelled.session_id).await.unwrap();
    sessions.open(lower.clone()).await.unwrap();
    assert!(sessions.open(cancelled).await.is_err());
    let distant = ServerSessionConfig {
        session_id: uuid::Uuid::from_u128(10_000).to_string(),
        ..config()
    };
    sessions.open(distant).await.unwrap();
    // Active retries remain recoverable even after their admission window rolls away.
    sessions.open(lower.clone()).await.unwrap();
    sessions.close(&lower.session_id).await.unwrap();
    assert!(sessions.open(lower).await.is_err());
    assert_eq!(sessions.shutdown().await, 2);
    let state = sessions.sessions.lock().await;
    assert!(state.active.is_empty());
    assert!(state.reserved.len() <= ADMISSION_WINDOW as usize);
}

#[tokio::test]
async fn closing_an_unknown_uuid_does_not_advance_the_admission_window() {
    let sessions = ServerSessions::default();
    sessions
        .close(&uuid::Uuid::from_u128(u128::MAX).to_string())
        .await
        .unwrap();

    let first = ServerSessionConfig {
        session_id: uuid::Uuid::from_u128(1).to_string(),
        ..config()
    };
    assert_eq!(
        sessions.open(first).await.unwrap(),
        uuid::Uuid::from_u128(1).to_string()
    );
}
