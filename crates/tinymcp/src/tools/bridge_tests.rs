//! Tests for the generic MCP bridge tools, end to end against a loopback MCP
//! server: what they declare, what they list without leaking, what they
//! scrub from a server's replies, and that the host's act gate runs first.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use tinymcp_bus::{HttpHeader, McpAuthConfig, McpClientConfig, McpServerConfig};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

use super::scrub::REDACTED;
use super::{ActGate, McpCallTool, McpListServersTool, McpListToolsTool};
use crate::McpServerRegistry;

const SECRET: &str = "s3cr3t-Value_91";

fn registry_with(endpoint: &str, auth: McpAuthConfig) -> Arc<McpServerRegistry> {
    Arc::new(
        McpServerRegistry::from_config(&McpClientConfig {
            servers: vec![McpServerConfig {
                name: "docs".into(),
                endpoint: endpoint.into(),
                description: Some("Docs MCP".into()),
                auth,
                ..McpServerConfig::default()
            }],
            ..McpClientConfig::default()
        })
        .unwrap(),
    )
}

fn test_registry() -> Arc<McpServerRegistry> {
    registry_with("https://example.com/mcp", McpAuthConfig::None)
}

fn every_auth_kind() -> Vec<(McpAuthConfig, &'static str)> {
    vec![
        (
            McpAuthConfig::BearerToken {
                token: SECRET.into(),
            },
            "bearer_token",
        ),
        (
            McpAuthConfig::Basic {
                username: "svc-user".into(),
                password: SECRET.into(),
            },
            "basic",
        ),
        (
            McpAuthConfig::Header {
                name: "X-Api-Key".into(),
                value: SECRET.into(),
            },
            "header",
        ),
        (
            McpAuthConfig::Headers {
                headers: vec![
                    HttpHeader::new("X-Org", "org-7"),
                    HttpHeader::new("X-Api-Key", SECRET),
                ],
            },
            "headers",
        ),
        (
            McpAuthConfig::QueryParam {
                name: "api_key".into(),
                value: SECRET.into(),
            },
            "query_param",
        ),
    ]
}

fn full_output(result: &ToolResult) -> String {
    format!(
        "{}\n{}\n{}",
        result.output(),
        result.markdown_formatted.clone().unwrap_or_default(),
        serde_json::to_string(&result.content).unwrap_or_default()
    )
}

/// What the loopback server reflects and how it fails.
#[derive(Clone)]
struct Reflect {
    reflected: String,
    fail: Option<&'static str>,
    calls: Arc<parking_lot::Mutex<Vec<Value>>>,
}

async fn handle(State(state): State<Reflect>, Json(body): Json<Value>) -> axum::response::Response {
    let method = body["method"].as_str().unwrap_or_default().to_string();
    if Some(method.as_str()) == state.fail {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("upstream rejected credential {}", state.reflected),
        )
            .into_response();
    }
    let echo = state.reflected.clone();
    let result = match method.as_str() {
        "initialize" => json!({
            "protocolVersion": tinymcp_bus::LATEST_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "echo", "version": "1.0.0" },
        }),
        "notifications/initialized" => return StatusCode::ACCEPTED.into_response(),
        "tools/list" => json!({
            "tools": [{
                "name": "whoami",
                "description": format!("Reports the key {echo}"),
                "inputSchema": { "type": "object", "properties": {} },
            }]
        }),
        "tools/call" => {
            state.calls.lock().push(body["params"]["arguments"].clone());
            json!({
                "content": [{ "type": "text", "text": format!("you sent {echo}") }],
                "structuredContent": { "key": echo, "nested": [echo.clone()] },
                "isError": false,
            })
        }
        _ => json!({}),
    };
    Json(json!({ "jsonrpc": "2.0", "id": body["id"].clone(), "result": result })).into_response()
}

/// Starts the server; returns its base URL and the `tools/call` arguments seen.
async fn echoing_server(
    echo: &str,
    fail: Option<&'static str>,
) -> (String, Arc<parking_lot::Mutex<Vec<Value>>>) {
    let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/mcp", post(handle))
        .with_state(Reflect {
            reflected: echo.to_string(),
            fail,
            calls: Arc::clone(&calls),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), calls)
}

