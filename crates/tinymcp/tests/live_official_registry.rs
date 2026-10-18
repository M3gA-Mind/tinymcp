//! An ignored smoke test against the official MCP registry and a listed server.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io;
use std::process::Stdio;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use tinymcp::registry::Store;
use tinymcp::transport::stdio::jailed::StdioLaunchers;
use tinymcp::transport::stdio::{StdioLaunch, StdioSpawner};
use tinymcp::{McpClientIdentityConfig, McpRegistry, McpRegistryAuthConfig};

#[derive(Debug)]
struct IsolatedSpawner;

static TEST_WORKSPACE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

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
            } else if let Some(workspace) = TEST_WORKSPACE.get() {
                command.current_dir(workspace);
            }
            for (key, value) in &launch.env {
                command.env(key, value);
            }
            command.spawn()
        })
    }
}

fn arguments_for(tool_schema: &Value) -> Option<Value> {
    let mut arguments = Map::new();
    let required = tool_schema
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let properties = tool_schema.get("properties");
    for name in required {
        let name = name.as_str()?;
        let schema = properties?.get(name)?;
        let value = if let Some(default) = schema.get("default") {
            default.clone()
        } else if let Some(constant) = schema.get("const") {
            constant.clone()
        } else if let Some(choice) = schema
            .get("enum")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        {
            choice.clone()
        } else {
            match schema.get("type").and_then(Value::as_str)? {
                "string" => json!("mcp registry smoke test"),
                "integer" => json!(1),
                "number" => json!(1.0),
                "boolean" => json!(false),
                "array" => json!([]),
                "object" => json!({}),
                _ => return None,
            }
        };
        arguments.insert(name.to_owned(), value);
    }
    Some(Value::Object(arguments))
}

fn is_read_only_smoke_tool(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["search", "echo", "ping", "health", "lookup"]
        .iter()
        .any(|kind| name.contains(kind))
}

/// Searches the official registry, installs a package server, and calls one
/// simple read-only tool. Requires network access and `npx` on `PATH`.
#[tokio::test]
#[ignore = "live: official MCP registry and npm package server"]
async fn live_official_registry_server_can_be_installed_connected_and_called() {
    let scratch = tempfile::tempdir().expect("create isolated npm home");
    TEST_WORKSPACE
        .set(scratch.path().to_path_buf())
        .expect("set isolated subprocess workspace");
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
        let tools: Vec<_> = connected
            .tools
            .iter()
            .filter(|tool| is_read_only_smoke_tool(&tool.name))
            .filter_map(|tool| arguments_for(&tool.input_schema).map(|args| (tool, args)))
            .collect();
        if tools.is_empty() {
            attempts.push(format!("{name}: no read-only tool has a simple schema"));
            registry
                .disconnect(&installed.server.server_id)
                .await
                .expect("disconnect server without a suitable tool");
            continue;
        }

        let mut answered = false;
        for (tool, arguments) in tools {
            match registry
                .tool_call(&installed.server.server_id, &tool.name, arguments.clone())
                .await
            {
                Ok(result) if !result.is_error => {
                    eprintln!("{name}: installed, connected, and called `{}`", tool.name);
                    answered = true;
                    break;
                }
                Ok(result) => {
                    let response = result
                        .result
                        .pointer("/content/0/text")
                        .and_then(Value::as_str)
                        .unwrap_or("no error text");
                    attempts.push(format!(
                        "{name}: tool `{}` returned an error (schema {}, arguments {}): {response}",
                        tool.name, tool.input_schema, arguments
                    ));
                }
                Err(error) => {
                    attempts.push(format!("{name}: tool `{}` failed: {error}", tool.name))
                }
            }
        }
        registry
            .disconnect(&installed.server.server_id)
            .await
            .expect("disconnect attempted server");
        if answered {
            return;
        }
    }

    panic!(
        "no package-only server from the official registry search could be installed and called:\n  {}",
        attempts.join("\n  ")
    );
}
