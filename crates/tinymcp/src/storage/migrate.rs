//! The v1 to v2 steps: the previous release's `mcp_*` collections and stream
//! become the `mcp.*` ones.
//!
//! Each step is a pure function of the records it is handed, so a resumed or
//! repeated run produces the same output. A step keeps a record's creation
//! time and expiry, renames its `*_at` time fields to `*_ms`, and re-keys it
//! with [`id`]. A cache row or index row that cannot be carried over (its body
//! is not JSON, or the field its new key is built from is missing) is dropped
//! instead of failing the step: it holds nothing the module cannot fetch
//! again, and one bad row must not keep a whole database from migrating.

use serde_json::{Map, Value};
use tinystoragedrivers_api::StreamStore;
use tinystoragedrivers_types::{ErrorKind, Result as StorageResult, StreamEntry};

use super::local::{Migrations, SourceRecord, Step, StreamProgress, TargetRecord};
use super::schema::{
    OWNER, REGISTRY_CACHE, REGISTRY_INDEX, REGISTRY_INDEX_STATE, SERVER_NAMES, SERVERS, TOOL_CACHE,
    doc_id,
};
use tinymcp_bus::storage::{
    LegacyServerDocument, convert_legacy_server, convert_legacy_server_name_claim,
    legacy_audit_stream_target,
};

/// The version the steps read.
const FROM: u32 = 1;
/// The version the steps write.
const TO: u32 = 2;

/// The v1 stream the audit log was appended to.
const LEGACY_AUDIT: &str = "mcp_audit";

/// A document step's per-record rule: the new id and body, or `None` to drop
/// the record.
type Rewrite = fn(&SourceRecord, Map<String, Value>) -> StorageResult<Option<(String, Value)>>;

/// Every step, in the order they run.
pub(super) fn steps() -> Vec<Step> {
    vec![
        documents("servers_v2", SERVERS, "mcp_servers", server),
        documents("server_names_v2", SERVER_NAMES, "mcp_server_names", claim),
        documents(
            "registry_cache_v2",
            REGISTRY_CACHE,
            "mcp_registry_cache",
            cached_response,
        ),
        documents("tool_cache_v2", TOOL_CACHE, "mcp_tool_cache", cached_tools),
        documents(
            "registry_index_v2",
            REGISTRY_INDEX,
            "mcp_registry_index",
            index_row,
        ),
        documents(
            "registry_index_state_v2",
            REGISTRY_INDEX_STATE,
            "mcp_registry_index_state",
            index_state,
        ),
    ]
}

/// Move the old audit stream family to `mcp.audit`, resuming each source from a
/// durable positional checkpoint so pre-existing target entries are retained.
/// The host must quiesce writers to both stream families for the duration of
/// this one-time migration; stream ports do not provide an atomic append plus
/// checkpoint operation.
pub(crate) async fn migrate_audit_streams(
    streams: &dyn StreamStore,
    migrations: &Migrations,
) -> StorageResult<()> {
    const STEP: &str = "audit_streams_v2";
    if migrations.done(OWNER, STEP).await? {
        return Ok(());
    }
    for source in streams.streams(LEGACY_AUDIT).await? {
        let Some(target) = audit_target(&source) else {
            continue;
        };
        migrate_audit_source(streams, migrations, &source, &target).await?;
    }
    migrations.mark_done(OWNER, STEP).await
}

fn audit_target(source: &str) -> Option<String> {
    legacy_audit_stream_target(source)
}

