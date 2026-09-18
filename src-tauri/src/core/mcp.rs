//! MCP stdio adapter over the authenticated Core IPC client.

use super::ipc::{CoreClient, IPC_METHODS};
use super::types::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

pub fn tool_descriptors() -> Vec<Value> {
    IPC_METHODS
        .iter()
        .map(|name| {
            let (description, properties, required) = match *name {
                "context.search" => (
                    "Search indexed source snapshots through the local Core.",
                    json!({
                        "query": {"type":"string"},
                        "provider": {"type":["string","null"]},
                        "kind": {"type":["string","null"]},
                        "limit": {"type":["integer","null"],"minimum":1,"maximum":50}
                    }),
                    vec!["query"],
                ),
                "context.get" => (
                    "Get a WorkContext for an explicit source ID or SourceRef.",
                    json!({
                        "source_id": {"type":["string","null"]},
                        "source": {"type":["object","null"]},
                        "max_age_seconds": {"type":["integer","null"]}
                    }),
                    Vec::new(),
                ),
                "memory.upsert" => (
                    "Create or update a user-owned memory with evidence and version guards.",
                    json!({
                        "id": {"type":["string","null"]},
                        "body": {"type":"string"},
                        "reason": {"type":"string"},
                        "evidence": {"type":"array", "items":{"type":"object"}},
                        "author": {"type":"string"},
                        "claim_type": {"type":"string"},
                        "idempotency_key": {"type":"string"},
                        "expected_version": {"type":["integer","null"],"minimum":0},
                        "supersedes_id": {"type":["string","null"]}
                    }),
                    vec![
                        "body",
                        "reason",
                        "evidence",
                        "author",
                        "claim_type",
                        "idempotency_key",
                    ],
                ),
                "memory.retract" => (
                    "Retract a user-owned memory without deleting its body or history.",
                    json!({
                        "id": {"type":"string"},
                        "expected_version": {"type":"integer","minimum":1},
                        "idempotency_key": {"type":"string"}
                    }),
                    vec!["id", "expected_version", "idempotency_key"],
                ),
                "sources.refresh" => (
                    "Refresh an explicitly supplied read-only fixture through the Core.",
                    json!({"fixture_path":{"type":"string"}}),
                    vec!["fixture_path"],
                ),
                "connections.status" => (
                    "Read the last success and failure state for one connection.",
                    json!({"connection_id":{"type":"string"}}),
                    vec!["connection_id"],
                ),
                _ => (
                    "Aidebook Core method.",
                    json!({"type":"object"}),
                    Vec::new(),
                ),
            };
            json!({
                "name": name,
                "description": description,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": true
                }
            })
        })
        .collect()
}

pub fn handle_message(client: &CoreClient, line: &str) -> Option<String> {
    let request = match serde_json::from_str::<JsonRpcRequest>(line) {
        Ok(request) => request,
        Err(error) => {
            return Some(encode_error(
                None,
                -32700,
                format!("invalid JSON: {error}"),
                None,
            ))
        }
    };
    if request.id.is_none() && request.method.starts_with("notifications/") {
        return None;
    }
    let id = request.id.clone();
    let response = match request.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "aidebook", "version": env!("CARGO_PKG_VERSION")}
            })),
            error: None,
        },
        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({"tools": tool_descriptors()})),
            error: None,
        },
        "tools/call" => {
            let name = request
                .params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !IPC_METHODS.contains(&name) {
                JsonRpcResponse {
                    jsonrpc: "2.0",
                    id,
                    result: Some(json!({
                        "content":[{"type":"text","text":format!("unknown Aidebook tool: {name}")}],
                        "isError": true
                    })),
                    error: None,
                }
            } else {
                match client.call(name, arguments) {
                    Ok(result) => JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: Some(json!({
                            "content":[{"type":"text","text":serde_json::to_string(&result).unwrap_or_else(|_| "null".to_string())}],
                            "isError": false
                        })),
                        error: None,
                    },
                    Err(error) => JsonRpcResponse {
                        jsonrpc: "2.0",
                        id,
                        result: Some(json!({
                            "content":[{"type":"text","text":error.to_string()}],
                            "isError": true
                        })),
                        error: None,
                    },
                }
            }
        }
        "ping" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({})),
            error: None,
        },
        _ => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "method not found".to_string(),
                data: None,
            }),
        },
    };
    serde_json::to_string(&response).ok()
}

pub fn serve_stdio(client: &CoreClient) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if let Some(response) = handle_message(client, &line) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn encode_error(id: Option<Value>, code: i64, message: String, data: Option<Value>) -> String {
    serde_json::to_string(&JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError { code, message, data }),
    })
    .unwrap_or_else(|_| "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"encoding failed\"}}".to_string())
}

#[allow(dead_code)]
fn _mcp_error_code(_error: &CoreError) -> i64 {
    -32000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_list_has_the_six_core_methods() {
        let tools = tool_descriptors();
        assert_eq!(tools.len(), 6);
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            IPC_METHODS.to_vec()
        );
    }
}
