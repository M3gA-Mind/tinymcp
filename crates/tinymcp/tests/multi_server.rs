//! Multiple installed MCP servers remain independent through the public API.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

#[path = "support/mcp_server.rs"]
mod mcp_server_fixture;

use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use tinymcp::registry::Store;
use tinymcp::{
    CommandKind, InstalledServer, McpClientIdentityConfig, McpRegistry, McpRegistryAuthConfig,
    ServerStatus, Transport,
};

use mcp_server_fixture::{
    mcp_server, mcp_server_with_reply, mcp_server_with_reply_and_initialization_failure,
};

fn registry_with_servers(servers: &[InstalledServer]) -> McpRegistry {
    let store = Store::open_in_memory().expect("open in-memory store");
    for server in servers {
        store.insert_server(server).expect("insert server");
    }
    McpRegistry::new(
        store,
        McpRegistryAuthConfig::default(),
        McpClientIdentityConfig::default(),
        None,
    )
    .expect("build registry")
}

fn remote_server(server_id: &str, qualified_name: &str, endpoint: &str) -> InstalledServer {
    InstalledServer {
        server_id: server_id.to_owned(),
        qualified_name: qualified_name.to_owned(),
        display_name: qualified_name.to_owned(),
        description: None,
        icon_url: None,
        command_kind: CommandKind::Node,
        command: String::new(),
        args: Vec::new(),
        env_keys: Vec::new(),
        config: None,
        installed_at: 1_000,
        last_connected_at: None,
        transport: Transport::HttpRemote {
            url: endpoint.to_owned(),
        },
        enabled: true,
    }
}

fn tool_text(result: &Value) -> &str {
    result["content"][0]["text"]
        .as_str()
        .expect("tool call returns a text block")
}

async fn call_forecast(registry: &McpRegistry, server_id: &str) -> String {
    let result = registry
        .tool_call(server_id, "forecast", json!({}))
        .await
        .expect("call forecast on connected server");
    assert!(!result.is_error, "the server answered successfully");
    tool_text(&result.result).to_owned()
}

fn assert_one_forecast_tool(tools: &[tinymcp::McpTool]) {
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "forecast");
}

#[tokio::test]
async fn same_named_tools_from_two_servers_keep_distinct_ids_and_routes() {
    let (endpoint_a, state_a) = mcp_server().await;
    let (endpoint_b, state_b) = mcp_server_with_reply("answer from B").await;
    let registry = registry_with_servers(&[
        remote_server("server-a", "com.example/a", &endpoint_a),
        remote_server("server-b", "com.example/b", &endpoint_b),
    ]);

    let connected_a = registry.connect("server-a").await.expect("connect A");
    let connected_b = registry.connect("server-b").await.expect("connect B");
    assert_ne!(connected_a.server_id, connected_b.server_id);
    assert_one_forecast_tool(&connected_a.tools);
    assert_one_forecast_tool(&connected_b.tools);

    let overviews = registry.connected_overview().await;
    assert!(overviews.iter().any(|server| {
        server.server_id == "server-a"
            && server.tools.len() == 1
            && server.tools[0].name == "forecast"
    }));
    assert!(overviews.iter().any(|server| {
        server.server_id == "server-b"
            && server.tools.len() == 1
            && server.tools[0].name == "forecast"
    }));

    assert_eq!(call_forecast(&registry, "server-a").await, "sunny");
    assert_eq!(call_forecast(&registry, "server-b").await, "answer from B");
    assert_eq!(state_a.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state_b.calls.load(Ordering::SeqCst), 1);

    registry.disconnect("server-a").await.expect("disconnect A");
    registry.disconnect("server-b").await.expect("disconnect B");
}

#[tokio::test]
async fn failed_connect_does_not_block_a_healthy_server() {
    let (endpoint, healthy_state) = mcp_server_with_reply("still healthy").await;
    let (unhealthy_endpoint, _unhealthy_state) =
        mcp_server_with_reply_and_initialization_failure("unavailable", true).await;

    let registry = registry_with_servers(&[
        remote_server("server-down", "com.example/down", &unhealthy_endpoint),
        remote_server("server-healthy", "com.example/healthy", &endpoint),
    ]);

    assert!(registry.connect("server-down").await.is_err());
    let connected = registry
        .connect("server-healthy")
        .await
        .expect("healthy server connects after peer failure");
    assert_one_forecast_tool(&connected.tools);

    let statuses = registry.status().await.expect("read server statuses");
    let down = statuses
        .iter()
        .find(|status| status.server_id == "server-down")
        .expect("failed server remains visible");
    let healthy = statuses
        .iter()
        .find(|status| status.server_id == "server-healthy")
        .expect("healthy server remains visible");
    assert_eq!(down.status, ServerStatus::Error);
    assert!(
        down.last_error
            .as_deref()
            .is_some_and(|error| !error.is_empty())
    );
    assert_eq!(healthy.status, ServerStatus::Connected);
    assert!(healthy.last_error.is_none());

    assert_eq!(
        call_forecast(&registry, "server-healthy").await,
        "still healthy"
    );
    assert_eq!(healthy_state.calls.load(Ordering::SeqCst), 1);
    registry
        .disconnect("server-healthy")
        .await
        .expect("disconnect healthy server");
}
