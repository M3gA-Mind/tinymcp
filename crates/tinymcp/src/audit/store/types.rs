//! The audit store and its two operations.

use crate::McpWriteListQueryExt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinystoragedrivers_core::{StorageError, StreamEntry};

use crate::error::{Error, Result};
use crate::storage::{AUDIT_STREAM, StoragePorts};
use tinymcp_bus::{ERROR_MESSAGE_MAX_BYTES, McpWriteListQuery, McpWriteRecord, NewMcpWriteRecord};

/// How many stream entries one read asks for.
const WINDOW: usize = 256;

/// One audit record as stored in the stream.
#[derive(Debug, Serialize, Deserialize)]
struct AuditEntry {
    timestamp_ms: i64,
    client_info: String,
    tool_name: String,
    #[serde(default)]
    args_summary: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resulting_chunk_id: Option<String>,
    success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
}

/// The durable record of MCP tool writes.
///
/// An append-only stream on the storage ports; a record's identifier is its
/// offset plus one, so identifiers start at 1 and only grow.
#[derive(Debug)]
pub struct AuditStore {
    ports: StoragePorts,
}

impl AuditStore {
    /// Opens the audit log over `ports`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the stream cannot be reached.
    pub fn open(ports: StoragePorts) -> Result<Self> {
        ports.run("opening the audit log", |ports| async move {
            ports.streams.len(AUDIT_STREAM).await.map(|_| ())
        })?;
        Ok(Self { ports })
    }

    /// Opens an audit log over a private in-memory backend, for tests.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the backend cannot be opened.
    pub fn open_in_memory() -> Result<Self> {
        Self::open(StoragePorts::memory()?)
    }

    /// Records one write, returning its assigned identifier.
    ///
    /// The error message is truncated to [`ERROR_MESSAGE_MAX_BYTES`] on a
    /// character boundary. It comes from a remote server and is bounded by
    /// nothing else, and an audit row that can be made arbitrarily large by a
    /// misbehaving server is a way to fill a user's disk.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Serialization`] when the argument summary cannot be
    /// encoded, and [`Error::Store`] when the append fails.
    pub fn record(&self, record: &NewMcpWriteRecord) -> Result<i64> {
        let entry = serde_json::to_value(AuditEntry {
            timestamp_ms: record.timestamp_ms,
            client_info: record.client_info.clone(),
            tool_name: record.tool_name.clone(),
            args_summary: record.args_summary.clone(),
            resulting_chunk_id: record.resulting_chunk_id.clone(),
            success: record.success,
            error_message: truncate_error(record.error_message.as_deref()),
        })?;
        let offset = self
            .ports
            .run("recording a write", move |ports| async move {
                ports.streams.append(AUDIT_STREAM, entry).await
            })?;
        offset_to_id(offset).ok_or_else(|| Error::malformed("the audit offset does not fit an id"))
    }

    /// Lists recorded writes, most recent first.
    ///
    /// The query's bounds are applied through its `resolved_*` accessors, so a
    /// caller cannot ask for an unbounded page. The stream is read in windows and
    /// filtered as it goes, so only matching records are held.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the read fails, and
    /// [`Error::MalformedResponse`] when a bound would not fit.
    pub fn list(&self, query: &McpWriteListQuery) -> Result<Vec<McpWriteRecord>> {
        let wanted = usize::try_from(query.resolved_limit())
            .map_err(|_| Error::malformed("limit is too large"))?;
        let skip = usize::try_from(query.resolved_offset())
            .map_err(|_| Error::malformed("offset is too large"))?;
        let since_ms = query.since_ms;
        let client = query.resolved_client_filter().map(ToString::to_string);
        let tool = query.resolved_tool_filter().map(ToString::to_string);
        let success_only = query.resolved_success_only();

        let entries = self
            .ports
            .run("listing audit records", move |ports| async move {
                let end = ports.streams.len(AUDIT_STREAM).await?;
                let mut matched: Vec<(u64, AuditEntry)> = Vec::new();
                let mut from = 0u64;
                while from < end {
                    let batch = ports.streams.read_window(AUDIT_STREAM, from, WINDOW).await?;
                    let Some(last) = batch.last().map(|entry| entry.offset) else {
                        break;
                    };
                    for StreamEntry { offset, value } in batch {
                        let entry: AuditEntry = serde_json::from_value(value)
                            .map_err(|error| StorageError::serialization(error.to_string()))?;
                        let keep = since_ms.is_none_or(|since| entry.timestamp_ms >= since)
                            && client.as_deref().is_none_or(|c| entry.client_info == c)
                            && tool.as_deref().is_none_or(|t| entry.tool_name == t)
                            && (!success_only || entry.success);
                        if keep {
                            matched.push((offset, entry));
                        }
                    }
                    from = last + 1;
                }
                Ok(matched)
            })?;

        let mut records: Vec<McpWriteRecord> = entries
            .into_iter()
            .filter_map(|(offset, entry)| {
                Some(McpWriteRecord {
                    id: offset_to_id(offset)?,
                    timestamp_ms: entry.timestamp_ms,
                    client_info: entry.client_info,
                    tool_name: entry.tool_name,
                    args_summary: entry.args_summary,
                    resulting_chunk_id: entry.resulting_chunk_id,
                    success: entry.success,
                    error_message: entry.error_message,
                })
            })
            .collect();
        // Newest first: by time, then by identifier, so two writes in the same
        // millisecond do not swap places between two reads of the same page.
        records.sort_by(|a, b| {
            b.timestamp_ms
                .cmp(&a.timestamp_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(records.into_iter().skip(skip).take(wanted).collect())
    }
}

/// An offset as a record identifier.
fn offset_to_id(offset: u64) -> Option<i64> {
    i64::try_from(offset).ok()?.checked_add(1)
}

/// Truncates an error message on a character boundary.
fn truncate_error(message: Option<&str>) -> Option<String> {
    let message = message?;
    if message.len() <= ERROR_MESSAGE_MAX_BYTES {
        return Some(message.to_string());
    }

    let mut end = ERROR_MESSAGE_MAX_BYTES;
    while end > 0 && !message.is_char_boundary(end) {
        end -= 1;
    }
    Some(message.get(..end).unwrap_or_default().to_string())
}
