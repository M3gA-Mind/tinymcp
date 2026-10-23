//! A documents port that opens fine and then fails every read and write, for
//! tests of how callers behave when storage goes away.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::storage::types::{
    Capabilities, CollectionSpec, DocumentStore, Filter, Page, Precondition, Query, Result,
    SearchHit, Sort, StorageError, Version, Versioned,
};
use serde_json::Value;

use super::StoragePorts;

/// Declares collections successfully; every other call is a backend failure.
#[derive(Debug, Default)]
pub(crate) struct FailingDocuments {
    /// Fail with `NotFound` (a collection never declared) instead of an outage.
    pub(crate) missing: bool,
    /// Refuse to declare the storage layer's own `tsd.` collections, as a
    /// host's grant does.
    pub(crate) refuse_ledger: bool,
}

impl FailingDocuments {
    fn down<T>(&self) -> Result<T> {
        Err(if self.missing {
            StorageError::not_found("no such collection")
        } else {
            StorageError::unavailable("the storage backend is down")
        })
    }
}

#[tinystoragedrivers_api::async_trait]
impl DocumentStore for FailingDocuments {
    fn capabilities(&self) -> Capabilities {
        Capabilities::none()
    }

    async fn ensure_collection(&self, spec: &CollectionSpec) -> Result<()> {
        if self.refuse_ledger && spec.name.starts_with("tsd.") {
            return Err(StorageError::invalid_input("schema is host-only"));
        }
        Ok(())
    }

    async fn get(&self, _collection: &str, _id: &str) -> Result<Option<Versioned<Value>>> {
        if _collection == "tsd.migrations" {
            Ok(None)
        } else {
            self.down()
        }
    }

    async fn put(
        &self,
        collection: &str,
        _id: &str,
        _doc: Value,
        _precondition: Precondition,
    ) -> Result<Version> {
        if collection == "tsd.migrations" {
            Ok(Version::FIRST)
        } else {
            self.down()
        }
    }

    async fn delete(
        &self,
        _collection: &str,
        _id: &str,
        _precondition: Precondition,
    ) -> Result<bool> {
        self.down()
    }

    async fn query(&self, collection: &str, _query: &Query) -> Result<Page<Versioned<Value>>> {
        if collection.starts_with("mcp_") {
            Ok(Page {
                items: Vec::new(),
                next: None,
            })
        } else {
            self.down()
        }
    }

    async fn count(&self, _collection: &str, _filter: &Filter) -> Result<u64> {
        self.down()
    }

    async fn delete_where(&self, _collection: &str, _filter: &Filter) -> Result<u64> {
        self.down()
    }

    async fn claim(
        &self,
        _collection: &str,
        _filter: &Filter,
        _sort: &[Sort],
        _patch: &Value,
    ) -> Result<Option<Versioned<Value>>> {
        self.down()
    }

    async fn search(
        &self,
        _collection: &str,
        _text: &str,
        _limit: usize,
    ) -> Result<Vec<SearchHit>> {
        self.down()
    }

    async fn drop_collection(&self, _collection: &str) -> Result<()> {
        self.down()
    }
}

impl StoragePorts {
    /// Ports whose documents fail after opening, over otherwise working
    /// streams and secrets.
    pub(crate) fn with_failing_documents() -> Self {
        Self::with_documents(FailingDocuments::default())
    }

    /// Ports whose collections were never declared: counts answer `NotFound`.
    pub(crate) fn with_undeclared_collections() -> Self {
        Self::with_documents(FailingDocuments {
            missing: true,
            ..FailingDocuments::default()
        })
    }

    /// Ports as a host leaves them: its collections are declared, the shared
    /// migration ledger is refused, and nothing else is reachable.
    pub(crate) fn with_host_owned_schema() -> Self {
        Self::with_documents(FailingDocuments {
            refuse_ledger: true,
            ..FailingDocuments::default()
        })
    }

    fn with_documents(documents: FailingDocuments) -> Self {
        let working = Self::memory().expect("memory ports");
        Self::new(
            std::sync::Arc::new(documents),
            working.streams,
            working.secrets,
        )
    }
}
