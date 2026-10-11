//! Stable model-facing MCP declarations shared by the module and library adapters.

use std::collections::HashSet;

use tinymcp_bus::{McpToolExposure, McpToolPresentation, McpToolSourceSpec};

use super::{naming, schema::tool_parameters};
use crate::sanitize::sanitize_for_llm;

const MAX_LABEL_BYTES: usize = 120;
const MAX_DESCRIPTION_BYTES: usize = 500;

/// Builds the stable model-facing declarations for sources already screened by the host.
///
/// Source and tool ordering is deterministic, duplicate tool names within one source are
/// collapsed, and every result carries the same names, schemas, descriptions, exposure, and
/// policy tags as the `tinytools` adapter.
#[must_use]
pub fn prepare_agent_tools(sources: &[McpToolSourceSpec]) -> Vec<McpToolPresentation> {
    let mut ordered: Vec<&McpToolSourceSpec> = sources.iter().collect();
    ordered.sort_by(|left, right| left.server_id.cmp(&right.server_id));

    let mut taken = HashSet::new();
    let mut prepared = Vec::new();
    for source in ordered {
        let mut tools: Vec<&tinymcp_bus::McpTool> = source.tools.iter().collect();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools.dedup_by(|left, right| left.name == right.name);

        for tool in tools {
            if tool.name.trim().is_empty() {
                continue;
            }
            let name =
                naming::disambiguated_tool_name(&source.server_id, &source.label, &tool.name);
            if !taken.insert(name.clone()) {
                continue;
            }
            let display_name = if source.display_name.trim().is_empty() {
                &source.family
            } else {
                &source.display_name
            };
            let description = format!(
                "MCP server {}: {}",
                sanitize_for_llm(display_name, MAX_LABEL_BYTES),
                sanitize_for_llm(
                    tool.description.as_deref().unwrap_or(&tool.name),
                    MAX_DESCRIPTION_BYTES
                ),
            );
            let deferred = !source
                .direct_tools
                .iter()
                .any(|direct| direct.trim() == tool.name)
                && source.default_exposure != McpToolExposure::Direct;
            let family = sanitize_for_llm(&source.family, MAX_LABEL_BYTES);

            prepared.push(McpToolPresentation {
                legacy_name: naming::legacy_tool_name(&source.server_id, &tool.name),
                name,
                server_id: source.server_id.clone(),
                remote_name: tool.name.clone(),
                family,
                description,
                parameters: tool_parameters(&tool.input_schema),
                deferred,
                tags: vec![
                    format!("mcp.server:{}", source.family),
                    format!("mcp.server_id:{}", source.server_id),
                    format!("mcp.tool:{}", tool.name),
                ],
            });
        }
    }
    prepared
}
