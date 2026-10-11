//! Wire tests for configuration document reconciliation results.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{AppliedServer, ConfigApplyReport, ConfigApplyResponse};
use serde_json::json;

#[test]
fn config_apply_response_preserves_the_document_and_ordered_changes() {
    let response = ConfigApplyResponse {
        document: json!({ "mcpServers": { "weather": { "command": "weather" } } }),
        report: ConfigApplyReport {
            installed: vec![AppliedServer {
                server_id: "id-1".into(),
                name: "weather".into(),
            }],
            updated: Vec::new(),
            removed: Vec::new(),
            connect_queued: vec!["id-1".into()],
        },
    };

    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        json!({
            "document": { "mcpServers": { "weather": { "command": "weather" } } },
            "report": {
                "installed": [{ "server_id": "id-1", "name": "weather" }],
                "updated": [],
                "removed": [],
                "connect_queued": ["id-1"]
            }
        })
    );
}
