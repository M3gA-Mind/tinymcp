#![allow(clippy::unwrap_used)]

use super::McpToolExposure;
use crate::McpClientIdentityConfig;
use serde_json::json;

#[test]
fn exposure_defaults_to_deferred_and_uses_snake_case_on_the_wire() {
    assert_eq!(McpToolExposure::default(), McpToolExposure::Deferred);
    for (value, wire) in [
        (McpToolExposure::Direct, "direct"),
        (McpToolExposure::Deferred, "deferred"),
    ] {
        let encoded = serde_json::to_value(value).unwrap();
        assert_eq!(encoded, json!(wire));
        assert_eq!(
            serde_json::from_value::<McpToolExposure>(encoded).unwrap(),
            value
        );
    }
}

#[test]
fn client_identity_round_trips_and_fills_defaults_from_partial_input() {
    let default = McpClientIdentityConfig::default();
    let encoded = serde_json::to_value(&default).unwrap();
    let decoded: McpClientIdentityConfig = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, default);

    let partial: McpClientIdentityConfig =
        serde_json::from_value(json!({ "name": "host" })).unwrap();
    assert_eq!(partial.name, "host");
    assert_eq!(partial.title, default.title);
    assert_eq!(partial.version, default.version);
}

#[cfg(feature = "schemars")]
#[test]
fn exposure_and_identity_generate_json_schemas() {
    let exposure = serde_json::to_value(schemars::schema_for!(McpToolExposure)).unwrap();
    assert!(exposure.to_string().contains("deferred"));
    let identity = serde_json::to_value(schemars::schema_for!(McpClientIdentityConfig)).unwrap();
    assert!(identity.to_string().contains("capabilities"));
}
