use super::*;

use serde::{Deserialize, Serialize};
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::{CollectionSpec, Scope};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Row {
    value: u32,
}

#[tokio::test]
async fn repo_update_is_atomic_and_conditional_delete_preserves_unmatched_rows() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = storage.documents();
    docs.ensure_collection(&CollectionSpec::new("flows.rows"))
        .await?;
    let repo = Repo::<Row>::new(docs, "flows.rows")?.with_retry(RetryPolicy::immediate());
    let (row, _) = repo
        .update("one", |old| {
            Ok(Some(Row {
                value: old.map_or(1, |row| row.value + 1),
            }))
        })
        .await?
        .ok_or("first write was not accepted")?;
    assert_eq!(row.value, 1);
    let (row, _) = repo
        .update("one", |old| {
            Ok(Some(Row {
                value: old.map_or(1, |row| row.value + 1),
            }))
        })
        .await?
        .ok_or("second write was not accepted")?;
    assert_eq!(row.value, 2);
    assert!(!repo.delete_if("one", |row| row.value == 1).await?);
    assert!(repo.delete_if("one", |row| row.value == 2).await?);
    Ok(())
}