fn allow_all() -> ActGate {
    Arc::new(|_| Ok(()))
}

fn call_tool(registry: Arc<McpServerRegistry>) -> McpCallTool {
    McpCallTool::new(registry, allow_all())
}

fn call_args() -> Value {
    json!({ "server": "docs", "tool": "whoami", "arguments": {} })
}

fn echoed_secret(auth: &McpAuthConfig) -> String {
    match auth {
        McpAuthConfig::Basic { username, password } => {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
        }
        _ => SECRET.to_string(),
    }
}

fn markdown() -> ToolCallOptions {
    ToolCallOptions {
        prefer_markdown: true,
    }
}

// The tool names, descriptions and schemas are a contract with every stored
// conversation and prompt cache; pin them as literals.
#[test]
fn declarations_are_byte_identical_to_the_host_originals() {
    let registry = test_registry();
    let list_servers = McpListServersTool::new(Arc::clone(&registry));
    assert_eq!(list_servers.name(), "mcp_list_servers");
    assert_eq!(
        list_servers.description(),
        "List named remote MCP servers registered in OpenHuman core. Use this before browsing tools on a specific MCP server."
    );
    assert_eq!(
        list_servers.parameters_schema(),
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    );
    assert_eq!(list_servers.permission_level(), PermissionLevel::ReadOnly);

    let list_tools = McpListToolsTool::new(Arc::clone(&registry));
    assert_eq!(list_tools.name(), "mcp_list_tools");
    assert_eq!(
        list_tools.description(),
        "List tools exposed by a named remote MCP server. Use this before calling `mcp_call_tool`."
    );
    assert_eq!(
        list_tools.parameters_schema(),
        json!({
            "type": "object",
            "properties": {"server": {"type": "string", "description": "Registered MCP server name from `mcp_list_servers`."}},
            "required": ["server"],
            "additionalProperties": false
        })
    );
    assert_eq!(list_tools.permission_level(), PermissionLevel::ReadOnly);

    let call = call_tool(registry);
    assert_eq!(call.name(), "mcp_call_tool");
    assert_eq!(
        call.description(),
        "Call a tool on a named remote MCP server. First inspect available tools with `mcp_list_tools`, then pass the remote tool name and its JSON arguments here."
    );
    assert_eq!(
        call.parameters_schema(),
        json!({
            "type": "object",
            "properties": {
                "server": {"type": "string", "description": "Registered MCP server name from `mcp_list_servers`."},
                "tool": {"type": "string", "description": "Remote MCP tool name from `mcp_list_tools`."},
                "arguments": {"type": "object", "description": "Arguments object passed through to the remote MCP tool."}
            },
            "required": ["server", "tool", "arguments"],
            "additionalProperties": false
        })
    );
    assert_eq!(call.permission_level(), PermissionLevel::Execute);

    let owner_specs = tinymcp_bus::bridge_tool_specs();
    assert_eq!(owner_specs.len(), 3);
    for (spec, wrapper) in owner_specs.iter().zip([
        (&list_servers as &dyn Tool),
        (&list_tools as &dyn Tool),
        (&call as &dyn Tool),
    ]) {
        assert_eq!(spec.name, wrapper.name());
        assert_eq!(spec.description, wrapper.description());
        assert_eq!(spec.parameters, wrapper.parameters_schema());
        assert_eq!(
            spec.effect,
            match wrapper.permission_level() {
                PermissionLevel::ReadOnly => tinymcp_bus::AgentToolEffect::Read,
                PermissionLevel::Execute => tinymcp_bus::AgentToolEffect::Execute,
                _ => panic!("unexpected permission level for a bridge tool"),
            }
        );
        assert!(!spec.deferred);
    }
}

#[tokio::test]
async fn list_servers_renders_registry_entries() {
    let result = McpListServersTool::new(test_registry())
        .execute(json!({}))
        .await
        .unwrap();
    assert!(result.output().contains("docs"));
    assert!(result.markdown_formatted.is_some());
}

