//! The remote tool's wire form, annotations included.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;

use super::*;

#[test]
fn a_listed_tools_annotations_are_kept() {
    let tool: McpRemoteTool = serde_json::from_value(json!({
        "name": "list_meetings",
        "inputSchema": {"type": "object"},
        "annotations": {"readOnlyHint": true, "openWorldHint": false},
    }))
    .unwrap();

    assert_eq!(
        tool.annotations,
        Some(json!({"readOnlyHint": true, "openWorldHint": false}))
    );
    assert!(tool.is_read_only());
}

#[test]
fn a_tool_without_the_hint_or_with_it_false_is_not_read_only() {
    for annotations in [
        None,
        Some(json!({})),
        Some(json!({"readOnlyHint": false})),
        Some(json!({"readOnlyHint": "yes"})),
    ] {
        let mut tool = McpRemoteTool::new("sign_out");
        tool.annotations = annotations.clone();
        assert!(!tool.is_read_only(), "{annotations:?}");
    }
}

#[test]
fn absent_annotations_stay_off_the_wire() {
    let wire = serde_json::to_value(McpRemoteTool::new("forecast")).unwrap();
    assert_eq!(
        wire,
        json!({"name": "forecast", "title": null, "description": null, "inputSchema": null})
    );

    let mut annotated = McpRemoteTool::new("forecast");
    annotated.annotations = Some(json!({"readOnlyHint": true}));
    let wire = serde_json::to_value(&annotated).unwrap();
    assert_eq!(wire["annotations"], json!({"readOnlyHint": true}));
    assert_eq!(
        serde_json::from_value::<McpRemoteTool>(wire).unwrap(),
        annotated
    );
}
