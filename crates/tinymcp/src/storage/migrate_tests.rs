//! Tests for the v1 to v2 steps, run through the registry over v1-shaped fixtures.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::storage::StoragePorts;
use crate::storage::AUDIT_STREAM;
use crate::storage::local::schema::MigrationReport;
use crate::storage::local::{MigrateOptions, Migrations, Schema};
use crate::storage::migrate;
use crate::storage::types::{
    CollectionSpec, DocumentStoreExt as _, Filter, Precondition, Query, Scope, Versioned,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tinystoragedrivers_api::StreamStore;
use tinystoragedrivers_api::{Storage, StorageBackend};
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::StreamEntry;

use super::*;
use crate::audit::AuditStore;
use crate::registry::store::Store;
use crate::storage::schema::{OWNER, storage_schema};
use tinymcp_bus::storage::SCHEMA_VERSION;

/// A memory backend holding v1 data, and the registry that migrates it.
struct Fixture {
    blocking: tokio::runtime::Runtime,
    storage: Storage,
    schema: Schema,
}

impl Fixture {
    fn new() -> Self {
        let blocking = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let backend = MemoryBackend::new();
        let scope = Scope::new("tinymcp").unwrap();
        let storage = backend.for_scope(&scope).unwrap();
        let schema = storage_schema().unwrap();
        Self {
            blocking,
            storage,
            schema,
        }
    }

    /// Writes one v1 document the way the previous release did.
    fn legacy(&self, collection: &str, id: &str, body: Value) {
        let documents = self.storage.documents().clone();
        let collection = collection.to_string();
        let id = id.to_string();
        self.blocking
            .block_on(async move {
                documents
                    .ensure_collection(&CollectionSpec::new(&collection))
                    .await?;
                documents
                    .put(&collection, &id, body, Precondition::Absent)
                    .await
            })
            .unwrap();
    }

    fn legacy_stream(&self, name: &str, entries: &[Value]) {
        let streams = self.storage.streams().clone();
        let name = name.to_string();
        let entries = entries.to_vec();
        self.blocking
            .block_on(async move {
                for entry in entries {
                    streams.append(&name, entry).await?;
                }
                Ok::<_, crate::storage::types::StorageError>(())
            })
            .unwrap();
    }

    fn migrate(&self) -> MigrationReport {
        let documents = Arc::clone(self.storage.documents());
        let streams = Arc::clone(self.storage.streams());
        self.blocking
            .block_on(async {
                let report = self
                    .schema
                    .migrate(&documents, &MigrateOptions::new())
                    .await?;
                let migrations = Migrations::new(&documents, OWNER);
                migrate::migrate_audit_streams(&*streams, &migrations).await?;
                Ok::<_, crate::storage::types::StorageError>(report)
            })
            .unwrap()
    }

    fn get(&self, collection: &str, id: &str) -> Option<Versioned<Value>> {
        let documents = self.storage.documents().clone();
        let collection = collection.to_string();
        let id = id.to_string();
        self.blocking
            .block_on(async move { documents.get(&collection, &id).await })
            .unwrap()
    }

    fn all(&self, collection: &str) -> Vec<Versioned<Value>> {
        let documents = self.storage.documents().clone();
        let collection = collection.to_string();
        self.blocking
            .block_on(async move { documents.query_all(&collection, &Query::all()).await })
            .unwrap()
    }

    fn count_where(&self, collection: &str, filter: Filter) -> u64 {
        let documents = self.storage.documents().clone();
        let collection = collection.to_string();
        self.blocking
            .block_on(async move { documents.count(&collection, &filter).await })
            .unwrap()
    }

    fn stream(&self, name: &str) -> Vec<Value> {
        let streams = self.storage.streams().clone();
        let name = name.to_string();
        self.blocking
            .block_on(async move {
                let len = streams.len(&name).await?;
                let window = streams.read_window(&name, 0, 1024).await?;
                assert_eq!(window.len() as u64, len);
                Ok::<Vec<Value>, crate::storage::types::StorageError>(
                    window.into_iter().map(|entry| entry.value).collect(),
                )
            })
            .unwrap()
    }
}

/// Persists the first migrated batch, then simulates the caller losing the
/// acknowledgement before its progress marker can be written.
#[derive(Debug)]
struct FailAfterFirstAppend {
    inner: Arc<dyn StreamStore>,
    fail: AtomicBool,
}

#[tinystoragedrivers_api::async_trait]
impl StreamStore for FailAfterFirstAppend {
    fn capabilities(&self) -> tinystoragedrivers_types::Capabilities {
        self.inner.capabilities()
    }

    async fn append(&self, stream: &str, value: Value) -> crate::storage::types::Result<u64> {
        self.inner.append(stream, value).await
    }

    async fn append_batch(
        &self,
        stream: &str,
        values: Vec<Value>,
    ) -> crate::storage::types::Result<u64> {
        let first = self.inner.append_batch(stream, values).await?;
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(crate::storage::types::StorageError::unavailable(
                "lost append acknowledgement",
            ));
        }
        Ok(first)
    }

    async fn append_batch_if_offset(
        &self,
        stream: &str,
        expected_next_offset: u64,
        values: Vec<Value>,
    ) -> crate::storage::types::Result<u64> {
        let first = self
            .inner
            .append_batch_if_offset(stream, expected_next_offset, values)
            .await?;
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(crate::storage::types::StorageError::unavailable(
                "lost conditional append acknowledgement",
            ));
        }
        Ok(first)
    }

    async fn read_window(
        &self,
        stream: &str,
        from: u64,
        limit: usize,
    ) -> crate::storage::types::Result<Vec<StreamEntry>> {
        self.inner.read_window(stream, from, limit).await
    }

    async fn len(&self, stream: &str) -> crate::storage::types::Result<u64> {
        self.inner.len(stream).await
    }

    async fn truncate_before(
        &self,
        stream: &str,
        offset: u64,
    ) -> crate::storage::types::Result<u64> {
        self.inner.truncate_before(stream, offset).await
    }

    async fn delete_stream(&self, stream: &str) -> crate::storage::types::Result<bool> {
        self.inner.delete_stream(stream).await
    }

    async fn streams(&self, prefix: &str) -> crate::storage::types::Result<Vec<String>> {
        self.inner.streams(prefix).await
    }
}