async fn migrate_audit_source(
    streams: &dyn StreamStore,
    migrations: &Migrations,
    source: &str,
    target: &str,
) -> StorageResult<()> {
    const STEP: &str = "audit_streams_v2";
    let source_len = stream_len(streams, source).await?;
    let mut progress =
        if let Some(progress) = migrations.stream_progress(OWNER, STEP, source).await? {
            progress
        } else {
            let progress = StreamProgress {
                owner: OWNER.to_owned(),
                step: STEP.to_owned(),
                source: source.to_owned(),
                target: target.to_owned(),
                source_offset: 0,
                target_offset: streams.len(target).await?,
            };
            migrations.save_stream_progress(&progress).await?;
            progress
        };
    if progress.target != target {
        return Err(tinystoragedrivers_types::StorageError::conflict(
            "audit migration target changed after progress was recorded",
        ));
    }
    while progress.source_offset < source_len {
        let Some(batch) = source_batch(
            streams,
            migrations,
            source,
            progress.source_offset,
            source_len,
        )
        .await?
        else {
            progress.source_offset = source_len;
            break;
        };
        if batch.is_empty() {
            return Err(tinystoragedrivers_types::StorageError::conflict(
                "audit source ended before its reported length",
            ));
        }
        let batch_size = batch.len();
        let count = u64::try_from(batch_size).unwrap_or(u64::MAX);
        let values = batch
            .into_iter()
            .map(|entry| entry.value)
            .collect::<Vec<_>>();
        let copied = streams
            .read_window(target, progress.target_offset, batch_size)
            .await?
            .into_iter()
            .map(|entry| entry.value)
            .collect::<Vec<_>>();
        if copied == values {
            progress.source_offset = progress.source_offset.saturating_add(count);
            progress.target_offset = progress.target_offset.saturating_add(count);
            migrations.save_stream_progress(&progress).await?;
            continue;
        }
        if streams.len(target).await? != progress.target_offset {
            return Err(tinystoragedrivers_types::StorageError::conflict(
                "audit target changed while migrating its source stream",
            ));
        }
        let first_offset = match streams
            .append_batch_if_offset(target, progress.target_offset, values.clone())
            .await
        {
            Ok(first_offset) => first_offset,
            Err(error) if error.kind() == ErrorKind::Conflict => {
                // Another opener may have copied this exact source batch
                // after our checkpoint read. Accept only the identical
                // batch at the expected offset; any different write stays
                // a conflict and leaves the source and checkpoint intact.
                let raced = streams
                    .read_window(target, progress.target_offset, batch_size)
                    .await?
                    .into_iter()
                    .map(|entry| entry.value)
                    .collect::<Vec<_>>();
                if raced != values {
                    return Err(error);
                }
                progress.source_offset = progress.source_offset.saturating_add(count);
                progress.target_offset = progress.target_offset.saturating_add(count);
                migrations.save_stream_progress(&progress).await?;
                continue;
            }
            Err(error) => return Err(error),
        };
        progress.source_offset = progress.source_offset.saturating_add(count);
        progress.target_offset = first_offset.saturating_add(count);
        migrations.save_stream_progress(&progress).await?;
    }
    if progress.source_offset == source_len {
        delete_migrated_source(streams, source, source_len).await?;
    }
    Ok(())
}

async fn delete_migrated_source(
    streams: &dyn StreamStore,
    source: &str,
    source_len: u64,
) -> StorageResult<()> {
    let current_len = stream_len(streams, source).await?;
    if current_len != source_len && !(current_len == 0 && source_len > 0) {
        return Err(tinystoragedrivers_types::StorageError::conflict(
            "audit source changed while it was being migrated",
        ));
    }
    match streams.delete_stream(source).await {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound && current_len == 0 => Ok(()),
        Err(error) => Err(error),
    }
}

