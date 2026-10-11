//! Converting a rendered MCP result into a `tinytools` result.

pub use crate::processing::MAX_LLM_BLOCK_BYTES;
use crate::processing::{bound_json, bound_text};
use serde_json::Value;
use tinymcp_bus::{McpResultEnvelope, McpToolContent, McpToolResult};
use tinytools::{ToolContent, ToolResult};

/// The most a content block may serialize to before its payload is elided.
///
/// Text, JSON, formatted Markdown, and unrecognized blocks are bounded before
/// being returned to a model. Above this the payload is replaced with a marker.
/// Maps a rendered MCP result onto [`ToolResult`].
///
/// The two shapes match by construction, so this is a mapping rather than a
/// translation. It is written once because spelled out at each call site it
/// would be as many chances to get the error flag the wrong way round.
#[must_use]
pub fn tool_result(result: McpToolResult) -> ToolResult {
    ToolResult {
        content: result
            .content
            .into_iter()
            .map(|block| match block {
                McpToolContent::Text { text } => ToolContent::Text {
                    text: bound_text(text),
                },
                McpToolContent::Json { data } => ToolContent::Json {
                    data: bound_json(data),
                },
                // The contract's block enum is `#[non_exhaustive]`: a kind this
                // build does not model travels as its JSON, bounded.
                other => passthrough(&other),
            })
            .collect(),
        is_error: result.is_error,
        markdown_formatted: result.markdown_formatted.map(bound_text),
        ..ToolResult::default()
    }
}

/// Maps a rendered MCP result onto [`ToolResult`], as [`tool_result`] does,
/// and attaches an [`McpResultEnvelope`] for `tool` on `server` as its
/// host-only metadata.
///
/// The model-facing content is exactly what [`tool_result`] produces; the
/// envelope carries the reply's `structuredContent`, `_meta` and embedded
/// resources for a host that renders tool UI.
#[must_use]
pub fn tool_result_for(server: &str, tool: &str, result: McpToolResult) -> ToolResult {
    let envelope = McpResultEnvelope::new(server, tool, &result);
    let mut mapped = tool_result(result);
    mapped.metadata = Some(envelope.to_metadata());
    mapped
}

/// An unrecognized block, carried as bounded JSON.
pub(crate) fn passthrough(block: &McpToolContent) -> ToolContent {
    ToolContent::Json {
        data: elide_oversized_block(block),
    }
}

/// An unrecognized block as JSON, its payload elided above
/// [`MAX_LLM_BLOCK_BYTES`].
pub(crate) fn elide_oversized_block(block: &McpToolContent) -> Value {
    crate::processing::elide_oversized_block(block)
}
