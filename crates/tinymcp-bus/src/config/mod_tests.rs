//! Unit tests for configuration payloads sent to the MCP module.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    McpAuthConfig, McpClientConfig, McpServerConfig, OpenConfiguredRequest, OpenConfiguredResponse,
    TransitionStatus,
};
use serde_json::json;

#[test]
fn configured_open_keeps_directory_and_client_config_together_on_the_wire() {
    let wire = json!({
        "data_dir": "/workspace/agents/alpha",
        "client": {
            "servers": [{
                "name": "alpha-weather",
                "endpoint": "https://alpha.example.test/mcp",
                "auth": { "kind": "bearer_token", "token": "credential" }
            }]
        }
    });

    let request: OpenConfiguredRequest = serde_json::from_value(wire.clone()).unwrap();

    assert_eq!(request.data_dir, "/workspace/agents/alpha");
    assert_eq!(request.client.servers[0].name, "alpha-weather");
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "data_dir": "/workspace/agents/alpha",
            "client": {
                "enabled": true,
                "servers": [{
                    "name": "alpha-weather",
                    "endpoint": "https://alpha.example.test/mcp",
                    "command": "",
                    "args": [],
                    "env": {},
                    "cwd": null,
                    "description": null,
                    "enabled": true,
                    "allowed_tools": [],
                    "disallowed_tools": [],
                    "timeout_secs": 30,
                    "auth": { "kind": "bearer_token", "token": "credential" }
                }],
                "client_identity": {
                    "name": "tinymcp",
                    "title": "TinyMCP Client",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "registry_auth": {
                    "smithery_api_key": null,
                    "mcp_official_base": null,
                    "mcp_official_token": null
                },
                "proxy": null
            }
        })
    );
}

#[test]
fn configured_open_debug_never_exposes_the_user_path_or_credentials() {
    let request = OpenConfiguredRequest {
        data_dir: "/private/user/workspace".into(),
        client: McpClientConfig {
            servers: vec![McpServerConfig {
                name: "private-server".into(),
                auth: McpAuthConfig::BearerToken {
                    token: "sentinel-secret".into(),
                },
                ..McpServerConfig::default()
            }],
            ..McpClientConfig::default()
        },
    };

    let debug = format!("{request:?}");

    assert!(debug.contains("OpenConfiguredRequest"));
    assert!(!debug.contains("/private/user/workspace"));
    assert!(!debug.contains("private-server"));
    assert!(!debug.contains("sentinel-secret"));
}

#[test]
fn configured_open_returns_an_opaque_cleanup_transition() {
    let response = OpenConfiguredResponse {
        object_path: "/ai/tinyhumans/tinymcp/Dir/d42".into(),
        transition_id: "t17".into(),
    };

    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "object_path": "/ai/tinyhumans/tinymcp/Dir/d42",
            "transition_id": "t17"
        })
    );
    assert_eq!(
        serde_json::to_value(TransitionStatus::Pending).unwrap(),
        json!("pending")
    );
    assert_eq!(
        serde_json::to_value(TransitionStatus::Complete).unwrap(),
        json!("complete")
    );
}
