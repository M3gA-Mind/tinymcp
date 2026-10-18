//! An ignored smoke test against the official MCP registry and a listed server.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use tinymcp::registry::Store;
use tinymcp::transport::stdio::jailed::StdioLaunchers;
use tinymcp::transport::stdio::{DirectSpawner, StdioSpawner};
use tinymcp::{McpClientIdentityConfig, McpRegistry, McpRegistryAuthConfig};

/// Searches the official registry, installs a package server, and calls one
/// no-argument tool. Requires network access and `npx` on `PATH`.
#[tokio::test]
#[ignore = "live: official MCP registry and npm package server"]
async fn live_official_registry_server_can_be_installed_connected_and_called() {
    let registry = McpRegistry::new(
        Store::open_in_memory().expect("open in-memory store"),
        McpRegistryAuthConfig::default(),
        McpClientIdentityConfig::default(),
        None,
    )
    .expect("build registry")
    .with_stdio(StdioLaunchers::sandboxed(
        Arc::new(DirectSpawner) as Arc<dyn StdioSpawner>
    ));

    let search = registry
        .registry_search(Some("everything"), 1, 10)
        .await
        .expect("search official MCP registry");
    assert!(
        !search.servers.is_empty(),
        "official registry search returned no servers"
    );

    let mut attempts = Vec::new();
    for summary in &search.servers {
        let name = &summary.qualified_name;
        let (detail, _) = match registry.registry_get(name).await {
            Ok(detail) => detail,
            Err(error) => {
                attempts.push(format!("{name}: detail lookup failed: {error}"));
                continue;
            }
        };
        let has_stdio_package = detail
            .connections
            .iter()
            .any(|connection| connection.r#type == "stdio" && connection.example_config.is_some());
        let has_hosted_endpoint = detail.connections.iter().any(|connection| {
            matches!(connection.r#type.as_str(), "http" | "http_remote" | "sse")
                && connection.deployment_url.is_some()
        });
        if !has_stdio_package || has_hosted_endpoint {
            attempts.push(format!("{name}: no package-only stdio connection"));
            continue;
        }

        let installed = match registry.install(name, BTreeMap::new(), None).await {
            Ok(installed) => installed,
            Err(error) => {
                attempts.push(format!("{name}: install failed: {error}"));
                continue;
            }
        };
        let connected = match registry.connect(&installed.server.server_id).await {
            Ok(connected) => connected,
            Err(error) => {
                attempts.push(format!("{name}: connect failed: {error}"));
                continue;
            }
        };
        let tool = connected.tools.iter().find(|tool| {
            tool.input_schema
                .get("required")
                .and_then(serde_json::Value::as_array)
                .is_none_or(Vec::is_empty)
        });
        let Some(tool) = tool else {
            attempts.push(format!("{name}: no tool accepts empty arguments"));
            registry
                .disconnect(&installed.server.server_id)
                .await
                .expect("disconnect server without a callable tool");
            continue;
        };

        match registry
            .tool_call(&installed.server.server_id, &tool.name, json!({}))
            .await
        {
            Ok(result) if !result.is_error => {
                eprintln!("{name}: installed, connected, and called `{}`", tool.name);
                registry
                    .disconnect(&installed.server.server_id)
                    .await
                    .expect("disconnect live server");
                return;
            }
            Ok(result) => attempts.push(format!("{name}: tool `{}` returned an error", tool.name)),
            Err(error) => attempts.push(format!("{name}: tool call failed: {error}")),
        }
        registry
            .disconnect(&installed.server.server_id)
            .await
            .expect("disconnect attempted server");
    }

    panic!(
        "no package-only server from the official registry search could be installed and called:\n  {}",
        attempts.join("\n  ")
    );
}