#[tokio::test]
async fn list_tools_requires_server() {
    let result = McpListToolsTool::new(test_registry())
        .execute(json!({}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_servers_never_emits_auth_secrets() {
    for (auth, kind) in every_auth_kind() {
        let tool = McpListServersTool::new(registry_with("https://example.com/mcp", auth));
        let rendered = full_output(&tool.execute(json!({})).await.unwrap());
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(
            rendered.contains("\"auth_configured\":true"),
            "{kind}: {rendered}"
        );
        assert!(rendered.contains(kind), "{kind}: {rendered}");
    }
}

#[tokio::test]
async fn list_servers_reports_no_auth_as_unconfigured() {
    let rendered = full_output(
        &McpListServersTool::new(test_registry())
            .execute(json!({}))
            .await
            .unwrap(),
    );
    assert!(rendered.contains("\"auth_configured\":false"), "{rendered}");
    assert!(!rendered.contains("\"auth\":"), "{rendered}");
}

#[tokio::test]
async fn list_servers_strips_endpoint_query_string_and_userinfo() {
    let tool = McpListServersTool::new(registry_with(
        "https://svc-user:private12345@example.com/mcp?token=othersecret&v=2#fragment",
        McpAuthConfig::None,
    ));
    let rendered = full_output(&tool.execute(json!({})).await.unwrap());
    assert!(rendered.contains("https://example.com/mcp"), "{rendered}");
    for secret in ["svc-user", "private12345", "othersecret", "v=2", "fragment"] {
        assert!(!rendered.contains(secret), "leaked {secret}: {rendered}");
    }
}

#[tokio::test]
async fn failing_calls_redact_configured_secrets_from_errors() {
    for (auth, kind) in every_auth_kind() {
        let echo = echoed_secret(&auth);
        let (base, _) = echoing_server(&echo, Some("tools/call")).await;
        let registry = registry_with(&format!("{base}/mcp"), auth.clone());
        let result = call_tool(registry).execute(call_args()).await.unwrap();
        let rendered = full_output(&result);
        assert!(result.is_error, "{kind}: {rendered}");
        assert!(
            rendered.contains("mcp_call_tool failed"),
            "{kind}: {rendered}"
        );
        assert!(rendered.contains(REDACTED), "{kind}: {rendered}");
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(!rendered.contains(&echo), "{kind} leaked: {rendered}");

        let (base, _) = echoing_server(&echo, Some("tools/list")).await;
        let registry = registry_with(&format!("{base}/mcp"), auth);
        let result = McpListToolsTool::new(registry)
            .execute(json!({ "server": "docs" }))
            .await
            .unwrap();
        let rendered = full_output(&result);
        assert!(result.is_error, "{kind}: {rendered}");
        assert!(
            rendered.contains("mcp_list_tools failed"),
            "{kind}: {rendered}"
        );
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(!rendered.contains(&echo), "{kind} leaked: {rendered}");
    }
}

#[tokio::test]
async fn successful_results_redact_echoed_secrets() {
    for (auth, kind) in every_auth_kind() {
        let echo = echoed_secret(&auth);
        let (base, _) = echoing_server(&echo, None).await;
        let registry = registry_with(&format!("{base}/mcp"), auth);

        let result = call_tool(Arc::clone(&registry))
            .execute_with_options(call_args(), markdown())
            .await
            .unwrap();
        let rendered = full_output(&result);
        assert!(!result.is_error, "{kind}: {rendered}");
        assert!(
            rendered.contains("you sent [redacted]"),
            "{kind}: {rendered}"
        );
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(!rendered.contains(&echo), "{kind} leaked: {rendered}");

        let result = McpListToolsTool::new(registry)
            .execute(json!({ "server": "docs" }))
            .await
            .unwrap();
        let rendered = full_output(&result);
        assert!(!result.is_error, "{kind}: {rendered}");
        assert!(rendered.contains("whoami"), "{kind}: {rendered}");
        assert!(!rendered.contains(SECRET), "{kind} leaked: {rendered}");
        assert!(!rendered.contains(&echo), "{kind} leaked: {rendered}");
    }
}

#[tokio::test]
async fn endpoint_query_and_userinfo_secrets_are_redacted() {
    let (base, _) = echoing_server(SECRET, Some("tools/call")).await;
    let registry = registry_with(&format!("{base}/mcp?token={SECRET}"), McpAuthConfig::None);
    let result = call_tool(registry).execute(call_args()).await.unwrap();
    let rendered = full_output(&result);
    assert!(result.is_error, "{rendered}");
    assert!(!rendered.contains(SECRET), "{rendered}");

    let (base, _) = echoing_server("url-password-42", None).await;
    let endpoint = base.replace("http://", "http://mcp-user:url-password-42@");
    let registry = registry_with(&format!("{endpoint}/mcp"), McpAuthConfig::None);
    let result = call_tool(registry).execute(call_args()).await.unwrap();
    let rendered = full_output(&result);
    assert!(!result.is_error, "{rendered}");
    assert!(rendered.contains(REDACTED), "{rendered}");
    assert!(!rendered.contains("mcp-user"), "{rendered}");
    assert!(!rendered.contains("url-password-42"), "{rendered}");
}

#[tokio::test]
async fn encoded_query_secrets_are_redacted_from_successful_results() {
    let echo = "a+b%2Fc";
    let (base, _) = echoing_server(echo, None).await;
    let registry = registry_with(&format!("{base}/mcp?api_key={echo}"), McpAuthConfig::None);
    for result in [
        call_tool(Arc::clone(&registry))
            .execute_with_options(call_args(), markdown())
            .await
            .unwrap(),
        McpListToolsTool::new(registry)
            .execute(json!({ "server": "docs" }))
            .await
            .unwrap(),
    ] {
        let rendered = full_output(&result);
        assert!(!result.is_error, "{rendered}");
        assert!(rendered.contains(REDACTED), "{rendered}");
        assert!(!rendered.contains(echo), "{rendered}");
    }
}

#[tokio::test]
async fn call_tool_decodes_json_encoded_arguments() {
    let (base, calls) = echoing_server("plain", None).await;
    let registry = registry_with(&format!("{base}/mcp"), McpAuthConfig::None);
    let result = call_tool(registry)
        .execute(json!({ "server": "docs", "tool": "whoami", "arguments": "{\"q\":1}" }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", full_output(&result));
    assert_eq!(*calls.lock(), [json!({ "q": 1 })]);
}

#[tokio::test]
async fn call_tool_refuses_arguments_that_are_not_an_object_naming_the_type() {
    let result = call_tool(test_registry())
        .execute(json!({ "server": "docs", "tool": "whoami", "arguments": 7 }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("a number"), "{}", result.output());
}

#[tokio::test]
async fn call_tool_asks_the_act_gate_first_and_a_refusal_sends_nothing() {
    let (base, calls) = echoing_server("plain", None).await;
    let registry = registry_with(&format!("{base}/mcp"), McpAuthConfig::None);

    let asked = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&asked);
    let gate: ActGate = Arc::new(move |name| {
        assert_eq!(name, "mcp_call_tool");
        seen.fetch_add(1, Ordering::SeqCst);
        Err(anyhow::anyhow!("approval denied"))
    });
    let error = McpCallTool::new(Arc::clone(&registry), gate)
        .execute(call_args())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "approval denied");
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert!(calls.lock().is_empty(), "a refused call must send nothing");

    let result = call_tool(registry).execute(call_args()).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(calls.lock().len(), 1);
}

#[test]
fn every_bridge_tool_renders_markdown_and_debug_omits_the_gate() {
    let registry = test_registry();
    assert!(McpListServersTool::new(Arc::clone(&registry)).supports_markdown());
    assert!(McpListToolsTool::new(Arc::clone(&registry)).supports_markdown());
    let call = call_tool(registry);
    assert!(call.supports_markdown());
    let debug = format!("{call:?}");
    assert!(debug.starts_with("McpCallTool"), "{debug}");
    assert!(debug.contains(".."), "the act gate is elided: {debug}");
}

#[tokio::test]
async fn call_tool_without_arguments_is_refused_before_any_call() {
    let (base, calls) = echoing_server("plain", None).await;
    let registry = registry_with(&format!("{base}/mcp"), McpAuthConfig::None);
    let error = call_tool(registry)
        .execute(json!({ "server": "docs", "tool": "whoami" }))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "missing required `arguments` object");
    assert!(calls.lock().is_empty());
}

#[test]
fn an_endpoint_that_is_not_a_url_is_cut_at_its_query_or_fragment() {
    use super::bridge::endpoint_without_query;
    assert_eq!(endpoint_without_query("docs/mcp?api_key=x"), "docs/mcp");
    assert_eq!(endpoint_without_query("docs/mcp#frag"), "docs/mcp");
    assert_eq!(endpoint_without_query("docs/mcp"), "docs/mcp");
    assert_eq!(
        endpoint_without_query("https://u:p@example.com/mcp?k=v#f"),
        "https://example.com/mcp"
    );
}

#[test]
fn identifiers_lose_the_markdown_a_model_wraps_them_in() {
    use super::bridge::required_string_arg;
    let parsed = |value: &str| required_string_arg(&json!({ "server": value }), "server").unwrap();
    assert_eq!(parsed("docs"), "docs");
    assert_eq!(parsed("docs`"), "docs");
    assert_eq!(parsed("`docs`"), "docs");
    assert_eq!(parsed("*docs*"), "docs");
    assert_eq!(parsed("`docs`."), "docs");
    assert_eq!(parsed("  **docs**:  "), "docs");
    assert_eq!(parsed("***docs***!"), "docs");
    assert_eq!(parsed("my_docs-v2"), "my_docs-v2");
    assert_eq!(parsed("get_data_"), "get_data_");
    assert_eq!(parsed("__init"), "__init");
    assert_eq!(parsed("__init__"), "__init__");
    assert_eq!(parsed("_docs_"), "_docs_");
    assert_eq!(parsed("docs."), "docs.");
    assert_eq!(parsed("v1.2"), "v1.2");
    assert_eq!(parsed("`get_data_`"), "get_data_");
    let error = required_string_arg(&json!({ "server": "```" }), "server").unwrap_err();
    assert_eq!(error.to_string(), "missing required `server`");
    assert!(required_string_arg(&json!({ "server": 7 }), "server").is_err());
}

#[tokio::test]
async fn a_fenced_server_name_reaches_the_configured_server() {
    let (base, calls) = echoing_server("plain", None).await;
    let registry = registry_with(&format!("{base}/mcp"), McpAuthConfig::None);
    let result = call_tool(registry)
        .execute(json!({ "server": "`docs`", "tool": "whoami`", "arguments": {} }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", full_output(&result));
    assert_eq!(calls.lock().len(), 1);
}

/// The generic dispatcher reports the per-server tool it reaches, so a tool
/// rule written for that tool binds `mcp_call_tool` too.
#[test]
fn the_call_tool_reports_the_per_server_target_for_tool_rules() {
    let tool = call_tool(test_registry());
    let args =
        json!({ "server": "docs", "tool": "deleteGoal", "arguments": "{\"permanent\":true}" });
    let call = tool.indirect_target(&args).expect("a target");
    assert_eq!(
        call.target.name,
        crate::tools::naming::disambiguated_tool_name("docs", "docs", "deleteGoal")
    );
    assert_eq!(call.target.family.as_deref(), Some("docs"));
    assert!(
        call.target
            .tags
            .contains(&"mcp.tool:deleteGoal".to_string())
    );
    assert_eq!(
        call.arguments,
        Some(json!({ "permanent": true })),
        "encoded arguments are decoded"
    );

    let rules: tinytools::ToolRules = serde_json::from_value(json!({ "rules": [
        { "effect": "deny", "match": { "tags": "mcp.tool:delete*", "arg": { "pointer": "/permanent", "value": "true" } } },
    ] }))
    .unwrap();
    let set = tinytools::ToolRuleSet::single(rules);
    let context = tinytools::RuleContext::new();
    assert!(!set.evaluate_call(&tool, &context, &args).callable);
    let soft =
        json!({ "server": "docs", "tool": "deleteGoal", "arguments": { "permanent": false } });
    assert!(set.evaluate_call(&tool, &context, &soft).callable);
    assert!(tool.indirect_target(&json!({ "server": "docs" })).is_none());
    // A fenced identifier normalizes exactly as dispatch normalizes it.
    let fenced =
        json!({ "server": " docs ", "tool": "`deleteGoal`", "arguments": { "permanent": true } });
    assert!(!set.evaluate_call(&tool, &context, &fenced).callable);
}
