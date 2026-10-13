//! The local copy of an upstream catalog, for searching without asking it.
//!
//! Two collections. `mcp_registry_index` holds one document per server: the
//! upstream's own record, plus a lowercased label and description to match a
//! query against, and the words (with their prefixes) the port's text search
//! indexes. `mcp_registry_index_state` holds one row per catalog: which base URL
//! the rows came from, where an unfinished sync stopped, and when the last sync
//! finished.
//!
//! # A sync is resumable
//!
//! Each page's rows are written, then the cursor for the next one. A sync that stops part way — a timeout, a restart — leaves the
//! rows it already wrote and a cursor to resume from. Rows from an earlier sync
//! stay until a sync finishes, at which point any server it did not see again
//! is removed.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinystoragedrivers_core::{Capability, DocumentStore, Filter, Precondition, Query, WriteOp};

use super::tool_cache::hex_digest;
use super::types::{Store, now_ms};
use crate::error::Result;
use crate::storage::{REGISTRY_INDEX, REGISTRY_INDEX_STATE};

/// How many index documents one batch writes.
const BATCH: usize = 100;
/// The longest prefix of a word the search text carries.
const MAX_PREFIX: usize = 12;
/// How many candidates one index search asks the port for.
const SEARCH_CANDIDATES: usize = 5_000;

/// One server, ready to be written to the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexRow {
    /// The registry's qualified name.
    pub(crate) qualified_name: String,
    /// The name and title, lowercased, for matching.
    pub(crate) label: String,
    /// The description, lowercased, for matching.
    pub(crate) description: String,
    /// The upstream's record for the server, as JSON.
    pub(crate) record_json: String,
}

/// One server an index search matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexHit {
    /// The registry's qualified name.
    pub(crate) qualified_name: String,
    /// Whether every word matched the name or title.
    pub(crate) in_label: bool,
    /// The upstream's record for the server, as JSON.
    pub(crate) record_json: String,
}

/// Where one catalog's index stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexState {
    /// The base URL the rows came from.
    pub(crate) base_url: String,
    /// The cursor an unfinished sync resumes from; `None` for its first page.
    pub(crate) cursor: Option<String>,
    /// How many pages the unfinished sync has written.
    pub(crate) pages: u32,
    /// When the unfinished sync started, in Unix epoch milliseconds; `None`
    /// when no sync is unfinished.
    pub(crate) started_at: Option<i64>,
    /// When the last sync finished, in Unix epoch milliseconds; `None` when
    /// none has.
    pub(crate) synced_at: Option<i64>,
}

impl IndexState {
    /// Whether a sync started and has not finished.
    pub(crate) const fn in_progress(&self) -> bool {
        self.started_at.is_some()
    }
}

/// One index document: the row plus the text the port searches.
#[derive(Debug, Serialize, Deserialize)]
struct IndexDoc {
    source: String,
    qualified_name: String,
    label: String,
    description: String,
    record_json: String,
    synced_at: i64,
    search: String,
}

/// One catalog's sync state as stored.
#[derive(Debug, Serialize, Deserialize)]
struct StateDoc {
    source: String,
    base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
    #[serde(default)]
    pages: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    synced_at: Option<i64>,
}

fn index_id(source: &str, qualified_name: &str) -> String {
    hex_digest(format!("{source}\0{qualified_name}").as_bytes())
}

/// The text a document is searched by: the words of the label and
/// description, each with its prefixes, so a partly typed word still matches
/// whatever tokenizer the backend uses.
fn search_text(label: &str, description: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for word in label
        .split(|c: char| !c.is_alphanumeric())
        .chain(description.split(|c: char| !c.is_alphanumeric()))
        .filter(|word| !word.is_empty())
    {
        let word = word.to_lowercase();
        let characters: Vec<char> = word.chars().collect();
        for end in 2..=characters.len().min(MAX_PREFIX) {
            words.push(characters[..end].iter().collect());
        }
        if characters.len() > MAX_PREFIX {
            words.push(word);
        }
    }
    words.sort();
    words.dedup();
    words.join(" ")
}