/// Makes one nonempty legacy stream report an empty page before its declared
/// end, as a failing backend can do during a read.
#[derive(Debug)]
struct EmptyFirstRead {
    inner: Arc<dyn StreamStore>,
    fail: AtomicBool,
}

#[tinystoragedrivers_api::async_trait]
impl StreamStore for EmptyFirstRead {
    fn capabilities(&self) -> tinystoragedrivers_types::Capabilities {
        self.inner.capabilities()
    }

    async fn append(&self, stream: &str, value: Value) -> crate::storage::types::Result<u64> {
        self.inner.append(stream, value).await
    }

    async fn append_batch(
        &self,
        stream: &str,
        values: Vec<Value>,
    ) -> crate::storage::types::Result<u64> {
        self.inner.append_batch(stream, values).await
    }

    async fn append_batch_if_offset(
        &self,
        stream: &str,
        expected_next_offset: u64,
        values: Vec<Value>,
    ) -> crate::storage::types::Result<u64> {
        self.inner
            .append_batch_if_offset(stream, expected_next_offset, values)
            .await
    }

    async fn read_window(
        &self,
        stream: &str,
        from: u64,
        limit: usize,
    ) -> crate::storage::types::Result<Vec<StreamEntry>> {
        if stream == "mcp_audit" && self.fail.swap(false, Ordering::SeqCst) {
            return Ok(Vec::new());
        }
        self.inner.read_window(stream, from, limit).await
    }

    async fn len(&self, stream: &str) -> crate::storage::types::Result<u64> {
        self.inner.len(stream).await
    }

    async fn truncate_before(
        &self,
        stream: &str,
        offset: u64,
    ) -> crate::storage::types::Result<u64> {
        self.inner.truncate_before(stream, offset).await
    }