async fn stream_len(streams: &dyn StreamStore, source: &str) -> StorageResult<u64> {
    match streams.len(source).await {
        Ok(length) => Ok(length),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

async fn source_batch(
    streams: &dyn StreamStore,
    migrations: &Migrations,
    source: &str,
    source_offset: u64,
    source_len: u64,
) -> StorageResult<Option<Vec<StreamEntry>>> {
    match streams.read_window(source, source_offset, 256).await {
        Ok(batch) => Ok(Some(batch)),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let current_len = stream_len(streams, source).await?;
            let checkpoint = migrations
                .stream_progress(OWNER, "audit_streams_v2", source)
                .await?;
            if current_len == 0 && checkpoint.is_some_and(|saved| saved.source_offset >= source_len)
            {
                Ok(None)
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

/// A step reading the legacy collection `legacy` and writing `item`, applying
/// `rewrite` to each record's body.
fn documents(id: &'static str, item: &'static str, legacy: &'static str, rewrite: Rewrite) -> Step {
    Step::new(id, item, FROM, TO, move |batch| {
        let mut targets = Vec::with_capacity(batch.len());
        for record in batch {
            let Value::Object(body) = record.body.clone() else {
                continue;
            };
            if let Some((new_id, body)) = rewrite(&record, body)? {
                let mut target =
                    TargetRecord::new(item, new_id, body).expires_ms(record.meta.expires_ms);
                if record.meta.created_ms > 0 {
                    target = target.created_ms(record.meta.created_ms);
                }
                targets.push(target);
            }
        }
        Ok(targets)
    })
    .source(legacy)
}

/// Moves `from` to `to` when the record has it.
fn rename(body: &mut Map<String, Value>, from: &str, to: &str) {
    if let Some(value) = body.remove(from) {
        body.insert(to.to_owned(), value);
    }
}

/// The string field `name`, if the record has it.
fn text<'a>(body: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    body.get(name).and_then(Value::as_str)
}

/// Replaces the JSON text in `from` with the value it holds, under `to`.
/// `None` when the field is missing or is not JSON.
fn unwrap_json(body: &mut Map<String, Value>, from: &str, to: &str) -> Option<()> {
    let parsed = serde_json::from_str::<Value>(text(body, from)?).ok()?;
    body.remove(from);
    body.insert(to.to_owned(), parsed);
    Some(())
}

fn server(
    record: &SourceRecord,
    body: Map<String, Value>,
) -> StorageResult<Option<(String, Value)>> {
    let body = serde_json::from_value(Value::Object(body)).map_err(|error| {
        tinystoragedrivers_types::StorageError::invalid_input(format!(
            "malformed legacy MCP server record: {error}"
        ))
    })?;
    let converted = convert_legacy_server(LegacyServerDocument {
        storage_id: record.id.clone(),
        body,
    })?;
    let body = serde_json::to_value(converted.body).map_err(|error| {
        tinystoragedrivers_types::StorageError::invalid_input(format!(
            "serialize converted MCP server record: {error}"
        ))
    })?;
    Ok(Some((converted.id, body)))
}

/// A name claim: keyed by the qualified name it holds. A claim from before
/// claims were timed reads as claimed at `0`, i.e. long expired.
fn claim(
    _record: &SourceRecord,
    body: Map<String, Value>,
) -> StorageResult<Option<(String, Value)>> {
    let claim = serde_json::from_value(Value::Object(body)).map_err(|error| {
        tinystoragedrivers_types::StorageError::invalid_input(format!(
            "malformed legacy MCP server-name claim: {error}"
        ))
    })?;
    let converted = convert_legacy_server_name_claim(claim)?;
    let body = serde_json::to_value(converted.body).map_err(|error| {
        tinystoragedrivers_types::StorageError::invalid_input(format!(
            "serialize converted MCP server-name claim: {error}"
        ))
    })?;
    Ok(Some((converted.id, body)))
}

fn cached_response(
    _record: &SourceRecord,
    mut body: Map<String, Value>,
) -> StorageResult<Option<(String, Value)>> {
    let Some(key) = text(&body, "cache_key").map(ToOwned::to_owned) else {
        return Ok(None);
    };
    if unwrap_json(&mut body, "body_json", "body").is_none() {
        return Ok(None);
    }
    rename(&mut body, "cached_at", "cached_ms");
    Ok(Some((doc_id(&[&key])?, Value::Object(body))))
}

fn cached_tools(
    _record: &SourceRecord,
    mut body: Map<String, Value>,
) -> StorageResult<Option<(String, Value)>> {
    let Some(key) = text(&body, "server_key").map(ToOwned::to_owned) else {
        return Ok(None);
    };
    rename(&mut body, "cached_at", "cached_ms");
    Ok(Some((doc_id(&[&key])?, Value::Object(body))))
}

fn index_row(
    _record: &SourceRecord,
    mut body: Map<String, Value>,
) -> StorageResult<Option<(String, Value)>> {
    let (Some(source), Some(name)) = (
        text(&body, "source").map(ToOwned::to_owned),
        text(&body, "qualified_name").map(ToOwned::to_owned),
    ) else {
        return Ok(None);
    };
    if unwrap_json(&mut body, "record_json", "record").is_none() {
        return Ok(None);
    }
    rename(&mut body, "synced_at", "synced_ms");
    Ok(Some((doc_id(&[&source, &name])?, Value::Object(body))))
}

fn index_state(
    _record: &SourceRecord,
    mut body: Map<String, Value>,
) -> StorageResult<Option<(String, Value)>> {
    let Some(source) = text(&body, "source").map(ToOwned::to_owned) else {
        return Ok(None);
    };
    rename(&mut body, "started_at", "started_ms");
    rename(&mut body, "synced_at", "synced_ms");
    Ok(Some((doc_id(&[&source])?, Value::Object(body))))
}

#[cfg(test)]
#[path = "migrate_tests.rs"]
mod tests;
