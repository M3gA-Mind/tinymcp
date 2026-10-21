//! A loopback Streamable HTTP server shared by registry tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tinymcp_bus::LATEST_PROTOCOL_VERSION;

/// What the loopback server observed.
#[derive(Debug, Default)]
pub(crate) struct Server {
    /// Number of tool calls received.
    pub(crate) calls: AtomicUsize,
    /// The credential carried by the last request, if any.
    pub(crate) authorization: Mutex<Option<String>>,
    /// Whether requests without authorization should receive 401.
    pub(crate) demand_auth: AtomicBool,
    /// Arguments from the last `tools/call` request.
    pub(crate) last_arguments: Mutex<Option<Value>>,
    /// Text returned by `forecast`.
    pub(crate) reply: Mutex<String>,
    /// Whether initialization should be refused with a server error.
    pub(crate) reject_initialization: bool,
}

/// Starts a loopback MCP server with the shared registry-test protocol.
pub(crate) async fn mcp_server() -> (String, Arc<Server>) {
    mcp_server_with_reply("sunny").await
}

/// Starts a loopback MCP server whose `forecast` tool returns `reply`.
pub(crate) async fn mcp_server_with_reply(reply: &str) -> (String, Arc<Server>) {
    mcp_server_with_reply_and_initialization_failure(reply, false).await
}

/// Starts a loopback MCP server whose initialization optionally returns 500.
pub(crate) async fn mcp_server_with_reply_and_initialization_failure(
    reply: &str,
    reject_initialization: bool,
) -> (String, Arc<Server>) {
    let state = Arc::new(Server {
        reply: Mutex::new(reply.to_owned()),
        reject_initialization,
        ..Server::default()
    });
    serve(state).await
}

#[allow(clippy::expect_used, clippy::panic)]
async fn serve(state: Arc<Server>) -> (String, Arc<Server>) {
    let app = Router::new()
        .route(
            "/mcp",
            post(handle).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback MCP server");
    let addr = listener.local_addr().expect("read loopback address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve MCP requests");
    });
    (format!("http://{addr}/mcp"), state)
}

async fn handle(
    State(state): State<Arc<Server>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    *state.authorization.lock() = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    if state.demand_auth.load(Ordering::SeqCst) && state.authorization.lock().is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            [("WWW-Authenticate", "Bearer resource_metadata=\"https://example.test/.well-known/oauth-protected-resource\"")],
            "unauthorized",
        )
            .into_response();
    }

    let id = body["id"].clone();
    if body.get("method").and_then(Value::as_str) == Some("initialize")
        && state.reject_initialization
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    match body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "initialize" => (
            [("Mcp-Session-Id", "session-1")],
            axum::Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": LATEST_PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "loopback", "version": "1.0.0" },
                },
            })),
        )
            .into_response(),
        "notifications/initialized" => StatusCode::NO_CONTENT.into_response(),
        "tools/list" => axum::Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [{
                    "name": "forecast",
                    "description": "tomorrow's weather",
                    "inputSchema": { "type": "object" },
                }],
            },
        }))
        .into_response(),
        "tools/call" => {
            state.calls.fetch_add(1, Ordering::SeqCst);
            *state.last_arguments.lock() = body["params"].get("arguments").cloned();
            let reply = state.reply.lock().clone();
            axum::Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "content": [{ "type": "text", "text": reply }] },
            }))
            .into_response()
        }
        other => (StatusCode::BAD_REQUEST, format!("unexpected {other}")).into_response(),
    }
}