impl Store {
    /// Where `source`'s index stands, or `None` when no sync has begun.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Store`] when the query fails.
    pub(crate) fn index_state(&self, source: &str) -> Result<Option<IndexState>> {
        let id = hex_digest(source.as_bytes());
        let found = self
            .ports
            .run("reading the index state", move |ports| async move {
                ports.documents.get(REGISTRY_INDEX_STATE, &id).await
            })?;
        found
            .map(|row| {
                let state: StateDoc = serde_json::from_value(row.doc)?;
                Ok(IndexState {
                    base_url: state.base_url,
                    cursor: state.cursor,
                    pages: state.pages,
                    started_at: state.started_at,
                    synced_at: state.synced_at,
                })
            })
            .transpose()
    }

    /// Starts a sync of `source` from its first page.
    ///
    /// Rows from a different base URL are dropped, since they describe a
    /// different catalog; rows from the same one stay searchable until the
    /// sync finishes.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Store`] when a write fails.
    pub(crate) fn begin_index_sync(&self, source: &str, base_url: &str) -> Result<()> {
        let source = source.to_string();
        let base_url = base_url.to_string();
        let now = now_ms();
        self.ports
            .run("beginning an index sync", move |ports| async move {
                let documents = ports.documents;
                let id = hex_digest(source.as_bytes());
                let previous = documents.get(REGISTRY_INDEX_STATE, &id).await?;
                let previous: Option<StateDoc> = previous
                    .map(|row| serde_json::from_value(row.doc))
                    .transpose()
                    .map_err(|error| {
                        tinystoragedrivers_core::StorageError::serialization(error.to_string())
                    })?;
                let same_catalog = previous
                    .as_ref()
                    .is_some_and(|state| state.base_url == base_url);
                if !same_catalog {
                    documents
                        .delete_where(REGISTRY_INDEX, &Filter::eq("source", source.clone()))
                        .await?;
                }
                let state = StateDoc {
                    source,
                    base_url,
                    cursor: None,
                    pages: 0,
                    started_at: Some(now),
                    synced_at: previous
                        .and_then(|state| state.synced_at)
                        .filter(|_| same_catalog),
                };
                let value = serde_json::to_value(state).map_err(|error| {
                    tinystoragedrivers_core::StorageError::serialization(error.to_string())
                })?;
                documents
                    .put(REGISTRY_INDEX_STATE, &id, value, Precondition::None)
                    .await
                    .map(|_| ())
            })
    }

    /// Writes one synced page of `source` and the cursor for the next.
    ///
    /// The rows go in batches, then the cursor: a failure part way leaves the
    /// cursor where it was, so the sync resumes at this page and rewrites
    /// the same rows.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Store`] when a write fails.
    pub(crate) fn store_index_page(
        &self,
        source: &str,
        rows: &[IndexRow],
        next_cursor: Option<&str>,
    ) -> Result<()> {
        let now = now_ms();
        let mut ops = Vec::with_capacity(rows.len());
        for row in rows {
            let doc = IndexDoc {
                source: source.to_string(),
                qualified_name: row.qualified_name.clone(),
                label: row.label.clone(),
                description: row.description.clone(),
                record_json: row.record_json.clone(),
                synced_at: now,
                search: search_text(&row.label, &row.description),
            };
            ops.push(WriteOp::Put {
                collection: REGISTRY_INDEX.to_string(),
                id: index_id(source, &row.qualified_name),
                doc: serde_json::to_value(doc)?,
                precondition: Precondition::None,
            });
        }
        let source = source.to_string();
        let next_cursor = next_cursor.map(ToString::to_string);
        self.ports
            .run("writing an index page", move |ports| async move {
                let documents = ports.documents;
                let atomic = documents.capabilities().contains(Capability::Transactions);
                while !ops.is_empty() {
                    let rest = ops.split_off(ops.len().min(BATCH));
                    let chunk = std::mem::replace(&mut ops, rest);
                    write_chunk(documents.as_ref(), chunk, atomic).await?;
                }
                let id = hex_digest(source.as_bytes());
                let Some(found) = documents.get(REGISTRY_INDEX_STATE, &id).await? else {
                    return Ok(());
                };
                let mut state: StateDoc =
                    serde_json::from_value(found.doc.clone()).map_err(|error| {
                        tinystoragedrivers_core::StorageError::serialization(error.to_string())
                    })?;
                state.cursor = next_cursor;
                state.pages += 1;
                let value = serde_json::to_value(state).map_err(|error| {
                    tinystoragedrivers_core::StorageError::serialization(error.to_string())
                })?;
                documents
                    .put(REGISTRY_INDEX_STATE, &id, value, found.unchanged())
                    .await
                    .map(|_| ())
            })
    }

