//! An ignored smoke test against the official MCP registry and a listed server.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

#[path = "support/live_official_registry.rs"]
mod support;

use serde_json::{Value, json};

/// Searches the official registry, installs its local npm calculator package,
/// and calls one deterministic computation. Requires network access and `npx`.
#[tokio::test]
#[ignore = "live: official MCP registry and npm package server"]
async fn live_official_registry_server_can_be_installed_connected_and_called() {
    const SERVER_NAME: &str = "io.github.headlessherm-creator/polymath-megablaster-mcp";

    let fixture = support::IsolatedRegistry::new();
    let registry = &fixture.registry;
    support::assert_search_lists(&fixture.catalog, &fixture.catalog_store, SERVER_NAME).await;

    let detail = fixture
        .catalog
        .get(
            &fixture.catalog_store,
            tinymcp::registry::sources::SOURCE_MCP_OFFICIAL,
            SERVER_NAME,
        )
        .await
        .expect("fetch official registry package details");
    assert!(
        detail.connections.iter().any(|connection| {
            connection.r#type == "stdio"
                && connection.example_config.as_ref().is_some_and(|config| {
                    config.get("command").and_then(Value::as_str) == Some("npx")
                })
        }),
        "{SERVER_NAME} no longer advertises its npm stdio package"
    );
    assert!(
        detail.connections.iter().all(|connection| {
            !matches!(connection.r#type.as_str(), "http" | "http_remote" | "sse")
                || connection.deployment_url.is_none()
        }),
        "{SERVER_NAME} unexpectedly advertises a hosted endpoint"
    );

    // The registry supplies an unversioned npx command, so npm resolves the
    // current package version when this live test installs it.
    let installed = registry
        .install(SERVER_NAME, fixture.server_env.clone(), None)
        .await
        .expect("install official npm package");
    let connected = registry
        .connect(&installed.server.server_id)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "connect installed package failed: {error}; child stderr: {}",
                support::child_diagnostics(&fixture.stderr_path)
            )
        });
    let calculate = connected
        .tools
        .iter()
        .find(|tool| tool.name == "calculate")
        .expect("Polymath package exposes its local `calculate` tool");
    assert_eq!(calculate.input_schema["type"], "object");
    assert!(
        calculate
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required.len() == 1 && required[0].as_str() == Some("expression")
            }),
        "local `calculate` tool's required argument schema changed: {}",
        calculate.input_schema
    );

    let result = registry
        .tool_call(
            &installed.server.server_id,
            "calculate",
            json!({ "expression": "2 + 2" }),
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "local `calculate` call failed: {error}; child stderr: {}",
                support::child_diagnostics(&fixture.stderr_path)
            )
        });
    assert!(
        !result.is_error,
        "local `calculate` returned an MCP error: {}; child stderr: {}",
        result.result,
        support::child_diagnostics(&fixture.stderr_path)
    );
    let text = result
        .result
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let calculation: Value =
        serde_json::from_str(text).expect("calculate tool response is structured JSON text");
    assert_eq!(
        calculation.get("result"),
        Some(&json!("4")),
        "`calculate` did not return 4 for 2 + 2: {text}"
    );

    registry
        .disconnect(&installed.server.server_id)
        .await
        .expect("disconnect installed package");
}
