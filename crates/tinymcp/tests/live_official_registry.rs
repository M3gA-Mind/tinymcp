//! An ignored smoke test against the official MCP registry and a listed server.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tinymcp::registry::Store;
use tinymcp::transport::stdio::jailed::StdioLaunchers;
use tinymcp::transport::stdio::{StdioLaunch, StdioSpawner};
use tinymcp::{McpClientIdentityConfig, McpRegistry, McpRegistryAuthConfig};

#[derive(Debug)]
struct IsolatedSpawner {
    stderr_path: PathBuf,
}

static TEST_WORKSPACE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

impl StdioSpawner for IsolatedSpawner {
    fn spawn(&self, launch: StdioLaunch) -> BoxFuture<'_, io::Result<tokio::process::Child>> {
        let stderr_path = self.stderr_path.clone();
        Box::pin(async move {
            let stderr = File::create(stderr_path)?;
            let mut command = tokio::process::Command::new(&launch.program);
            command
                .args(&launch.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::from(stderr))
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

fn child_diagnostics(path: &std::path::Path) -> String {
    let Ok(mut stderr) = File::open(path) else {
        return "stderr was unavailable".to_owned();
    };
    let Ok(length) = stderr.metadata().map(|metadata| metadata.len()) else {
        return "stderr size was unavailable".to_owned();
    };
    let start = length.saturating_sub(8 * 1024);
    if stderr.seek(SeekFrom::Start(start)).is_err() {
        return "stderr could not be read".to_owned();
    }
    let mut bytes = Vec::with_capacity(8 * 1024);
    if stderr.take(8 * 1024).read_to_end(&mut bytes).is_err() {
        return "stderr could not be read".to_owned();
    }
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

/// Searches the official registry, installs its self-contained npm echo
/// package, and calls the read-only `get` tool. Requires network access and
/// `npx` on `PATH`.
#[tokio::test]
#[ignore = "live: official MCP registry and npm package server"]
async fn live_official_registry_server_can_be_installed_connected_and_called() {
    const SERVER_NAME: &str = "io.github.mrfentmen/postman-echo-mcp";

    let scratch = tempfile::tempdir().expect("create isolated npm home");
    TEST_WORKSPACE
        .set(scratch.path().to_path_buf())
        .expect("set isolated subprocess workspace");
    let npm_cache = scratch.path().join("npm-cache");
    let uv_cache = scratch.path().join("uv-cache");
    let xdg_cache = scratch.path().join("cache");
    let temp_dir = scratch.path().join("tmp");
    for directory in [&npm_cache, &uv_cache, &xdg_cache, &temp_dir] {
        std::fs::create_dir_all(directory)
            .expect("create isolated cache and temporary directories");
    }
    let npm_user_config = scratch.path().join("npmrc");
    std::fs::write(&npm_user_config, "").expect("create empty npm user config");
    let stderr_path = scratch.path().join("child-stderr.log");
    let isolated_spawner = Arc::new(IsolatedSpawner {
        stderr_path: stderr_path.clone(),
    });
    let server_env = BTreeMap::from([
        (
            "HOME".to_owned(),
            scratch.path().to_string_lossy().into_owned(),
        ),
        ("TMPDIR".to_owned(), temp_dir.to_string_lossy().into_owned()),
        ("TMP".to_owned(), temp_dir.to_string_lossy().into_owned()),
        ("TEMP".to_owned(), temp_dir.to_string_lossy().into_owned()),
        (
            "XDG_CACHE_HOME".to_owned(),
            xdg_cache.to_string_lossy().into_owned(),
        ),
        (
            "UV_CACHE_DIR".to_owned(),
            uv_cache.to_string_lossy().into_owned(),
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
    .with_stdio(StdioLaunchers::sandboxed(isolated_spawner));

    let first_page = registry
        .registry_search(Some("echo"), 1, 20)
        .await
        .expect("search official MCP registry for echo servers");
    let listed_on_first_page = first_page
        .servers
        .iter()
        .any(|server| server.qualified_name == SERVER_NAME);
    let listed = if listed_on_first_page {
        true
    } else {
        assert!(
            first_page.total_pages >= 2,
            "official registry search for `echo` did not list {SERVER_NAME}"
        );
        let second_page = registry
            .registry_search(Some("echo"), 2, 20)
            .await
            .expect("search second official registry page for echo servers");
        second_page
            .servers
            .iter()
            .any(|server| server.qualified_name == SERVER_NAME)
    };
    assert!(
        listed,
        "official registry search for `echo` did not list the verified package"
    );
    let qualified_name = SERVER_NAME;
    let (detail, _) = registry
        .registry_get(&qualified_name)
        .await
        .expect("fetch official registry package details");
    assert!(
        detail.connections.iter().any(|connection| {
            connection.r#type == "stdio"
                && connection.example_config.as_ref().is_some_and(|config| {
                    config.get("command").and_then(Value::as_str) == Some("npx")
                })
        }),
        "{qualified_name} no longer advertises its npm stdio package"
    );
    assert!(
        detail.connections.iter().all(|connection| {
            !matches!(connection.r#type.as_str(), "http" | "http_remote" | "sse")
                || connection.deployment_url.is_none()
        }),
        "{qualified_name} unexpectedly advertises a hosted endpoint"
    );

    let installed = registry
        .install(&qualified_name, server_env, None)
        .await
        .expect("install official npm package");
    let connected = registry
        .connect(&installed.server.server_id)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "connect installed package failed: {error}; child stderr: {}",
                child_diagnostics(&stderr_path)
            )
        });
    let get_tool = connected
        .tools
        .iter()
        .find(|tool| tool.name == "get")
        .expect("Postman Echo package exposes the read-only `get` tool");
    assert_eq!(get_tool.input_schema["type"], "object");
    assert!(
        get_tool
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty),
        "read-only `get` tool unexpectedly requires arguments: {}",
        get_tool.input_schema
    );
    let result = registry
        .tool_call(&installed.server.server_id, "get", json!({}))
        .await
        .unwrap_or_else(|error| {
            panic!(
                "read-only `get` call failed: {error}; child stderr: {}",
                child_diagnostics(&stderr_path)
            )
        });
    assert!(
        !result.is_error,
        "read-only `get` returned an MCP error: {}; child stderr: {}",
        result.result,
        child_diagnostics(&stderr_path)
    );
    registry
        .disconnect(&installed.server.server_id)
        .await
        .expect("disconnect installed package");
}
