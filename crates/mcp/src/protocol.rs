use serde::{Deserialize, Serialize};
use serde_json::Value;

/// MCP initialize request params.
#[derive(Debug, Deserialize)]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: Option<ClientInfo>,
}

/// Client capabilities.
#[derive(Debug, Deserialize, Default)]
pub struct ClientCapabilities {
    #[serde(default)]
    pub roots: Option<Value>,
    #[serde(default)]
    pub sampling: Option<Value>,
}

/// Client info.
#[derive(Debug, Deserialize, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: Option<String>,
}

/// Server capabilities for MCP.
#[derive(Debug, Serialize)]
pub struct ServerCapabilities {
    pub tools: Option<ToolsCapability>,
    pub resources: Option<Value>,
    pub prompts: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// MCP tool definition.
#[derive(Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// MCP tool call arguments.
#[derive(Debug, Deserialize)]
pub struct CallToolArguments {
    pub name: String,
    #[serde(default)]
    pub arguments: Option<Value>,
}

/// MCP tool result content item.
#[derive(Debug, Serialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

/// MCP tool result.
#[derive(Debug, Serialize)]
pub struct CallToolResult {
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    pub content: Vec<ToolContent>,
}

impl CallToolResult {
    pub fn success(text: String) -> Self {
        Self {
            is_error: None,
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text,
            }],
        }
    }

    pub fn error(text: String) -> Self {
        Self {
            is_error: Some(true),
            content: vec![ToolContent {
                content_type: "text".to_string(),
                text,
            }],
        }
    }
}

/// List tools result.
#[derive(Debug, Serialize)]
pub struct ListToolsResult {
    pub tools: Vec<Tool>,
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
