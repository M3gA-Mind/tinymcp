//! Tests for the pure registry-answer helpers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{ConnStatus, RegistryServerDetail, ServerStatus};

const QUALIFIED: &str = "io.github.ryudi84/uuid";
const AGENT: NotConnectedRemedy<'static> = NotConnectedRemedy {
    connect: None,
    status: "mcp_registry_status",
};
const RPC: NotConnectedRemedy<'static> = NotConnectedRemedy {
    connect: Some("mcp_clients_connect"),
    status: "mcp_clients_status",
};

fn install(server_id: &str) -> ConnStatus {
    ConnStatus {
        server_id: server_id.into(),
        qualified_name: QUALIFIED.into(),
        display_name: "uuid".into(),
        status: ServerStatus::Disconnected,
        tool_count: 0,
        last_error: None,
        auth_hint: None,
    }
}

#[test]
fn a_blank_identifier_is_refused_with_the_field_name() {
    assert_eq!(
        require_identifier("   ", "server_id").unwrap_err(),
        "server_id must not be empty"
    );
}

#[test]
fn an_identifier_is_trimmed() {
    assert_eq!(
        require_identifier("  srv-1  ", "server_id").unwrap(),
        "srv-1"
    );
}

#[test]
fn a_detail_renders_its_env_keys_beside_the_catalog_record() {
    let detail = ServerDetail {
        server: serde_json::from_value::<RegistryServerDetail>(serde_json::json!({
            "qualified_name": QUALIFIED,
            "display_name": "uuid",
        }))
        .unwrap(),
        required_env_keys: vec!["API_KEY".into()],
    };
    let json = detail.server_json().unwrap();
    assert_eq!(json["required_env_keys"], serde_json::json!(["API_KEY"]));
    assert_eq!(json["qualified_name"], QUALIFIED);
}

#[test]
fn an_agent_refusal_names_no_connect_call() {
    let NotConnectedExplanation::Refused(message) =
        explain_not_connected("srv-1", &[install("srv-1")], AGENT)
    else {
        panic!("an exact id is refused, not resolved");
    };
    assert!(message.starts_with("server_id=srv-1 is disconnected; it has to be connected"));
    assert!(!message.contains("mcp_clients_"));
}

#[test]
fn an_rpc_refusal_names_the_connect_call() {
    let NotConnectedExplanation::Refused(message) =
        explain_not_connected("srv-1", &[install("srv-1")], RPC)
    else {
        panic!("an exact id is refused");
    };
    assert!(message.contains("connect it first via mcp_clients_connect"));
}

#[test]
fn a_last_error_is_appended() {
    let mut failing = install("srv-1");
    failing.last_error = Some("boom".into());
    assert!(not_connected_message(&failing, RPC).ends_with("(last error: boom)"));
}

#[test]
fn a_qualified_name_resolves_to_its_single_install() {
    assert_eq!(
        explain_not_connected(QUALIFIED, &[install("srv-1")], AGENT),
        NotConnectedExplanation::Resolved("srv-1".into())
    );
}

#[test]
fn an_unknown_id_names_the_status_call_and_is_not_called_unconnected() {
    let NotConnectedExplanation::Refused(message) =
        explain_not_connected("absent", &[install("srv-1")], AGENT)
    else {
        panic!("unknown ids are refused");
    };
    assert!(message.contains("no installed MCP server"));
    assert!(message.contains("mcp_registry_status"));
    assert!(!message.contains("not connected"));
}

#[test]
fn a_registry_name_installed_twice_is_refused_with_both_ids() {
    let NotConnectedExplanation::Refused(message) =
        explain_not_connected(QUALIFIED, &[install("srv-a"), install("srv-b")], AGENT)
    else {
        panic!("an ambiguous name must not resolve");
    };
    assert!(message.contains("srv-a, srv-b"));
}