    async fn delete_stream(&self, stream: &str) -> crate::storage::types::Result<bool> {
        self.inner.delete_stream(stream).await
    }

    async fn streams(&self, prefix: &str) -> crate::storage::types::Result<Vec<String>> {
        self.inner.streams(prefix).await
    }
}

/// Adds an entry to the source after its length was read but before the page
/// containing its original contents is returned.
#[derive(Debug)]
struct AppendDuringRead {
    inner: Arc<dyn StreamStore>,
    append: AtomicBool,
}

/// Models a second, independent opener winning the target append after this
/// importer reads its checkpoint but before its own conditional append.
#[derive(Debug)]
struct CompetingAuditAppend {
    inner: Arc<dyn StreamStore>,
    raced: AtomicBool,
    competing_batch: Option<Vec<Value>>,
}

#[tinystoragedrivers_api::async_trait]
impl StreamStore for CompetingAuditAppend {
    fn capabilities(&self) -> tinystoragedrivers_types::Capabilities {
        self.inner.capabilities()
    }

    async fn append(&self, stream: &str, value: Value) -> crate::storage::types::Result<u64> {
        self.inner.append(stream, value).await
    }

    async fn append_batch(
        &self,
        stream: &str,
        values: Vec<Value>,
    ) -> crate::storage::types::Result<u64> {
        // This is the pre-fix importer path. Make the competing opener append
        // first, then allow the unconditional append to expose duplication.
        if stream == AUDIT_STREAM && !self.raced.swap(true, Ordering::SeqCst) {
            self.inner
                .append_batch(
                    stream,
                    self.competing_batch
                        .clone()
                        .unwrap_or_else(|| values.clone()),
                )
                .await?;
        }
        self.inner.append_batch(stream, values).await
    }

    async fn append_batch_if_offset(
        &self,
        stream: &str,
        expected_next_offset: u64,
        values: Vec<Value>,
    ) -> crate::storage::types::Result<u64> {
        if stream == AUDIT_STREAM && !self.raced.swap(true, Ordering::SeqCst) {
            // A second independently bound port handle wins at precisely the
            // expected target offset. The current importer must recognize the
            // exact batch after its conditional append reports a conflict.
            self.inner
                .append_batch_if_offset(
                    stream,
                    expected_next_offset,
                    self.competing_batch
                        .clone()
                        .unwrap_or_else(|| values.clone()),
                )
                .await?;
        }
        self.inner
            .append_batch_if_offset(stream, expected_next_offset, values)
            .await
    }

    async fn read_window(
        &self,
        stream: &str,
        from: u64,
        limit: usize,
    ) -> crate::storage::types::Result<Vec<StreamEntry>> {
        self.inner.read_window(stream, from, limit).await
    }

    async fn len(&self, stream: &str) -> crate::storage::types::Result<u64> {
        self.inner.len(stream).await
    }

    async fn truncate_before(
        &self,
        stream: &str,
        offset: u64,
    ) -> crate::storage::types::Result<u64> {
        self.inner.truncate_before(stream, offset).await
    }

    async fn delete_stream(&self, stream: &str) -> crate::storage::types::Result<bool> {
        self.inner.delete_stream(stream).await
    }

    async fn streams(&self, prefix: &str) -> crate::storage::types::Result<Vec<String>> {
        self.inner.streams(prefix).await
    }
}

#[tinystoragedrivers_api::async_trait]
impl StreamStore for AppendDuringRead {
    fn capabilities(&self) -> tinystoragedrivers_types::Capabilities {
        self.inner.capabilities()
    }

    async fn append(&self, stream: &str, value: Value) -> crate::storage::types::Result<u64> {
        self.inner.append(stream, value).await
    }

    async fn append_batch(
        &self,
        stream: &str,
        values: Vec<Value>,
    ) -> crate::storage::types::Result<u64> {
        self.inner.append_batch(stream, values).await
    }

