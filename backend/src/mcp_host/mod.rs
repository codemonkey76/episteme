use anyhow::{anyhow, Result};
use rmcp::{
    model::CallToolRequestParams,
    service::RunningService,
    transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess},
    Peer, RoleClient, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

/// Separator between server name and tool name in the flat tool namespace
/// exposed to the model (e.g. `filesystem__read_file`).
const TOOL_SEP: &str = "__";

/// How long to wait for a server to come up (spawn/connect + initialize).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// Namespaced name (`server__tool`) — what the model sees and calls.
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Whether this tool mutates external state (requires approval).
    /// Derived from the server's `readOnlyHint` annotation; unused until
    /// approval resumption lands, but kept current for it.
    pub requires_approval: bool,
}

/// Connection status of one configured server, for the settings UI.
#[derive(Debug, Clone, Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    pub error: Option<String>,
}

/// One configured server and, if up, its live rmcp client.
struct ServerConn {
    config: McpServerConfig,
    client: Option<RunningService<RoleClient, ()>>,
    tools: Vec<McpTool>,
    error: Option<String>,
}

/// Manages connections to MCP servers and dispatches tool calls.
pub struct McpHost {
    servers: Vec<ServerConn>,
}

impl McpHost {
    pub fn new() -> Self {
        Self { servers: Vec::new() }
    }

    /// Per-server connection state, for the settings UI.
    pub fn status(&self) -> Vec<McpServerStatus> {
        self.servers
            .iter()
            .map(|s| McpServerStatus {
                name: s.config.name.clone(),
                connected: s.client.is_some(),
                tool_count: s.tools.len(),
                error: s.error.clone(),
            })
            .collect()
    }

    /// Drop all current connections and connect to the given servers.
    /// Returns one status per server so the caller can log outcomes.
    pub async fn connect_all(&mut self, configs: Vec<McpServerConfig>) -> Vec<McpServerStatus> {
        for conn in self.servers.drain(..) {
            disconnect_conn(conn).await;
        }
        for config in configs {
            self.servers.push(connect_one(config).await);
        }
        self.status()
    }

    /// (Re)connect a single server, replacing any existing connection of the
    /// same name. Returns its resulting status.
    pub async fn reconnect(&mut self, config: McpServerConfig) -> McpServerStatus {
        self.disconnect(&config.name).await;
        let conn = connect_one(config).await;
        let status = McpServerStatus {
            name: conn.config.name.clone(),
            connected: conn.client.is_some(),
            tool_count: conn.tools.len(),
            error: conn.error.clone(),
        };
        self.servers.push(conn);
        status
    }

    /// Disconnect and forget a server by name.
    pub async fn disconnect(&mut self, name: &str) {
        if let Some(pos) = self.servers.iter().position(|s| s.config.name == name) {
            disconnect_conn(self.servers.remove(pos)).await;
        }
    }

    /// Return the flat list of tools offered by all connected servers.
    /// Served from cache — called on every agent iteration.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        Ok(self.servers.iter().flat_map(|s| s.tools.clone()).collect())
    }

    /// Resolve a namespaced tool name to the peer handle that serves it.
    /// Cloning the peer lets the caller run the (possibly slow) tool call
    /// without holding the McpHost lock.
    pub fn peer_for(&self, tool_name: &str) -> Result<(Peer<RoleClient>, String)> {
        let (server, tool) = tool_name
            .split_once(TOOL_SEP)
            .ok_or_else(|| anyhow!("unknown tool '{tool_name}'"))?;
        let conn = self
            .servers
            .iter()
            .find(|s| s.config.name == server)
            .ok_or_else(|| anyhow!("no MCP server named '{server}'"))?;
        let client = conn
            .client
            .as_ref()
            .ok_or_else(|| anyhow!("MCP server '{server}' is not connected"))?;
        Ok((client.peer().clone(), tool.to_string()))
    }
}

/// Run a tool call on an already-resolved peer handle.
pub async fn call_on_peer(peer: &Peer<RoleClient>, tool: &str, args: Value) -> Result<Value> {
    let arguments = match args {
        Value::Object(map) => Some(map),
        Value::Null => None,
        other => Some(
            serde_json::json!({ "value": other })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        ),
    };

    let mut params = CallToolRequestParams::new(tool.to_string());
    if let Some(map) = arguments {
        params = params.with_arguments(map);
    }

    let result = peer
        .call_tool(params)
        .await
        .map_err(|e| anyhow!("MCP call '{tool}' failed: {e}"))?;

    // Prefer the structured result; otherwise join the text content blocks.
    let value = if let Some(structured) = result.structured_content {
        structured
    } else {
        let text: Vec<String> = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect();
        Value::String(text.join("\n"))
    };

    if result.is_error == Some(true) {
        return Err(anyhow!("tool '{tool}' returned an error: {value}"));
    }
    Ok(value)
}

/// Connect to one server and discover its tools. Failures are captured in
/// the returned `ServerConn` (error string, no client) rather than bubbled,
/// so one bad server never takes down the rest.
async fn connect_one(config: McpServerConfig) -> ServerConn {
    match tokio::time::timeout(CONNECT_TIMEOUT, try_connect(&config)).await {
        Ok(Ok((client, tools))) => ServerConn { config, client: Some(client), tools, error: None },
        Ok(Err(e)) => ServerConn { config, client: None, tools: Vec::new(), error: Some(e.to_string()) },
        Err(_) => ServerConn {
            config,
            client: None,
            tools: Vec::new(),
            error: Some(format!("timed out after {}s", CONNECT_TIMEOUT.as_secs())),
        },
    }
}

async fn try_connect(
    config: &McpServerConfig,
) -> Result<(RunningService<RoleClient, ()>, Vec<McpTool>)> {
    let client = match &config.transport {
        McpTransport::Stdio { command, args } => {
            let cmd = Command::new(command).configure(|c| {
                c.args(args);
            });
            ()
                .serve(TokioChildProcess::new(cmd)?)
                .await
                .map_err(|e| anyhow!("failed to start '{command}': {e}"))?
        }
        McpTransport::Http { url } => ()
            .serve(StreamableHttpClientTransport::from_uri(url.clone()))
            .await
            .map_err(|e| anyhow!("failed to connect to '{url}': {e}"))?,
    };

    let tools = client
        .peer()
        .list_all_tools()
        .await
        .map_err(|e| anyhow!("connected, but listing tools failed: {e}"))?
        .into_iter()
        .map(|t| McpTool {
            server: config.name.clone(),
            name: format!("{}{}{}", config.name, TOOL_SEP, t.name),
            description: t.description.map(|d| d.to_string()).unwrap_or_default(),
            input_schema: Value::Object((*t.input_schema).clone()),
            // Only tools explicitly marked read-only skip approval.
            requires_approval: !t
                .annotations
                .as_ref()
                .and_then(|a| a.read_only_hint)
                .unwrap_or(false),
        })
        .collect();

    Ok((client, tools))
}

async fn disconnect_conn(conn: ServerConn) {
    if let Some(client) = conn.client {
        // Graceful shutdown; ignore errors — the process/connection is going away.
        let _ = client.cancel().await;
    }
}
