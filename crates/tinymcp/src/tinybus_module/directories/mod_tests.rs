//! Unit tests for serving additional data directories.
//!
//! These run over a real in-memory bus: the property under test is that a path
//! `Open` returns is one a caller can actually reach, and that what it reaches
//! is isolated from every other directory. A test that only looked at the
//! returned string would agree with itself whatever the module did.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Error as BusError};
use tinymcp_bus::{
    DIRECTORY_OBJECT_PREFIX, INTERFACE, McpClientConfig, McpServerConfig, OBJECT_PATH, errors,
};

use super::MAX_OPEN_DIRECTORIES;
use crate::registry::SupervisorConfig;
use crate::tinybus_module::{McpService, ModuleConfig};

/// A connection on a fresh in-memory bus with the root object served.
async fn serve_root(config: &ModuleConfig) -> Connection {
    let bus = MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let _broker_task = broker.spawn(bus.clone());
    let connection = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();

    let service = McpService::new(config)
        .unwrap()
        .with_opener(connection.clone(), config, SupervisorConfig::default())
        .unwrap();
    connection
        .serve_at(OBJECT_PATH.try_into().unwrap(), service)
        .await
        .unwrap();
    connection.request_name(INTERFACE).await.unwrap();
    connection
}

/// Calls `member` on the object at `path`.
async fn call<R: serde::de::DeserializeOwned>(
    connection: &Connection,
    path: &str,
    member: &str,
    args: Value,
) -> Result<R, BusError> {
    connection
        .proxy(INTERFACE, path, INTERFACE)
        .unwrap()
        .call(member, args)
        .await
}

fn config_in(dir: &std::path::Path) -> ModuleConfig {
    ModuleConfig {
        data_dir: Some(dir.to_path_buf()),
        ..ModuleConfig::default()
    }
}

/// The name a failed call travelled under.
fn failure_name(error: &BusError) -> String {
    match error {
        BusError::MethodFailed { name, .. } => name.clone(),
        other => panic!("expected a method failure, got {other:?}"),
    }
}

fn audit_record(tool: &str) -> Value {
    json!({
        "timestamp_ms": 1_000,
        "client_info": "test",
        "tool_name": tool,
        "args_summary": {},
        "success": true,
    })
}

