use super::*;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

use serde_json::json;
use std::sync::Arc;
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_document::memory::MemoryConnection;
use tinystoragedrivers_testkit::{MemoryBackend, TestClock};
use tinystoragedrivers_types::Scope;

#[tokio::test]
async fn owner_migration_rewrites_in_place_once_and_preserves_document_metadata() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = Arc::clone(storage.documents());
    docs.ensure_collection(&CollectionSpec::new("flows.items"))
        .await?;
    docs.put_with(
        "flows.items",
        "one",
        json!({"value": "before"}),
        Precondition::Absent,
        WriteOptions::new()
            .schema(1)
            .created_ms(123)
            .expires_ms(i64::MAX),
    )
    .await?;
    let before = docs
        .query("flows.items", &Query::filter(Filter::default()).limit(100))
        .await?;
    assert_eq!(before.items.len(), 1);
    assert_eq!(before.items[0].meta.schema, 1);

    let schema = Schema::new(Owner::new("flows")?)
        .collection_with(2, CollectionSpec::new("flows.items"), ScopeClass::Tenant)
        .step(Step::new("items_v2", "flows.items", 1, 2, |rows| {
            assert_eq!(rows.len(), 1, "one source row should be passed to the step");
            Ok(rows
                .into_iter()
                .map(|row| TargetRecord::new("flows.items", row.id, json!({"value": "after"})))
                .collect())
        }));

    let first = schema.migrate(&docs, &MigrateOptions::default()).await?;
    let stored = docs.get("flows.items", "one").await?;
    assert!(stored.is_some(), "migration report: {first:?}");
    let stored = stored.ok_or("row retained")?;
    assert_eq!(stored.doc["value"], "after");
    assert_eq!(stored.meta.schema, 2);
    assert_eq!(stored.meta.created_ms, 123);
    assert_eq!(stored.meta.expires_ms, Some(i64::MAX));
    assert_eq!(first.ran.len(), 1);

    let second = schema.migrate(&docs, &MigrateOptions::default()).await?;
    assert_eq!(second.already_done, 1);
    assert_eq!(
        docs.get("flows.items", "one")
            .await?
            .ok_or("row retained")?
            .doc["value"],
        "after"
    );
    Ok(())
}

#[tokio::test]
async fn owner_migration_rejects_same_id_conflicts_in_a_different_declared_collection() -> TestResult
{
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = Arc::clone(storage.documents());
    docs.ensure_collection(&CollectionSpec::new("legacy.items"))
        .await?;
    docs.put_with(
        "legacy.items",
        "same-id",
        json!({"value": "legacy"}),
        Precondition::Absent,
        WriteOptions::new().schema(1),
    )
    .await?;
    docs.put_with(
        "flows.other",
        "same-id",
        json!({"value": "stale target"}),
        Precondition::Absent,
        WriteOptions::new().schema(1),
    )
    .await?;

    let schema = Schema::new(Owner::new("flows")?)
        .collection_with(2, CollectionSpec::new("flows.items"), ScopeClass::Tenant)
        .collection_with(9, CollectionSpec::new("flows.other"), ScopeClass::Tenant)
        .step(
            Step::new("items_v2", "flows.items", 1, 2, |rows| {
                Ok(rows
                    .into_iter()
                    .map(|row| {
                        TargetRecord::new("flows.other", row.id, json!({"value": "migrated"}))
                    })
                    .collect())
            })
            .source("legacy.items"),
        );

    let error = schema
        .migrate(&docs, &MigrateOptions::default())
        .await
        .err()
        .ok_or("a different collection's same-id row must not be overwritten")?;
    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert_eq!(
        docs.get("flows.other", "same-id")
            .await?
            .ok_or("target remains")?
            .doc["value"],
        "stale target"
    );
    assert!(docs.get("legacy.items", "same-id").await?.is_some());
    Ok(())
}

#[tokio::test]
async fn owner_migration_uses_the_declared_version_of_a_different_target_collection() -> TestResult
{
    let clock = TestClock::new(100);
    let connection = MemoryConnection::with_clock(Arc::new({
        let clock = clock.clone();
        move || clock.now_ms()
    }));
    let docs = connection.for_scope(Scope::new("local")?).documents();
    docs.ensure_collection(&CollectionSpec::new("legacy.items"))
        .await?;
    docs.put_with(
        "legacy.items",
        "one",
        json!({"value": "before"}),
        Precondition::Absent,
        WriteOptions::new().schema(1),
    )
    .await?;

    let schema = Schema::new(Owner::new("flows")?)
        .collection_with(2, CollectionSpec::new("flows.items"), ScopeClass::Tenant)
        .collection_with(9, CollectionSpec::new("flows.archive"), ScopeClass::Tenant)
        .step(
            Step::new("items_to_archive", "flows.items", 1, 2, |rows| {
                Ok(rows
                    .into_iter()
                    .map(|row| {
                        TargetRecord::new("flows.archive", row.id, json!({"value": "archived"}))
                            .created_ms(456)
                            .expires_ms(Some(789))
                    })
                    .collect())
            })
            .source("legacy.items"),
        );

    let report = schema.migrate(&docs, &MigrateOptions::default()).await?;
    assert_eq!(report.ran.len(), 1);
    let archived = docs
        .get("flows.archive", "one")
        .await?
        .ok_or("archived row missing")?;
    assert_eq!(archived.meta.schema, 9);
    assert!(archived.meta.created_ms > 0);
    assert_ne!(archived.meta.created_ms, 456);
    assert_eq!(archived.meta.expires_ms, Some(789));
    assert_eq!(archived.doc["value"], "archived");
    assert_eq!(clock.advance(700), 800);
    assert!(docs.get("flows.archive", "one").await?.is_none());
    Ok(())
}

