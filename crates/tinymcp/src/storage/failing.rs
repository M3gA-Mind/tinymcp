//! A documents port that opens fine and then fails every read and write, for
//! tests of how callers behave when storage goes away.

use serde_json::Value;
use tinystoragedrivers_core::{
    Capabilities, CollectionSpec, DocumentStore, Filter, Page, Precondition, Query, Result,
    SearchHit, Sort, StorageError, Version, Versioned, async_trait,
};

use super::StoragePorts;

/// Declares collections successfully; every other call is a backend failure.
#[derive(Debug)]
pub(crate) struct FailingDocuments;

fn down<T>() -> Result<T> {
    Err(StorageError::unavailable("the storage backend is down"))
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
        down()
    }

    async fn put(
        &self,
        _collection: &str,
        _id: &str,
        _doc: Value,
        _precondition: Precondition,
    ) -> Result<Version> {
        down()
    }

    async fn delete(
        &self,
        _collection: &str,
        _id: &str,
        _precondition: Precondition,
    ) -> Result<bool> {
        down()
    }

    async fn query(&self, _collection: &str, _query: &Query) -> Result<Page<Versioned<Value>>> {
        down()
    }

    async fn count(&self, _collection: &str, _filter: &Filter) -> Result<u64> {
        down()
    }

    async fn delete_where(&self, _collection: &str, _filter: &Filter) -> Result<u64> {
        down()
    }

    async fn claim(
        &self,
        _collection: &str,
        _filter: &Filter,
        _sort: &[Sort],
        _patch: &Value,
    ) -> Result<Option<Versioned<Value>>> {
        down()
    }

    async fn search(
        &self,
        _collection: &str,
        _text: &str,
        _limit: usize,
    ) -> Result<Vec<SearchHit>> {
        down()
    }

    async fn drop_collection(&self, _collection: &str) -> Result<()> {
        down()
    }
}

impl StoragePorts {
    /// Ports whose documents fail after opening, over otherwise working
    /// streams and secrets.
    pub(crate) fn with_failing_documents() -> Self {
        let working = Self::memory().expect("memory ports");
        Self::new(
            std::sync::Arc::new(FailingDocuments),
            working.streams,
            working.secrets,
        )
    }
}