    async fn append_batch_if_offset(
        &self,
        stream: &str,
        expected_next_offset: u64,
        values: Vec<Value>,
    ) -> crate::storage::types::Result<u64> {
        self.inner
            .append_batch_if_offset(stream, expected_next_offset, values)
            .await
    }

    async fn read_window(
        &self,
        stream: &str,
        from: u64,
        limit: usize,
    ) -> crate::storage::types::Result<Vec<StreamEntry>> {
        let page = self.inner.read_window(stream, from, limit).await?;
        if stream == "mcp_audit" && self.append.swap(false, Ordering::SeqCst) {
            self.inner
                .append(stream, json!({"timestamp_ms": 2}))
                .await?;
        }
        Ok(page)
    }

    async fn len(&self, stream: &str) -> crate::storage::types::Result<u64> {
        self.inner.len(stream).await
    }

    async fn truncate_before(
        &self,
        stream: &str,
        offset: u64,
    ) -> crate::storage::types::Result<u64> {
        self.inner.truncate_before(stream, offset).await
    }

    async fn delete_stream(&self, stream: &str) -> crate::storage::types::Result<bool> {
        self.inner.delete_stream(stream).await
    }

    async fn streams(&self, prefix: &str) -> crate::storage::types::Result<Vec<String>> {
        self.inner.streams(prefix).await
    }
}

fn v1_server() -> Value {
    json!({
        "server_id": "srv-1",
        "qualified_name": "@acme/files",
        "display_name": "Files",
        "command_kind": "node",
        "command": "npx",
        "args": ["-y", "files"],
        "env_keys": ["TOKEN"],
        "installed_at": 1_700_000_000_000_i64,
        "last_connected_at": 1_700_000_100_000_i64,
        "transport": "stdio",
        "enabled": true,
        "catalog_managed": true,
    })
}

#[test]
fn a_v1_server_moves_to_the_v2_collection_with_ms_time_fields() {
    let fixture = Fixture::new();
    fixture.legacy("mcp_servers", "srv-1", v1_server());

    fixture.migrate();

    let moved = fixture.get(SERVERS, "srv-1").expect("the server moved");
    assert_eq!(moved.doc["installed_ms"], 1_700_000_000_000_i64);
    assert_eq!(moved.doc["last_connected_ms"], 1_700_000_100_000_i64);
    assert!(moved.doc.get("installed_at").is_none());
    assert!(moved.doc.get("last_connected_at").is_none());
    assert_eq!(moved.doc["qualified_name"], "@acme/files");
    assert_eq!(moved.doc["catalog_managed"], true);
    assert_eq!(moved.doc["args"], json!(["-y", "files"]));
    assert_eq!(moved.meta.schema, SCHEMA_VERSION);
    assert_eq!(fixture.all("mcp_servers").len(), 0);
}

#[test]
fn opening_the_registry_store_runs_the_owner_document_migrations() {
    let fixture = Fixture::new();
    fixture.legacy("mcp_servers", "srv-1", v1_server());
    let ports = crate::storage::StoragePorts::from_storage(
        &fixture.storage,
        Arc::clone(fixture.storage.secrets()),
    );

    let _store = Store::open(ports).unwrap();

    let moved = fixture
        .get(SERVERS, "srv-1")
        .expect("owner open migrated the row");
    assert_eq!(moved.doc["installed_ms"], 1_700_000_000_000_i64);
    assert_eq!(fixture.all("mcp_servers").len(), 0);
}

#[test]
fn previous_release_public_writer_data_is_readable_after_owner_open() {
    let fixture = Fixture::new();
    let released_document: Value =
        serde_json::from_str(include_str!("fixtures/tinymcp-v0.19.1-server.json")).unwrap();
    fixture.legacy(SERVERS, "released-fixture-server", released_document);
    let ports = crate::storage::StoragePorts::from_storage(
        &fixture.storage,
        Arc::clone(fixture.storage.secrets()),
    );

    let store = Store::open(ports).unwrap();
    let loaded = store.get_server("released-fixture-server").unwrap();

    assert_eq!(loaded.qualified_name, "@test/server");
    assert_eq!(loaded.display_name, "Test Server");
    assert_eq!(loaded.command, "npx");
    assert_eq!(loaded.args, ["-y", "@test/server"]);
    assert_eq!(loaded.env_keys, ["API_KEY"]);
    assert_eq!(loaded.installed_at, 1_700_000_000_000);
    assert!(loaded.enabled);
    assert_eq!(loaded.transport, tinymcp_bus::Transport::Stdio);
}

