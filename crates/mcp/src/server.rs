use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::jsonrpc::{self, Request, Response};
use crate::tools::{self, McpState};

/// Run the MCP server over stdio.
pub async fn run_stdio() -> Result<()> {
    let state = Arc::new(McpState::new());
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    tracing::info!("MCP server started on stdio");

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        // Parse JSON-RPC request
        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::error(None, jsonrpc::error_codes::PARSE_ERROR, e.to_string());
                let json = serde_json::to_string(&resp)?;
                stdout.write_all(json.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
                continue;
            }
        };

        let response = handle_request(&state, request).await;
        let json = serde_json::to_string(&response)?;
        stdout.write_all(json.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn handle_request(state: &Arc<McpState>, request: Request) -> Response {
    match request.method.as_str() {
        "initialize" => handle_initialize(request),
        "notifications/initialized" => {
            // Client notification, no response needed but we return success
            Response::success(request.id, serde_json::json!(null))
        }
        "tools/list" => handle_list_tools(request),
        "tools/call" => handle_call_tool(state, request).await,
        "ping" => Response::success(request.id, serde_json::json!(null)),
        _ => Response::error(
            request.id,
            jsonrpc::error_codes::METHOD_NOT_FOUND,
            format!("Method not found: {}", request.method),
        ),
    }
}

fn handle_initialize(request: Request) -> Response {
    let result = serde_json::json!({
        "protocolVersion": "2025-03-26",
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "dbstudio",
            "version": env!("CARGO_PKG_VERSION")
        }
    });
    Response::success(request.id, result)
}

fn handle_list_tools(request: Request) -> Response {
    let tool_list = tools::list_tools();
    let result = serde_json::json!({
        "tools": tool_list
    });
    Response::success(request.id, result)
}

async fn handle_call_tool(state: &Arc<McpState>, request: Request) -> Response {
    let params: Value = request.params.clone();
    let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let arguments = params.get("arguments").cloned();

    let result = tools::call_tool(state, tool_name, arguments).await;
    Response::success(request.id, serde_json::to_value(result).unwrap_or_default())
}