#[tokio::test]
async fn owner_migration_reads_compatible_shared_progress_records() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = Arc::clone(storage.documents());
    let key = composite_key(&["flows", "flows.items", "items_v2"])?;
    docs.put_as(
        "tsd.migrations",
        key.as_str(),
        &json!({
            "owner": "flows",
            "step": "items_v2",
            "from": 1,
            "to": 2,
            "cursor": null,
            "written": 0,
            "done": true
        }),
        Precondition::Absent,
    )
    .await?;
    let schema = Schema::new(Owner::new("flows")?)
        .collection_with(2, CollectionSpec::new("flows.items"), ScopeClass::Tenant)
        .step(Step::new("items_v2", "flows.items", 1, 2, |_| {
            Err(StorageError::backend(
                "completed migration should skip the step",
            ))
        }));

    let report = schema.migrate(&docs, &MigrateOptions::default()).await?;
    assert_eq!(report.already_done, 1);
    Ok(())
}

#[tokio::test]
async fn migration_rejects_zero_batches_non_object_outputs_and_undeclared_targets() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = Arc::clone(storage.documents());
    let schema = Schema::new(Owner::new("flows")?)
        .collection_with(2, CollectionSpec::new("flows.items"), ScopeClass::Tenant)
        .step(Step::new("items_v2", "flows.items", 1, 2, |_| {
            Ok(vec![TargetRecord::new(
                "other.items",
                "one",
                json!(["not an object"]),
            )])
        }));
    let error = schema
        .migrate(&docs, &MigrateOptions { batch_size: 0 })
        .await
        .err()
        .ok_or("zero-sized migration batches are invalid")?;
    assert_eq!(error.kind(), ErrorKind::InvalidInput);

    docs.put_with(
        "flows.items",
        "one",
        json!({"old": true}),
        Precondition::Absent,
        WriteOptions::new().schema(1),
    )
    .await?;
    let error = schema
        .migrate(&docs, &MigrateOptions::default())
        .await
        .err()
        .ok_or("migration outputs must be objects")?;
    assert_eq!(error.kind(), ErrorKind::InvalidInput);

    let undeclared = Schema::new(Owner::new("flows")?)
        .collection_with(2, CollectionSpec::new("flows.items"), ScopeClass::Tenant)
        .step(Step::new("items_v2", "flows.items", 1, 2, |_| {
            Ok(vec![TargetRecord::new(
                "other.items",
                "one",
                json!({"new": true}),
            )])
        }));
    let error = undeclared
        .migrate(&docs, &MigrateOptions::default())
        .await
        .err()
        .ok_or("migration cannot write outside declared owner collections")?;
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(docs.get("flows.items", "one").await?.is_some());
    Ok(())
}

#[test]
fn owner_and_progress_keys_validate_and_keep_the_legacy_encoding() -> TestResult {
    assert_eq!(Owner::new("flows")?.as_str(), "flows");
    for invalid in ["", "tsd", "Flows", "a", "bad-name", "has.dot"] {
        assert!(Owner::new(invalid).is_err(), "accepted owner {invalid:?}");
    }
    assert_eq!(
        composite_key(&["~flows%", "step/one\0"])?.as_str(),
        "%7Eflows%25/step%2Fone%00"
    );
    assert!(composite_key(&[]).is_err());
    Ok(())
}

#[tokio::test]
async fn stream_progress_and_external_completion_round_trip_in_the_shared_ledger() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = Arc::clone(storage.documents());
    let migrations = Migrations::new(&docs, "flows");
    assert!(!migrations.done("flows", "import").await?);
    migrations.mark_done("flows", "import").await?;
    assert!(migrations.done("flows", "import").await?);
    let progress = StreamProgress {
        owner: "flows".into(),
        step: "audit".into(),
        source: "legacy.audit".into(),
        target: "flows.audit".into(),
        source_offset: 2,
        target_offset: 4,
    };
    assert!(
        migrations
            .stream_progress("flows", "audit", "legacy.audit")
            .await?
            .is_none()
    );
    migrations.save_stream_progress(&progress).await?;
    let persisted = migrations
        .stream_progress("flows", "audit", "legacy.audit")
        .await?
        .ok_or("saved checkpoint")?;
    assert_eq!(persisted.source_offset, progress.source_offset);
    assert_eq!(persisted.target_offset, progress.target_offset);
    Ok(())
}

#[tokio::test]
async fn migration_accepts_an_identical_destination_after_a_lost_write_reply() -> TestResult {
    let storage = MemoryBackend::new().for_scope(&Scope::new("local")?)?;
    let docs = Arc::clone(storage.documents());
    docs.put_with(
        "legacy.items",
        "one",
        json!({"value": "before"}),
        Precondition::Absent,
        WriteOptions::new().schema(1),
    )
    .await?;
    docs.put_with(
        "flows.archive",
        "one",
        json!({"value": "after"}),
        Precondition::Absent,
        WriteOptions::new().schema(9),
    )
    .await?;
    let schema = Schema::new(Owner::new("flows")?)
        .collection_with(2, CollectionSpec::new("flows.items"), ScopeClass::Tenant)
        .collection_with(9, CollectionSpec::new("flows.archive"), ScopeClass::Tenant)
        .step(
            Step::new("archive_v1", "flows.items", 1, 2, |rows| {
                Ok(rows
                    .into_iter()
                    .map(|row| {
                        TargetRecord::new("flows.archive", row.id, json!({"value": "after"}))
                    })
                    .collect())
            })
            .source("legacy.items"),
        );
    let report = schema.migrate(&docs, &MigrateOptions::default()).await?;
    assert_eq!(report.ran[0].written, 1);
    assert!(docs.get("legacy.items", "one").await?.is_none());
    Ok(())
}
