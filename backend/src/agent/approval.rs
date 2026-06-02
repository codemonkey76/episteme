use crate::mcp_host::McpTool;

/// Returns `true` if the tool requires explicit user approval before running.
///
/// Read-only tools run immediately; anything that mutates external state is gated.
/// In v1 the classification comes from the tool's own metadata flag (`requires_approval`),
/// set by the MCP server that registers the tool.
pub fn requires_approval(tool: &McpTool) -> bool {
    tool.requires_approval
}
