//! Compatibility tests for the host and owner MCP record converters.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use serde_json::json;

fn legacy_server(storage_id: &str) -> LegacyServerDocument {
    LegacyServerDocument {
        storage_id: storage_id.to_owned(),
        body: serde_json::from_value(json!({
            "server_id": "srv-1",
            "qualified_name": "@acme/files",
            "display_name": "Files",
            "command_kind": "node",
            "command": "npx",
            "args": ["-y", "files"],
            "env_keys": ["TOKEN"],
            "installed_at": 1_700_000_000_000_i64,
            "last_connected_at": 1_700_000_100_000_i64,
            "transport": "stdio",
            "catalog_managed": true,
            "future_owner_field": {"preserve": true}
        }))
        .unwrap(),
    }
}

#[test]
fn server_conversion_rekeys_and_renames_times_without_losing_optional_or_unknown_fields() {
    let converted = convert_legacy_server(legacy_server("srv/a%~")).unwrap();
    assert_eq!(converted.id, "srv%2Fa%25%7E");
    assert_eq!(converted.body.installed_ms, 1_700_000_000_000);
    assert_eq!(converted.body.last_connected_ms, Some(1_700_000_100_000));
    assert_eq!(converted.body.transport.as_deref(), Some("stdio"));
    assert_eq!(converted.body.catalog_managed, Some(true));
    assert_eq!(
        converted.body.extra["future_owner_field"],
        json!({"preserve": true})
    );

    let serialized = serde_json::to_value(converted.body).unwrap();
    assert_eq!(serialized["installed_ms"], 1_700_000_000_000_i64);
    assert_eq!(serialized["last_connected_ms"], 1_700_000_100_000_i64);
    assert!(serialized.get("installed_at").is_none());
    assert!(serialized.get("last_connected_at").is_none());
}

#[test]
fn server_conversion_preserves_absent_optional_fields_and_rejects_malformed_times() {
    let mut value = serde_json::to_value(legacy_server("srv-1")).unwrap();
    let body = value.as_object_mut().unwrap();
    let body = body.get_mut("body").unwrap().as_object_mut().unwrap();
    body.remove("last_connected_at");
    body.remove("description");
    let record: LegacyServerDocument = serde_json::from_value(value).unwrap();
    let converted = convert_legacy_server(record).unwrap();
    assert_eq!(converted.body.last_connected_ms, None);
    assert_eq!(converted.body.description, None);

    let malformed: Result<LegacyServerDocument, _> = serde_json::from_value(json!({
        "storage_id": "srv-1",
        "body": {
            "server_id": "srv-1",
            "qualified_name": "@acme/files",
            "display_name": "Files",
            "command_kind": "node",
            "command": "npx",
            "installed_at": "not-a-timestamp"
        }
    }));
    assert!(malformed.is_err());
}

#[test]
fn name_claim_conversion_escapes_the_qualified_name_and_defaults_missing_time() {
    let claim: LegacyServerNameClaim = serde_json::from_value(json!({
        "qualified_name": "@acme/files",
        "server_id": "srv-1",
        "claimed_at": 5_000,
        "future_claim_field": "preserve"
    }))
    .unwrap();
    let converted = convert_legacy_server_name_claim(claim).unwrap();
    assert_eq!(converted.id, "@acme%2Ffiles");
    assert_eq!(converted.claimed_ms, 5_000);
    assert_eq!(converted.extra["future_claim_field"], "preserve");

    let old: LegacyServerNameClaim = serde_json::from_value(json!({
        "qualified_name": "@acme/old",
        "server_id": "srv-0"
    }))
    .unwrap();
    assert_eq!(convert_legacy_server_name_claim(old).unwrap().claimed_ms, 0);
}

#[test]
fn name_claim_conversion_rejects_malformed_time_and_empty_key() {
    let malformed: Result<LegacyServerNameClaim, _> = serde_json::from_value(json!({
        "qualified_name": "@acme/files",
        "server_id": "srv-1",
        "claimed_at": "yesterday"
    }));
    assert!(malformed.is_err());

    let empty = LegacyServerNameClaim {
        qualified_name: String::new(),
        server_id: "srv-1".into(),
        claimed_at: None,
        extra: Default::default(),
    };
    assert!(convert_legacy_server_name_claim(empty).is_err());
}

#[test]
fn audit_stream_conversion_matches_only_the_legacy_family() {
    assert_eq!(
        legacy_audit_stream_target("mcp_audit"),
        Some("mcp.audit".into())
    );
    assert_eq!(
        legacy_audit_stream_target("mcp_audit/workspace/a"),
        Some("mcp.audit/workspace/a".into())
    );
    assert_eq!(legacy_audit_stream_target("mcp_audit_extra"), None);
    assert_eq!(
        legacy_audit_stream_target("mcp_audit/archive"),
        Some("mcp.audit/archive".into())
    );
}