    /// Finishes the sync of `source`, dropping every server it did not see.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Store`] when a write fails.
    pub(crate) fn finish_index_sync(&self, source: &str) -> Result<()> {
        let source = source.to_string();
        let now = now_ms();
        self.ports
            .run("finishing an index sync", move |ports| async move {
                let documents = ports.documents;
                let id = hex_digest(source.as_bytes());
                let Some(found) = documents.get(REGISTRY_INDEX_STATE, &id).await? else {
                    return Ok(());
                };
                let mut state: StateDoc =
                    serde_json::from_value(found.doc.clone()).map_err(|error| {
                        tinystoragedrivers_core::StorageError::serialization(error.to_string())
                    })?;
                if let Some(started_at) = state.started_at {
                    documents
                        .delete_where(
                            REGISTRY_INDEX,
                            &Filter::eq("source", source.clone())
                                .and(Filter::lt("synced_at", started_at)),
                        )
                        .await?;
                }
                state.cursor = None;
                state.pages = 0;
                state.started_at = None;
                state.synced_at = Some(now);
                let value = serde_json::to_value(state).map_err(|error| {
                    tinystoragedrivers_core::StorageError::serialization(error.to_string())
                })?;
                documents
                    .put(REGISTRY_INDEX_STATE, &id, value, found.unchanged())
                    .await
                    .map(|_| ())
            })
    }

    /// Every indexed server of `source` matching each of `terms` in its label
    /// or description.
    ///
    /// The port's text search finds the candidates (whole words and the
    /// prefixes of words, ranked); each is then checked against every term as
    /// a substring of its stored label or description, which is the matching
    /// rule callers rely on. The terms are matched as given, so a caller
    /// lowercases them to match the stored text.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Store`] when the query fails.
    pub(crate) fn search_index(&self, source: &str, terms: &[String]) -> Result<Vec<IndexHit>> {
        let wanted_source = source.to_string();
        let source = source.to_string();
        let owned_terms = terms.to_vec();
        let rows = self
            .ports
            .run("searching the index", move |ports| async move {
                let documents = ports.documents;
                let searchable: Vec<&str> = owned_terms
                    .iter()
                    .map(String::as_str)
                    .filter(|term| term.chars().count() >= 2)
                    .collect();
                let mut found: Vec<Value> = Vec::new();
                if searchable.is_empty() {
                    // Nothing the text index can hold (no terms, or one letter
                    // each): read the source's rows.
                    let page = tinystoragedrivers_core::DocumentStoreExt::query_all(
                        documents.as_ref(),
                        REGISTRY_INDEX,
                        &Query::filter(Filter::eq("source", source.clone())),
                    )
                    .await?;
                    found.extend(page.into_iter().map(|row| row.doc));
                } else {
                    let hits = documents
                        .search(REGISTRY_INDEX, &searchable.join(" "), SEARCH_CANDIDATES)
                        .await?;
                    for hit in hits {
                        if let Some(row) = documents.get(REGISTRY_INDEX, &hit.id).await? {
                            found.push(row.doc);
                        }
                    }
                }
                Ok(found)
            })?;

        let mut hits = Vec::new();
        for doc in rows {
            let row: IndexDoc = serde_json::from_value(doc)?;
            if row.source != wanted_source {
                continue;
            }
            let matches = terms.iter().all(|term| {
                row.label.contains(term.as_str()) || row.description.contains(term.as_str())
            });
            if !matches {
                continue;
            }
            hits.push(IndexHit {
                in_label: terms.iter().all(|term| row.label.contains(term.as_str())),
                qualified_name: row.qualified_name,
                record_json: row.record_json,
            });
        }
        Ok(hits)
    }
}

/// Writes one chunk of index documents, atomically when the backend can.
async fn write_chunk(
    documents: &dyn DocumentStore,
    chunk: Vec<WriteOp>,
    atomic: bool,
) -> tinystoragedrivers_core::Result<()> {
    if atomic {
        return documents.atomic_batch(chunk).await.map(|_| ());
    }
    for op in chunk {
        if let WriteOp::Put {
            collection,
            id,
            doc,
            precondition,
        } = op
        {
            documents.put(&collection, &id, doc, precondition).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