#[test]
fn a_server_never_connected_keeps_no_last_connected_field() {
    let fixture = Fixture::new();
    let mut server = v1_server();
    server.as_object_mut().unwrap().remove("last_connected_at");
    fixture.legacy("mcp_servers", "srv-1", server);

    fixture.migrate();

    let moved = fixture.get(SERVERS, "srv-1").unwrap();
    assert!(moved.doc.get("last_connected_ms").is_none());
    assert_eq!(moved.doc["installed_ms"], 1_700_000_000_000_i64);
}

#[test]
fn a_server_id_with_a_slash_is_escaped_into_its_v2_key() {
    let fixture = Fixture::new();
    fixture.legacy("mcp_servers", "a/b", v1_server());

    fixture.migrate();

    assert!(fixture.get(SERVERS, "a%2Fb").is_some());
}

#[test]
fn a_name_claim_is_keyed_by_the_qualified_name_and_keeps_its_time() {
    let fixture = Fixture::new();
    fixture.legacy(
        "mcp_server_names",
        "legacy-claim-files",
        json!({"qualified_name": "@acme/files", "server_id": "srv-1", "claimed_at": 5_000}),
    );
    fixture.legacy(
        "mcp_server_names",
        "legacy-claim-old",
        json!({"qualified_name": "@acme/old", "server_id": "srv-0"}),
    );

    fixture.migrate();

    let claim = fixture.get(SERVER_NAMES, "@acme%2Ffiles").unwrap();
    assert_eq!(
        claim.doc,
        json!({"qualified_name": "@acme/files", "server_id": "srv-1", "claimed_ms": 5_000})
    );
    // A claim that predates the time reads as long expired.
    let old = fixture.get(SERVER_NAMES, "@acme%2Fold").unwrap();
    assert_eq!(old.doc["claimed_ms"], 0);
    assert_eq!(fixture.all("mcp_server_names").len(), 0);
}

#[test]
fn a_cached_browse_response_becomes_an_object_body_with_a_cached_time() {
    let fixture = Fixture::new();
    fixture.legacy(
        "mcp_registry_cache",
        "digest-1",
        json!({"cache_key": "official:search:slack", "body_json": "{\"servers\":[1,2]}", "cached_at": 9_000}),
    );

    fixture.migrate();

    let moved = fixture
        .get(REGISTRY_CACHE, "official:search:slack")
        .unwrap();
    assert_eq!(
        moved.doc,
        json!({"cache_key": "official:search:slack", "body": {"servers": [1, 2]}, "cached_ms": 9_000})
    );
}

#[test]
fn a_cached_response_that_is_not_json_is_dropped_rather_than_blocking_the_step() {
    let fixture = Fixture::new();
    fixture.legacy(
        "mcp_registry_cache",
        "digest-1",
        json!({"cache_key": "k", "body_json": "not json", "cached_at": 1}),
    );
    fixture.legacy(
        "mcp_registry_cache",
        "digest-2",
        json!({"cache_key": "ok", "body_json": "[]", "cached_at": 2}),
    );

    fixture.migrate();

    assert!(fixture.get(REGISTRY_CACHE, "k").is_none());
    assert!(fixture.get(REGISTRY_CACHE, "ok").is_some());
    assert_eq!(fixture.all("mcp_registry_cache").len(), 0);
}

