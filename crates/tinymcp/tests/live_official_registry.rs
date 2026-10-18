//! An ignored smoke test against the official MCP registry and a listed server.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io;
use std::process::Stdio;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::json;
use tinymcp::registry::Store;
use tinymcp::transport::stdio::jailed::StdioLaunchers;
use tinymcp::transport::stdio::{StdioLaunch, StdioSpawner};
use tinymcp::{McpClientIdentityConfig, McpRegistry, McpRegistryAuthConfig};

#[derive(Debug)]
struct IsolatedSpawner;

impl StdioSpawner for IsolatedSpawner {
    fn spawn(&self, launch: StdioLaunch) -> BoxFuture<'_, io::Result<tokio::process::Child>> {
        Box::pin(async move {
            let mut command = tokio::process::Command::new(&launch.program);
            command
                .args(&launch.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .env_clear();
            if let Some(cwd) = launch.cwd.as_ref() {
                command.current_dir(cwd);
            }
            for (key, value) in &launch.env {
                command.env(key, value);
            }
            command.spawn()
        })
    }
}

/// Searches the official registry, installs a package server, and calls one
/// no-argument tool. Requires network access and `npx` on `PATH`.
#[tokio::test]
#[ignore = "live: official MCP registry and npm package server"]
async fn live_official_registry_server_can_be_installed_connected_and_called() {
    let scratch = tempfile::tempdir().expect("create isolated npm home");
    let npm_cache = scratch.path().join("npm-cache");
    std::fs::create_dir_all(&npm_cache).expect("create isolated npm cache");
    let npm_user_config = scratch.path().join("npmrc");
    std::fs::write(&npm_user_config, "").expect("create empty npm user config");
    let server_env = BTreeMap::from([
        (
            "HOME".to_owned(),
            scratch.path().to_string_lossy().into_owned(),
        ),
        (
            "NPM_CONFIG_CACHE".to_owned(),
            npm_cache.to_string_lossy().into_owned(),
        ),
        (
            "NPM_CONFIG_USERCONFIG".to_owned(),
            npm_user_config.to_string_lossy().into_owned(),
        ),
        (
            "PATH".to_owned(),
            std::env::var("PATH").expect("PATH is needed to resolve npx"),
        ),
    ]);
    let registry = McpRegistry::new(
        Store::open_in_memory().expect("open in-memory store"),
        McpRegistryAuthConfig::default(),
        McpClientIdentityConfig::default(),
        None,
    )
    .expect("build registry")
    .with_stdio(StdioLaunchers::sandboxed(Arc::new(IsolatedSpawner)));

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

        let installed = match registry.install(name, server_env.clone(), None).await {
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
            Ok(_) => attempts.push(format!("{name}: tool `{}` returned an error", tool.name)),
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
