//! The public stores, driven only through the storage ports.
//!
//! The module links no database: a host hands it ports, and what one store
//! writes, a store reopened over the same ports reads back. These tests use the
//! in-memory backend and never touch the filesystem.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use tinymcp::storage::{self, StoragePorts};
use tinymcp::{AuditStore, CommandKind, InstalledServer, NewMcpWriteRecord, Store, Transport};
use tinystoragedrivers_testkit::memory_storage;
use tinystoragedrivers_types::Scope;

fn memory_ports() -> StoragePorts {
    let scope = Scope::new("test").unwrap();
    let storage = memory_storage(&scope);
    StoragePorts::from_storage(&storage, std::sync::Arc::clone(storage.secrets()))
}

fn server(id: &str, name: &str) -> InstalledServer {
    InstalledServer {
        server_id: id.to_string(),
        qualified_name: name.to_string(),
        display_name: "Weather".to_string(),
        description: Some("Forecasts".to_string()),
        icon_url: None,
        command_kind: CommandKind::Node,
        command: "weather-mcp".to_string(),
        args: vec!["--verbose".to_string()],
        env_keys: vec!["API_KEY".to_string()],
        config: None,
        installed_at: 1_700_000_000,
        last_connected_at: Some(1_700_000_100),
        transport: Transport::Stdio,
        enabled: true,
    }
}

#[test]
fn installs_and_credentials_survive_reopening_over_the_same_ports() {
    let ports = memory_ports();
    let store = Store::open(ports.clone()).unwrap();
    store
        .insert_server(&server("srv-1", "@acme/weather"))
        .unwrap();
    store
        .set_env_values(
            "srv-1",
            &BTreeMap::from([("API_KEY".to_string(), "sekrit".to_string())]),
        )
        .unwrap();
    drop(store);

    let reopened = Store::open(ports).unwrap();
    let loaded = reopened.get_server("srv-1").unwrap();
    assert_eq!(loaded, server("srv-1", "@acme/weather"));
    assert_eq!(
        reopened.load_env_values("srv-1").unwrap().get("API_KEY"),
        Some(&"sekrit".to_string())
    );
}

#[test]
fn the_audit_log_and_the_servers_share_ports_but_not_records() {
    let ports = memory_ports();
    let store = Store::open(ports.clone()).unwrap();
    let audit = AuditStore::open(ports).unwrap();
    store
        .insert_server(&server("srv-1", "@acme/weather"))
        .unwrap();
    audit
        .record(&NewMcpWriteRecord {
            timestamp_ms: 10,
            client_info: "claude".into(),
            tool_name: "memory_write".into(),
            args_summary: serde_json::json!({}),
            resulting_chunk_id: None,
            success: true,
            error_message: None,
        })
        .unwrap();

    assert_eq!(store.list_servers().unwrap().len(), 1);
    assert_eq!(
        audit
            .list(&tinymcp::McpWriteListQuery::default())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn the_declared_grant_covers_the_collections_a_store_creates() {
    let granted = storage::grant();
    for spec in storage::collection_specs() {
        assert!(granted.collections.contains(&spec.name));
    }
    assert!(granted.streams.contains(&storage::AUDIT_STREAM.to_string()));
}

#[test]
fn the_storage_schema_composes_and_the_grant_is_generated_from_it() {
    let collections: Vec<_> = storage::collection_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    let granted = storage::grant();
    assert_eq!(granted.collections, collections);
    assert_eq!(granted.streams, vec![storage::AUDIT_STREAM.to_owned()]);
    assert!(
        collections
            .iter()
            .any(|name| name == tinymcp_bus::storage::MIGRATIONS)
    );
    assert!(
        collections
            .iter()
            .filter(|name| name.as_str() != tinymcp_bus::storage::MIGRATIONS)
            .chain(&granted.streams)
            .all(|name| name.starts_with(&format!("{}.", storage::OWNER)))
    );
}
