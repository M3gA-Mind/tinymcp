//! A documents port that opens fine and then fails every read and write, for
//! tests of how callers behave when storage goes away.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;
use tinystoragedrivers_core::{
    Capabilities, CollectionSpec, DocumentStore, Filter, Page, Precondition, Query, Result,
    SearchHit, Sort, StorageError, Version, Versioned, async_trait,
};

use super::StoragePorts;

/// Declares collections successfully; every other call is a backend failure.
#[derive(Debug, Default)]
pub(crate) struct FailingDocuments {
    /// Fail with `NotFound` (a collection never declared) instead of an outage.
    pub(crate) missing: bool,
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

#[async_trait]
impl DocumentStore for FailingDocuments {
    fn capabilities(&self) -> Capabilities {
        Capabilities::none()
    }

    async fn ensure_collection(&self, _spec: &CollectionSpec) -> Result<()> {
        Ok(())
    }

    async fn get(&self, _collection: &str, _id: &str) -> Result<Option<Versioned<Value>>> {
        self.down()
    }

    async fn put(
        &self,
        _collection: &str,
        _id: &str,
        _doc: Value,
        _precondition: Precondition,
    ) -> Result<Version> {
        self.down()
    }

    async fn delete(
        &self,
        _collection: &str,
        _id: &str,
        _precondition: Precondition,
    ) -> Result<bool> {
        self.down()
    }

    async fn query(&self, _collection: &str, _query: &Query) -> Result<Page<Versioned<Value>>> {
        self.down()
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
        Self::with_documents(FailingDocuments { missing: true })
    }

    fn with_documents(documents: FailingDocuments) -> Self {
        let working = Self::memory().expect("memory ports");
        Self::new(
            std::sync::Arc::new(FailingDocuments::default()),
            working.streams,
            working.secrets,
        )
    }
}
