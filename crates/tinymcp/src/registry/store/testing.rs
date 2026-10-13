//! Test-only helpers that reach past the store's API into its documents.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;
use tinystoragedrivers_core::{DocumentStoreExt as _, Precondition, Query};

use super::types::Store;

impl Store {
    /// Subtracts `age_ms` from the numeric `fields` of every document in
    /// `collection`, so a test can make cached entries look old without
    /// waiting.
    pub(crate) fn age_collection(
        &self,
        collection: &'static str,
        fields: &'static [&'static str],
        age_ms: i64,
    ) {
        self.ports
            .run("ageing a collection", move |ports| async move {
                for row in ports.documents.query_all(collection, &Query::all()).await? {
                    let mut doc = row.doc.clone();
                    for field in fields {
                        if let Some(Value::Number(number)) = doc.get(*field).cloned() {
                            if let Some(value) = number.as_i64() {
                                doc[*field] = Value::from(value - age_ms);
                            }
                        }
                    }
                    ports
                        .documents
                        .put(collection, &row.id, doc, Precondition::Version(row.version))
                        .await?;
                }
                Ok(())
            })
            .expect("the collection ages");
    }

    /// Writes a raw document, for tests that need a record the store would
    /// never produce itself.
    pub(crate) fn put_raw(&self, collection: &'static str, id: &str, doc: Value) {
        let id = id.to_string();
        self.ports
            .run("seeding a document", move |ports| async move {
                ports
                    .documents
                    .put(collection, &id, doc, Precondition::None)
                    .await
                    .map(|_| ())
            })
            .expect("the document writes");
    }

    /// Reads a raw document.
    pub(crate) fn get_raw(&self, collection: &'static str, id: &str) -> Option<Value> {
        let id = id.to_string();
        self.ports
            .run("reading a document", move |ports| async move {
                ports.documents.get(collection, &id).await
            })
            .expect("the document reads")
            .map(|row| row.doc)
    }
}