async fn audit_tools(connection: &Connection, path: &str) -> Vec<String> {
    let records: Vec<Value> = call(connection, path, "AuditListWrites", json!([{}]))
        .await
        .unwrap();
    records
        .iter()
        .map(|record| record["tool_name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn opening_a_directory_returns_a_path_that_answers() {
    let root_dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let connection = serve_root(&config_in(root_dir.path())).await;

    let path: String = call(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([other.path().to_str().unwrap()]),
    )
    .await
    .unwrap();

    assert!(path.starts_with(DIRECTORY_OBJECT_PREFIX), "{path}");
    let installed: Vec<Value> = call(&connection, &path, "InstalledList", json!([]))
        .await
        .unwrap();
    assert_eq!(installed.len(), 0);
    assert!(crate::Store::path_for(other.path()).exists());
}

#[tokio::test]
async fn configured_directories_keep_their_own_mocked_static_servers() {
    let root_dir = tempfile::tempdir().unwrap();
    let (alpha_dir, beta_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let alpha_url = mock_static_server("alpha provider").await;
    let beta_url = mock_static_server("beta provider").await;
    let connection = serve_root(&config_in(root_dir.path())).await;

    let alpha_path: String = call(
        &connection,
        OBJECT_PATH,
        "OpenConfigured",
        json!([configured_open(alpha_dir.path(), "alpha", &alpha_url)]),
    )
    .await
    .unwrap();
    let beta_path: String = call(
        &connection,
        OBJECT_PATH,
        "OpenConfigured",
        json!([configured_open(beta_dir.path(), "beta", &beta_url)]),
    )
    .await
    .unwrap();

    assert_ne!(alpha_path, beta_path);
    let alpha_names: Vec<String> = call(&connection, &alpha_path, "StaticList", json!([]))
        .await
        .unwrap();
    let beta_names: Vec<String> = call(&connection, &beta_path, "StaticList", json!([]))
        .await
        .unwrap();
    assert_eq!(alpha_names, ["alpha"]);
    assert_eq!(beta_names, ["beta"]);

    let alpha_result: Value = call(
        &connection,
        &alpha_path,
        "StaticCallTool",
        json!(["alpha", "identity", {}]),
    )
    .await
    .unwrap();
    let beta_result: Value = call(
        &connection,
        &beta_path,
        "StaticCallTool",
        json!(["beta", "identity", {}]),
    )
    .await
    .unwrap();

    assert_eq!(
        alpha_result["rendered"]["content"][0]["text"],
        "alpha provider"
    );
    assert_eq!(
        beta_result["rendered"]["content"][0]["text"],
        "beta provider"
    );
    assert!(crate::Store::path_for(alpha_dir.path()).exists());
    assert!(crate::Store::path_for(beta_dir.path()).exists());
}

#[tokio::test]
async fn reopening_one_directory_with_different_client_settings_is_refused() {
    let root_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let connection = serve_root(&config_in(root_dir.path())).await;
    let first_config = configured_open(data_dir.path(), "alpha", "http://127.0.0.1:1/");
    let first_path: String = call(
        &connection,
        OBJECT_PATH,
        "OpenConfigured",
        json!([first_config]),
    )
    .await
    .unwrap();
    let same_path: String = call(
        &connection,
        OBJECT_PATH,
        "OpenConfigured",
        json!([configured_open(
            data_dir.path(),
            "alpha",
            "http://127.0.0.1:1/"
        )]),
    )
    .await
    .unwrap();
    let error = call::<String>(
        &connection,
        OBJECT_PATH,
        "OpenConfigured",
        json!([configured_open(
            data_dir.path(),
            "beta",
            "http://127.0.0.1:2/"
        )]),
    )
    .await
    .unwrap_err();

    assert_eq!(first_path, same_path);
    assert_eq!(failure_name(&error), errors::INVALID_ARGUMENT);
    assert!(!error.to_string().contains("alpha"));
    let names: Vec<String> = call(&connection, &first_path, "StaticList", json!([]))
        .await
        .unwrap();
    assert_eq!(names, ["alpha"]);
}

fn configured_open(data_dir: &std::path::Path, name: &str, endpoint: &str) -> Value {
    json!({
        "data_dir": data_dir,
        "client": McpClientConfig {
            servers: vec![McpServerConfig {
                name: name.into(),
                endpoint: endpoint.into(),
                ..McpServerConfig::default()
            }],
            ..McpClientConfig::default()
        },
    })
}

async fn mock_static_server(reply: &'static str) -> String {
    use axum::Router;
    use axum::routing::post;

    let app = Router::new().route(
        "/",
        post(move |axum::Json(body): axum::Json<Value>| async move {
            let result = match body["method"].as_str().unwrap_or_default() {
                "initialize" => json!({
                    "protocolVersion": tinymcp_bus::LATEST_PROTOCOL_VERSION,
                    "capabilities": {},
                    "serverInfo": { "name": reply, "version": "1" },
                }),
                "tools/list" => json!({ "tools": [{ "name": "identity" }] }),
                "tools/call" => json!({
                    "content": [{ "type": "text", "text": reply }],
                    "isError": false,
                }),
                _ => json!({}),
            };
            axum::Json(json!({
                "jsonrpc": "2.0",
                "id": body["id"].clone(),
                "result": result,
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{address}/")
}

#[tokio::test]
async fn opening_the_same_directory_twice_returns_the_same_path() {
    let root_dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let connection = serve_root(&config_in(root_dir.path())).await;
    let arg = json!([other.path().to_str().unwrap()]);

    let first: String = call(&connection, OBJECT_PATH, "Open", arg.clone())
        .await
        .unwrap();
    let second: String = call(&connection, OBJECT_PATH, "Open", arg).await.unwrap();

    assert_eq!(first, second);
}

#[tokio::test]
async fn different_directories_get_different_paths() {
    let root_dir = tempfile::tempdir().unwrap();
    let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let connection = serve_root(&config_in(root_dir.path())).await;

    let first: String = call(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([one.path().to_str().unwrap()]),
    )
    .await
    .unwrap();
    let second: String = call(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([two.path().to_str().unwrap()]),
    )
    .await
    .unwrap();

    assert_ne!(first, second);
}

#[tokio::test]
async fn the_load_time_directory_answers_at_the_root_path() {
    let root_dir = tempfile::tempdir().unwrap();
    let connection = serve_root(&config_in(root_dir.path())).await;

    let path: String = call(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([root_dir.path().to_str().unwrap()]),
    )
    .await
    .unwrap();

    // Opening it again as a second object would put two handles on one file.
    assert_eq!(path, OBJECT_PATH);
}

#[tokio::test]
async fn each_directory_keeps_its_own_records() {
    let root_dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let connection = serve_root(&config_in(root_dir.path())).await;
    let opened: String = call(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([other.path().to_str().unwrap()]),
    )
    .await
    .unwrap();

    let _: i64 = call(
        &connection,
        &opened,
        "AuditRecordWrite",
        json!([audit_record("after_login")]),
    )
    .await
    .unwrap();
    let _: i64 = call(
        &connection,
        OBJECT_PATH,
        "AuditRecordWrite",
        json!([audit_record("before_login")]),
    )
    .await
    .unwrap();

    // The whole point of the seam: a call after the switch is answered from the
    // new directory's store, not the one the module was loaded with.
    assert_eq!(audit_tools(&connection, &opened).await, ["after_login"]);
    assert_eq!(
        audit_tools(&connection, OBJECT_PATH).await,
        ["before_login"]
    );
}

#[tokio::test]
async fn a_relative_or_empty_path_is_refused_by_name() {
    let connection = serve_root(&ModuleConfig::default()).await;

    for bad in ["", "relative/dir", "./here"] {
        let error = call::<String>(&connection, OBJECT_PATH, "Open", json!([bad]))
            .await
            .unwrap_err();

        assert_eq!(failure_name(&error), errors::INVALID_ARGUMENT, "{bad:?}");
    }
}

#[tokio::test]
async fn a_refusal_never_echoes_the_path() {
    let connection = serve_root(&ModuleConfig::default()).await;

    let error = call::<String>(
        &connection,
        OBJECT_PATH,
        "Open",
        json!(["relative/secret-user"]),
    )
    .await
    .unwrap_err();

    assert!(!error.to_string().contains("secret-user"), "{error}");
}

#[tokio::test]
async fn an_opened_directory_cannot_open_further_directories() {
    let root_dir = tempfile::tempdir().unwrap();
    let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let connection = serve_root(&config_in(root_dir.path())).await;
    let opened: String = call(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([one.path().to_str().unwrap()]),
    )
    .await
    .unwrap();

    let error = call::<String>(
        &connection,
        &opened,
        "Open",
        json!([two.path().to_str().unwrap()]),
    )
    .await
    .unwrap_err();

    assert_eq!(failure_name(&error), errors::INVALID_ARGUMENT);
}

#[tokio::test]
async fn a_service_that_is_not_a_root_refuses_to_open() {
    let connection = serve_root(&ModuleConfig::default()).await;
    // Built the way `Open` builds its objects: no opener.
    connection
        .serve_at(
            "/ai/tinyhumans/tinymcp/Plain".try_into().unwrap(),
            McpService::new(&ModuleConfig::default()).unwrap(),
        )
        .await
        .unwrap();

    let error = call::<String>(
        &connection,
        "/ai/tinyhumans/tinymcp/Plain",
        "Open",
        json!(["/somewhere"]),
    )
    .await
    .unwrap_err();

    assert_eq!(failure_name(&error), errors::INVALID_ARGUMENT);
}

#[tokio::test]
async fn the_number_of_served_directories_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let connection = serve_root(&config_in(root.path())).await;
    let parent = tempfile::tempdir().unwrap();

    // The root's own directory already counts as one.
    for index in 1..MAX_OPEN_DIRECTORIES {
        let dir = parent.path().join(index.to_string());
        let _: String = call(
            &connection,
            OBJECT_PATH,
            "Open",
            json!([dir.to_str().unwrap()]),
        )
        .await
        .unwrap();
    }

    let overflow = parent.path().join("one-too-many");
    let error = call::<String>(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([overflow.to_str().unwrap()]),
    )
    .await
    .unwrap_err();
    assert_eq!(failure_name(&error), errors::INVALID_ARGUMENT);
    assert!(!overflow.exists(), "a refused open must create nothing");

    // Directories already served still answer.
    let again: String = call(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([parent.path().join("1").to_str().unwrap()]),
    )
    .await
    .unwrap();
    assert!(again.starts_with(DIRECTORY_OBJECT_PREFIX));
}

#[tokio::test]
async fn opener_debug_output_does_not_expose_directory_or_client_data() {
    let connection = serve_root(&ModuleConfig::default()).await;
    let opener = super::DirectoryOpener::new(
        connection,
        &ModuleConfig::default(),
        SupervisorConfig::default(),
    )
    .unwrap();
    let debug = format!("{opener:?}");
    assert!(debug.contains("DirectoryOpener"));
    assert!(!debug.contains("data_dir"));
}

#[tokio::test]
async fn opening_a_directory_propagates_store_creation_errors() {
    let connection = serve_root(&ModuleConfig::default()).await;
    let opener = super::DirectoryOpener::new(
        connection,
        &ModuleConfig::default(),
        SupervisorConfig::default(),
    )
    .unwrap();
    let parent = tempfile::NamedTempFile::new().unwrap();
    let path = parent.path().join("not-a-directory");
    let error = opener.open(path.to_str().unwrap()).await.unwrap_err();
    assert!(matches!(
        error,
        crate::Error::StoreIo { .. } | crate::Error::Store { .. }
    ));
}

#[tokio::test]
async fn an_absolute_open_matches_a_relative_load_time_directory() {
    let relative_root = tempfile::tempdir_in(".").unwrap();
    let absolute = relative_root.path().canonicalize().unwrap();
    let relative = absolute
        .strip_prefix(std::env::current_dir().unwrap())
        .unwrap();
    let connection = serve_root(&config_in(relative)).await;
    let path: String = call(
        &connection,
        OBJECT_PATH,
        "Open",
        json!([absolute.to_str().unwrap()]),
    )
    .await
    .unwrap();
    assert_eq!(path, OBJECT_PATH);
}

#[test]
fn serving_failures_keep_the_bus_reason() {
    let error = super::types::serving_error("path already served");
    assert_eq!(
        error.to_string(),
        "bus failure: could not serve the directory: path already served"
    );
}

#[test]
fn configured_directory_paths_are_normalized_to_absolute_paths() {
    let path = super::super::service::absolute_data_dir(std::path::Path::new("relative")).unwrap();
    assert!(path.is_absolute());
}
