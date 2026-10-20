use super::*;

use serde::{Deserialize, Serialize};
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_testkit::{ConflictOnce, MemoryBackend};
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

#[tokio::test]
async fn repo_declines_updates_and_conditional_deletes_without_mutating_storage() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = storage.documents();
    docs.ensure_collection(&CollectionSpec::new("flows.rows"))
        .await?;
    let repo = Repo::<Row>::new(docs, "flows.rows")?.with_retry(RetryPolicy::immediate());
    assert!(repo.update("missing", |_| Ok(None)).await?.is_none());
    assert!(!repo.delete_if("missing", |_| true).await?);
    repo.update("one", |_| Ok(Some(Row { value: 1 }))).await?;
    assert!(repo.update("one", |_| Ok(None)).await?.is_none());
    assert!(!repo.delete_if("one", |_| false).await?);
    assert_eq!(
        docs.get("flows.rows", "one")
            .await?
            .ok_or("row missing")?
            .doc["value"],
        1
    );
    assert!(Repo::<Row>::new(docs, "bad/name").is_err());
    let clone = repo.clone();
    assert!(format!("{clone:?}").contains("flows.rows"));
    Ok(())
}

#[tokio::test]
async fn repo_reports_decode_and_update_closure_errors_without_overwriting_rows() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = storage.documents();
    docs.ensure_collection(&CollectionSpec::new("flows.rows"))
        .await?;
    docs.put(
        "flows.rows",
        "bad",
        serde_json::json!({"value": "wrong type"}),
        Precondition::Absent,
    )
    .await?;
    let repo = Repo::<Row>::new(docs, "flows.rows")?.with_retry(RetryPolicy::immediate());
    assert!(
        repo.update("bad", |_| Ok(Some(Row { value: 2 })))
            .await
            .is_err()
    );
    assert!(
        repo.update("new", |_| Err(
            tinystoragedrivers_types::StorageError::backend("closure")
        ))
        .await
        .is_err()
    );
    assert!(docs.get("flows.rows", "new").await?.is_none());
    let default = RetryPolicy::default();
    assert_eq!(default.attempts, 32);
    let first_pause = default.pause(0);
    assert!((500_000..=1_000_000).contains(&first_pause.as_nanos()));
    assert_eq!(RetryPolicy::immediate().pause(10), Duration::ZERO);
    Ok(())
}

#[tokio::test]
async fn repo_retries_a_lost_update_and_delete_compare_and_swap() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = storage.documents();
    docs.ensure_collection(&CollectionSpec::new("flows.rows"))
        .await?;
    docs.put(
        "flows.rows",
        "one",
        serde_json::json!({"value": 1}),
        tinystoragedrivers_types::Precondition::Absent,
    )
    .await?;

    let conflicted: Arc<dyn DocumentStore> = Arc::new(ConflictOnce::new(Arc::clone(docs)));
    let repo = Repo::<Row>::new(&conflicted, "flows.rows")?.with_retry(RetryPolicy::immediate());
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let (row, _) = repo
        .update("one", |old| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(Row {
                value: old.map_or(1, |row| row.value + 1),
            }))
        })
        .await?
        .ok_or("retry did not store an updated row")?;
    assert_eq!(row.value, 2);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);

    let conflicted: Arc<dyn DocumentStore> = Arc::new(ConflictOnce::new(Arc::clone(docs)));
    let repo = Repo::<Row>::new(&conflicted, "flows.rows")?.with_retry(RetryPolicy::immediate());
    assert!(repo.delete_if("one", |row| row.value == 2).await?);
    assert!(docs.get("flows.rows", "one").await?.is_none());

    docs.put(
        "flows.rows",
        "one",
        serde_json::json!({"value": 3}),
        tinystoragedrivers_types::Precondition::Absent,
    )
    .await?;
    let conflicted: Arc<dyn DocumentStore> = Arc::new(ConflictOnce::new(Arc::clone(docs)));
    let single_attempt = Repo::<Row>::new(&conflicted, "flows.rows")?.with_retry(RetryPolicy::new(
        1,
        Duration::ZERO,
        Duration::ZERO,
    ));
    let error = single_attempt
        .update("one", |_| Ok(Some(Row { value: 4 })))
        .await
        .err()
        .ok_or("the exhausted retry budget must return its conflict")?;
    assert_eq!(error.kind(), tinystoragedrivers_types::ErrorKind::Conflict);
    assert_eq!(
        docs.get("flows.rows", "one")
            .await?
            .ok_or("row disappeared")?
            .doc["value"],
        3
    );
    Ok(())
}