#[test]
fn cached_tools_are_keyed_by_the_server_key() {
    let fixture = Fixture::new();
    fixture.legacy(
        "mcp_tool_cache",
        "digest-1",
        json!({"server_key": "static:docs", "fingerprint": "abc", "tools": [{"name": "t"}], "cached_at": 7_000}),
    );

    fixture.migrate();

    let moved = fixture.get(TOOL_CACHE, "static:docs").unwrap();
    assert_eq!(
        moved.doc,
        json!({"server_key": "static:docs", "fingerprint": "abc", "tools": [{"name": "t"}], "cached_ms": 7_000})
    );
}

#[test]
fn an_index_row_gets_an_object_record_a_synced_time_and_a_composite_key() {
    let fixture = Fixture::new();
    fixture.legacy(
        "mcp_registry_index",
        "digest-1",
        json!({
            "source": "official",
            "qualified_name": "@acme/files",
            "label": "files",
            "description": "read files",
            "record_json": "{\"name\":\"files\"}",
            "synced_at": 4_000,
            "search": "fi fil file files",
        }),
    );

    fixture.migrate();

    let moved = fixture
        .get(REGISTRY_INDEX, "official/@acme%2Ffiles")
        .unwrap();
    assert_eq!(moved.doc["record"], json!({"name": "files"}));
    assert_eq!(moved.doc["synced_ms"], 4_000);
    assert!(moved.doc.get("record_json").is_none());
    assert!(moved.doc.get("synced_at").is_none());
    assert_eq!(moved.doc["search"], "fi fil file files");
    assert_eq!(
        fixture.count_where(REGISTRY_INDEX, Filter::eq("source", "official")),
        1
    );
}

#[test]
fn an_index_state_moves_its_times_to_ms_fields() {
    let fixture = Fixture::new();
    fixture.legacy(
        "mcp_registry_index_state",
        "digest-1",
        json!({"source": "official", "base_url": "https://r", "cursor": "c", "pages": 3, "started_at": 100, "synced_at": 50}),
    );
    fixture.legacy(
        "mcp_registry_index_state",
        "digest-2",
        json!({"source": "other", "base_url": "https://o", "pages": 0}),
    );

    fixture.migrate();

    let state = fixture.get(REGISTRY_INDEX_STATE, "official").unwrap();
    assert_eq!(
        state.doc,
        json!({"source": "official", "base_url": "https://r", "cursor": "c", "pages": 3, "started_ms": 100, "synced_ms": 50})
    );
    let idle = fixture.get(REGISTRY_INDEX_STATE, "other").unwrap();
    assert!(idle.doc.get("started_ms").is_none());
    assert!(idle.doc.get("synced_ms").is_none());
}

#[test]
fn the_audit_stream_moves_in_order_under_its_v2_name() {
    let fixture = Fixture::new();
    let entries = [
        json!({"timestamp_ms": 1, "client_info": "c", "tool_name": "a", "args_summary": {}, "success": true}),
        json!({"timestamp_ms": 2, "client_info": "c", "tool_name": "b", "args_summary": {}, "success": false, "error_message": "boom"}),
    ];
    fixture.legacy_stream("mcp_audit", &entries);

    fixture.migrate();

    assert_eq!(fixture.stream(AUDIT_STREAM), entries);
    assert_eq!(fixture.stream("mcp_audit").len(), 0);
}

#[test]
fn opening_the_audit_store_runs_owner_stream_migrations() {
    let fixture = Fixture::new();
    let entries = [json!({"timestamp_ms": 1}), json!({"timestamp_ms": 2})];
    fixture.legacy_stream("mcp_audit", &entries);
    let ports = crate::storage::StoragePorts::from_storage(
        &fixture.storage,
        Arc::clone(fixture.storage.secrets()),
    );

    let _audit = AuditStore::open(ports).unwrap();

    assert_eq!(fixture.stream(AUDIT_STREAM), entries);
    assert_eq!(fixture.stream("mcp_audit"), Vec::<Value>::new());
}

