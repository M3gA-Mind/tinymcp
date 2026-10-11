//! The tools that expose the installed-server registry to a model.
//!
//! Search the catalog, inspect a server, list installs and their status,
//! connect and disconnect, list and call a connected server's tools, and
//! uninstall. There is deliberately no install tool: a server is declared by
//! the user, never installed by a model from a catalog listing.

use serde_json::json;

use super::types::{AgentToolEffect, AgentToolSpec};

/// One of the registry's agent tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RegistryTool {
    /// `mcp_registry_search` — search the catalog.
    Search,
    /// `mcp_registry_get` — one catalog server's detail.
    Get,
    /// `mcp_registry_installed_list` — the installed servers.
    InstalledList,
    /// `mcp_registry_status` — each install's connection status.
    Status,
    /// `mcp_registry_list_tools` — a connected server's tools.
    ListTools,
    /// `mcp_registry_connect` — connect an install.
    Connect,
    /// `mcp_registry_disconnect` — disconnect an install.
    Disconnect,
    /// `mcp_registry_tool_call` — call a tool on a connected server.
    ToolCall,
    /// `mcp_registry_uninstall` — remove an install.
    Uninstall,
}

impl RegistryTool {
    /// Every registry tool, in the order a host registers them.
    pub const ALL: [Self; 9] = [
        Self::Search,
        Self::Get,
        Self::InstalledList,
        Self::Status,
        Self::ListTools,
        Self::Connect,
        Self::Disconnect,
        Self::ToolCall,
        Self::Uninstall,
    ];

    /// The tool's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Search => "mcp_registry_search",
            Self::Get => "mcp_registry_get",
            Self::InstalledList => "mcp_registry_installed_list",
            Self::Status => "mcp_registry_status",
            Self::ListTools => "mcp_registry_list_tools",
            Self::Connect => "mcp_registry_connect",
            Self::Disconnect => "mcp_registry_disconnect",
            Self::ToolCall => "mcp_registry_tool_call",
            Self::Uninstall => "mcp_registry_uninstall",
        }
    }

    /// The registry tool called `name`, if there is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// The tool's spec.
    ///
    /// The descriptions and schemas are the ones the tools have always
    /// shipped with; see [`AgentToolSpec`] on why they must not drift.
    #[must_use]
    pub fn spec(self) -> AgentToolSpec {
        let (description, parameters, effect, deferred) = match self {
            Self::Search => (
                "Search the MCP server registry catalog by `query`, optionally filtered by \
                 `transport` (\"stdio\" | \"hosted\" | \"all\"), paginated by `page` / \
                 `page_size`. Use to discover installable MCP servers.",
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "transport": { "type": "string", "enum": ["stdio", "hosted", "all"] },
                        "page": { "type": "integer", "minimum": 1 },
                        "page_size": { "type": "integer", "minimum": 1 }
                    }
                }),
                AgentToolEffect::Read,
                true,
            ),
            Self::Get => (
                "Get one MCP registry server's detail by `qualified_name`.",
                json!({
                    "type": "object",
                    "properties": { "qualified_name": { "type": "string" } },
                    "required": ["qualified_name"]
                }),
                AgentToolEffect::Read,
                true,
            ),
            Self::InstalledList => (
                "List the MCP servers currently installed for this user.",
                json!({ "type": "object", "properties": {} }),
                AgentToolEffect::Read,
                true,
            ),
            Self::Status => (
                "Report the connection status of installed MCP servers.",
                json!({ "type": "object", "properties": {} }),
                AgentToolEffect::Read,
                false,
            ),
            Self::ListTools => (
                "List the tools (name, description, input schema) exposed by a \
                 connected MCP server, given its `server_id`. Use this to discover \
                 what a connected server can do before calling `mcp_registry_tool_call`. \
                 The server must already be connected (see `mcp_registry_status` / \
                 `mcp_registry_connect`).",
                server_id_schema(),
                AgentToolEffect::Read,
                false,
            ),
            Self::Connect => (
                "Connect (spawn + handshake) an installed MCP server by `server_id`, \
                 returning its tools.",
                server_id_schema(),
                AgentToolEffect::Execute,
                false,
            ),
            Self::Disconnect => (
                "Disconnect (stop) a connected MCP server by `server_id`.",
                server_id_schema(),
                AgentToolEffect::Execute,
                false,
            ),
            Self::ToolCall => (
                "Invoke a tool on a connected MCP server: `server_id` + `tool_name` + \
                 `arguments` object.",
                json!({
                    "type": "object",
                    "properties": {
                        "server_id": { "type": "string" },
                        "tool_name": { "type": "string" },
                        "arguments": { "type": "object" }
                    },
                    "required": ["server_id", "tool_name"]
                }),
                AgentToolEffect::Execute,
                false,
            ),
            Self::Uninstall => (
                "Uninstall an installed MCP server by `server_id`. Default-OFF (opt-in).",
                server_id_schema(),
                AgentToolEffect::Write,
                false,
            ),
        };

        AgentToolSpec {
            name: self.name().to_string(),
            description: description.to_string(),
            parameters,
            effect,
            deferred,
        }
    }
}

/// Every registry tool's spec, in registration order.
#[must_use]
pub fn registry_tool_specs() -> Vec<AgentToolSpec> {
    RegistryTool::ALL
        .into_iter()
        .map(RegistryTool::spec)
        .collect()
}

/// The fixed tools for browsing and calling host-declared MCP servers.
///
/// These declarations are shared by the loadable module and the optional
/// `tinytools::Tool` wrappers so model-facing names and schemas do not drift.
#[must_use]
pub fn bridge_tool_specs() -> Vec<AgentToolSpec> {
    vec![
        AgentToolSpec {
            name: "mcp_list_servers".to_string(),
            description: "List named remote MCP servers registered in OpenHuman core. Use this before browsing tools on a specific MCP server.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            effect: AgentToolEffect::Read,
            deferred: false,
        },
        AgentToolSpec {
            name: "mcp_list_tools".to_string(),
            description: "List tools exposed by a named remote MCP server. Use this before calling `mcp_call_tool`.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "Registered MCP server name from `mcp_list_servers`."
                    }
                },
                "required": ["server"],
                "additionalProperties": false
            }),
            effect: AgentToolEffect::Read,
            deferred: false,
        },
        AgentToolSpec {
            name: "mcp_call_tool".to_string(),
            description: "Call a tool on a named remote MCP server. First inspect available tools with `mcp_list_tools`, then pass the remote tool name and its JSON arguments here.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "Registered MCP server name from `mcp_list_servers`."
                    },
                    "tool": {
                        "type": "string",
                        "description": "Remote MCP tool name from `mcp_list_tools`."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments object passed through to the remote MCP tool."
                    }
                },
                "required": ["server", "tool", "arguments"],
                "additionalProperties": false
            }),
            effect: AgentToolEffect::Execute,
            deferred: false,
        },
    ]
}

/// The schema of a tool addressed by one installed server.
fn server_id_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": { "server_id": { "type": "string" } },
        "required": ["server_id"]
    })
}
