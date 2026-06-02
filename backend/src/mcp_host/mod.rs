use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    Stdio { command: String, args: Vec<String> },
    Http { url: String },
}

/// A tool discovered from a connected MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub server: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Whether this tool mutates external state (requires approval).
    pub requires_approval: bool,
}

/// Manages connections to MCP servers and dispatches tool calls.
pub struct McpHost {
    servers: Vec<McpServerConfig>,
    // TODO: hold live rmcp ServiceHandle per server once wired in
}

impl McpHost {
    pub fn new() -> Self {
        Self { servers: Vec::new() }
    }

    pub fn set_servers(&mut self, servers: Vec<McpServerConfig>) {
        self.servers = servers;
    }

    pub fn servers(&self) -> &[McpServerConfig] {
        &self.servers
    }

    /// Return the flat list of tools offered by all connected servers.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        // TODO: query each rmcp ServiceHandle and aggregate
        Ok(Vec::new())
    }

    /// Execute a tool call and return its JSON result.
    pub async fn execute(&self, tool_name: &str, args: Value) -> Result<Value> {
        // TODO: dispatch via rmcp ServiceHandle
        let _ = args;
        anyhow::bail!("mcp_host stub — '{tool_name}' not wired yet")
    }
}