#[test]
fn audit_import_keeps_existing_target_rows_and_duplicate_source_events() {
    let fixture = Fixture::new();
    let repeated = json!({"timestamp_ms": 1, "tool_name": "same"});
    fixture.legacy_stream("mcp.audit", &[json!({"timestamp_ms": 0})]);
    fixture.legacy_stream("mcp_audit", &[repeated.clone(), repeated.clone()]);

    fixture.migrate();

    assert_eq!(
        fixture.stream(AUDIT_STREAM),
        [json!({"timestamp_ms": 0}), repeated.clone(), repeated]
    );
}

#[test]
fn concurrent_owner_openers_do_not_duplicate_an_audit_batch() {
    let fixture = Fixture::new();
    let repeated = json!({"timestamp_ms": 1, "tool_name": "same"});
    fixture.legacy_stream("mcp_audit", &[repeated.clone(), repeated.clone()]);

    // These are independently constructed owner handles over the same real
    // memory engine scope. The stream decorator deterministically lets the
    // competing opener win after this importer has observed its baseline.
    let first = StoragePorts::from_storage(&fixture.storage, Arc::clone(fixture.storage.secrets()));
    let competing =
        StoragePorts::from_storage(&fixture.storage, Arc::clone(fixture.storage.secrets()));
    let racing = CompetingAuditAppend {
        inner: Arc::clone(&competing.streams),
        raced: AtomicBool::new(false),
        competing_batch: None,
    };
    let migrations = Migrations::new(&first.documents, OWNER);

    fixture
        .blocking
        .block_on(migrate::migrate_audit_streams(&racing, &migrations))
        .unwrap();

    assert_eq!(fixture.stream(AUDIT_STREAM), [repeated.clone(), repeated]);
    assert_eq!(fixture.stream("mcp_audit"), Vec::<Value>::new());
}

#[test]
fn audit_import_keeps_its_source_when_a_different_batch_wins_the_offset() {
    let fixture = Fixture::new();
    let source = json!({"timestamp_ms": 1, "tool_name": "source"});
    let competing = json!({"timestamp_ms": 9, "tool_name": "other"});
    fixture.legacy_stream("mcp_audit", std::slice::from_ref(&source));
    let storage =
        StoragePorts::from_storage(&fixture.storage, Arc::clone(fixture.storage.secrets()));
    let racing = CompetingAuditAppend {
        inner: Arc::clone(&storage.streams),
        raced: AtomicBool::new(false),
        competing_batch: Some(vec![competing.clone()]),
    };
    let migrations = Migrations::new(&storage.documents, OWNER);

    let error = fixture
        .blocking
        .block_on(migrate::migrate_audit_streams(&racing, &migrations))
        .unwrap_err();

    assert_eq!(error.kind(), tinystoragedrivers_types::ErrorKind::Conflict);
    assert_eq!(fixture.stream("mcp_audit"), [source]);
    assert_eq!(fixture.stream(AUDIT_STREAM), [competing]);
    let progress = fixture
        .blocking
        .block_on(migrations.stream_progress(OWNER, "audit_streams_v2", "mcp_audit"))
        .unwrap()
        .unwrap();
    assert_eq!(progress.source_offset, 0);
    assert_eq!(progress.target_offset, 0);
    assert!(
        !fixture
            .blocking
            .block_on(migrations.done(OWNER, "audit_streams_v2"))
            .unwrap()
    );
}

#[test]
fn audit_import_only_moves_exact_legacy_stream_names() {
    let fixture = Fixture::new();
    let alias = json!({"timestamp_ms": 9});
    fixture.legacy_stream("mcp_audit", &[json!({"timestamp_ms": 1})]);
    fixture.legacy_stream("mcp_audit_extra", std::slice::from_ref(&alias));
    fixture.legacy_stream("mcp_audit/archive", &[json!({"timestamp_ms": 2})]);

    fixture.migrate();

    assert_eq!(fixture.stream(AUDIT_STREAM), [json!({"timestamp_ms": 1})]);
    assert_eq!(
        fixture.stream("mcp.audit/archive"),
        [json!({"timestamp_ms": 2})]
    );
    assert_eq!(fixture.stream("mcp_audit_extra"), [alias]);
}

