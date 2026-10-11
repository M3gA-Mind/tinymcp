//! Unit tests for configuration payloads sent to the MCP module.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::OpenConfiguredRequest;
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
                    "version": "0.6.0"
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
