//! Tests for the storage names and shapes carried by the bus contract.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn collection_contract_keeps_v2_names_and_indexes() {
    let specs = collection_specs();
    let names: Vec<_> = specs.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(
        names,
        [
            SERVERS,
            SERVER_NAMES,
            REGISTRY_CACHE,
            TOOL_CACHE,
            REGISTRY_INDEX,
            REGISTRY_INDEX_STATE,
            MIGRATIONS,
        ]
    );
    let servers = specs.iter().find(|spec| spec.name == SERVERS).unwrap();
    let indexed: Vec<_> = servers
        .indexes
        .iter()
        .map(|index| index.name.as_str())
        .collect();
    assert_eq!(indexed, ["qualified_name", "installed_ms"]);
}

#[test]
fn ids_are_escaped_keys() {
    assert_eq!(doc_id(&["@acme/files"]).unwrap(), "@acme%2Ffiles");
    assert_eq!(doc_id(&["official", "@a/b"]).unwrap(), "official/@a%2Fb");
    assert!(doc_id(&[""]).is_err());
}

#[test]
fn composite_ids_escape_reserved_bytes_without_changing_component_boundaries() {
    assert_eq!(
        doc_id(&["~owner%", "part/with\0bytes"]).unwrap(),
        "%7Eowner%25/part%2Fwith%00bytes"
    );
    assert_eq!(doc_id(&["owner", "~leading"]).unwrap(), "owner/~leading");
    assert!(doc_id(&[]).is_err());
}

#[test]
fn overlong_composite_ids_keep_the_legacy_digest_encoding() {
    let component = "x".repeat(513);
    let id = doc_id(&[&component]).unwrap();
    assert_eq!(id.len(), 53);
    assert_eq!(id, "~gww6aciomttu22wqiicabhbdutruxav56d2pgf73zrpsn6nrajaq");
}

#[test]
fn write_options_pin_the_collection_schema_version() {
    assert_eq!(write_options().schema, SCHEMA_VERSION);
}