#[test]
fn audit_import_retries_after_an_empty_page_before_source_end() {
    let fixture = Fixture::new();
    let entries = [json!({"timestamp_ms": 1})];
    fixture.legacy_stream("mcp_audit", &entries);
    let streams = Arc::new(EmptyFirstRead {
        inner: Arc::clone(fixture.storage.streams()),
        fail: AtomicBool::new(true),
    });
    let documents = Arc::clone(fixture.storage.documents());
    let migrations = Migrations::new(&documents, OWNER);

    let first = fixture
        .blocking
        .block_on(migrate::migrate_audit_streams(&*streams, &migrations));
    assert!(
        first.is_err(),
        "an empty page before the declared end is an error"
    );
    assert_eq!(fixture.stream("mcp_audit"), entries);
    assert_eq!(fixture.stream(AUDIT_STREAM), Vec::<Value>::new());

    fixture
        .blocking
        .block_on(migrate::migrate_audit_streams(&*streams, &migrations))
        .unwrap();
    assert_eq!(fixture.stream("mcp_audit"), Vec::<Value>::new());
    assert_eq!(fixture.stream(AUDIT_STREAM), entries);
}

#[test]
fn audit_import_rechecks_source_length_before_deleting_it() {
    let fixture = Fixture::new();
    fixture.legacy_stream("mcp_audit", &[json!({"timestamp_ms": 1})]);
    let streams = Arc::new(AppendDuringRead {
        inner: Arc::clone(fixture.storage.streams()),
        append: AtomicBool::new(true),
    });
    let documents = Arc::clone(fixture.storage.documents());
    let migrations = Migrations::new(&documents, OWNER);

    let first = fixture
        .blocking
        .block_on(migrate::migrate_audit_streams(&*streams, &migrations));
    assert!(
        first.is_err(),
        "a source changed during import must be preserved"
    );
    assert_eq!(fixture.stream("mcp_audit").len(), 2);
    assert_eq!(fixture.stream(AUDIT_STREAM), [json!({"timestamp_ms": 1})]);

    fixture
        .blocking
        .block_on(migrate::migrate_audit_streams(&*streams, &migrations))
        .unwrap();
    assert_eq!(fixture.stream("mcp_audit"), Vec::<Value>::new());
    assert_eq!(
        fixture.stream(AUDIT_STREAM),
        [json!({"timestamp_ms": 1}), json!({"timestamp_ms": 2})]
    );
}

#[test]
fn audit_import_recovers_when_append_succeeds_but_progress_write_is_lost() {
    let fixture = Fixture::new();
    let repeated = json!({"timestamp_ms": 1, "tool_name": "same"});
    fixture.legacy_stream("mcp_audit", &[repeated.clone(), repeated.clone()]);
    let streams = Arc::new(FailAfterFirstAppend {
        inner: Arc::clone(fixture.storage.streams()),
        fail: AtomicBool::new(true),
    });
    let documents = Arc::clone(fixture.storage.documents());
    let migrations = Migrations::new(&documents, OWNER);

    let first = fixture
        .blocking
        .block_on(migrate::migrate_audit_streams(&*streams, &migrations));
    assert!(
        first.is_err(),
        "the simulated lost acknowledgement must fail the first run"
    );
    fixture
        .blocking
        .block_on(migrate::migrate_audit_streams(&*streams, &migrations))
        .unwrap();

    assert_eq!(fixture.stream(AUDIT_STREAM), [repeated.clone(), repeated]);
    assert_eq!(fixture.stream("mcp_audit"), Vec::<Value>::new());
}

#[test]
fn migrating_again_changes_nothing() {
    let fixture = Fixture::new();
    fixture.legacy("mcp_servers", "srv-1", v1_server());
    fixture.legacy_stream("mcp_audit", &[json!({"timestamp_ms": 1})]);

    fixture.migrate();
    let before = fixture.get(SERVERS, "srv-1").unwrap();
    fixture.migrate();

    let after = fixture.get(SERVERS, "srv-1").unwrap();
    assert_eq!(before, after);
    assert_eq!(fixture.stream(AUDIT_STREAM).len(), 1);
}
