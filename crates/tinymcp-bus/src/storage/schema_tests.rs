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
fn write_options_pin_the_collection_schema_version() {
    assert_eq!(write_options().schema, SCHEMA_VERSION);
}
