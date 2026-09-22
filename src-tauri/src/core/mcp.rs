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

fn connection_properties() -> Value {
    json!({
        "id":{"type":"string","description":"Stable unique connection ID; use a new ID to add another vault/account."},
        "provider":{"type":"string","enum":["obsidian","github","jira","confluence"]},
        "label":{"type":"string"},
        "account":{"type":"string","description":"Empty for Obsidian, GitHub login or Atlassian email."},
        "scope":{"type":"string","description":"Absolute vault path, owner/repository, or https://tenant.atlassian.net."},
        "project":{"type":"string","description":"Required only for Jira project scope."},
        "jira_scope":{"type":"string","enum":["project","mine"],"description":"Legacy default project; choose mine for cross-project assigned/created issues."},
        "jira_include_reporter":{"type":"boolean"},
        "jira_include_parents":{"type":"boolean"},
        "confluence_mode":{"type":"string","enum":["authored","watched","selected"]},
        "confluence_page_ids":{"type":"array","items":{"type":"string"}},
        "auth":{"type":"string","enum":["local","gh_cli","token"]},
        "auto_sync":{"type":"boolean","default":true,"description":"Automatic refresh applies to local Obsidian vaults only."}
    })
}

pub fn tool_descriptors() -> Vec<Value> {
    IPC_METHODS
        .iter()
        .map(|name| {
            let (description, properties, required) = match *name {
                "plugins.list" => (
                    "List saved Aidebook source connections (Obsidian, GitHub, Jira, Confluence), without credentials.",
                    json!({}), Vec::new(),
                ),
                "plugins.get" => (
                    "Read one saved source connection by its stable ID, without credentials.",
                    json!({"id":{"type":"string"}}), vec!["id"],
                ),
                "plugins.add" => (
                    "Add an explicitly requested source connection without replacing existing connections. This configures built-in providers; it does not install executable plugins. Tokens must be set in the app.",
                    connection_properties(), vec!["id","provider","label","account","scope","auth"],
                ),
                "plugins.update" => {
                    let mut properties = connection_properties();
                    properties.as_object_mut().unwrap().remove("id");
                    properties.as_object_mut().unwrap().remove("provider");
                    (
                        "Partially edit an explicitly selected connection. ID/provider and omitted fields are preserved. Does not change stored credentials.",
                        json!({"id":{"type":"string"}, "changes":{"type":"object","properties":properties,"additionalProperties":false}}),
                        vec!["id","changes"],
                    )
                },
                "plugins.remove" => (
                    "Disconnect the explicitly selected source connection and stop its automatic refresh. Preserve source files, cached evidence, memories and credentials.",
                    json!({"id":{"type":"string"}}), vec!["id"],
                ),
                "plugins.confluence.search" => (
                    "Search accessible Confluence pages using a saved connection. Returns candidates only; does not index or select pages. Save selected page IDs with plugins.update then plugins.refresh.",
                    json!({"id":{"type":"string"},"query":{"type":"string","minLength":1,"maxLength":500}}), vec!["id","query"],
                ),
                "plugins.refresh" => (
                    "Read and index the actual saved vault/repository/project using its existing authentication. Does not write to source files or remote providers.",
                    json!({"id":{"type":"string"}}), vec!["id"],
                ),
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
                "context.query.v1" => (
                    "Query sources, review-approved memories, graph evidence, freshness, and availability as one bounded context packet.",
                    json!({
                        "query": {"type":"string","maxLength":500},
                        "source_id": {"type":["string","null"]},
                        "source": {"type":["object","null"]},
                        "provider": {"type":["string","null"]},
                        "kind": {"type":["string","null"]},
                        "max_age_seconds": {"type":["integer","null"],"minimum":0},
                        "max_depth": {"type":["integer","null"],"minimum":1,"maximum":8},
                        "max_nodes": {"type":["integer","null"],"minimum":1,"maximum":200},
                        "max_edges": {"type":["integer","null"],"minimum":1,"maximum":400},
                        "max_sources": {"type":["integer","null"],"minimum":1,"maximum":50},
                        "max_memories": {"type":["integer","null"],"minimum":1,"maximum":100}
                    }),
                    vec!["query"],
                ),
                "observation.capture" => (
                    "Capture an evidence-backed local observation for later candidate distillation.",
                    json!({
                        "id": {"type":["string","null"]},
                        "session_id": {"type":"string"},
                        "body": {"type":"string"},
                        "evidence": {"type":"array", "items":{"type":"object"}},
                        "actor": {"type":"string"},
                        "idempotency_key": {"type":"string"}
                    }),
                    vec!["session_id", "body", "evidence", "actor", "idempotency_key"],
                ),
                "observation.get" => (
                    "Read one captured observation by ID.",
                    json!({"id":{"type":"string"}}),
                    vec!["id"],
                ),
                "candidate.distill" => (
                    "Distill one captured observation into a review candidate; it does not create a canonical memory.",
                    json!({
                        "observation_id": {"type":"string"},
                        "body": {"type":"string"},
                        "reason": {"type":"string"},
                        "author": {"type":"string"},
                        "claim_type": {"type":"string"},
                        "idempotency_key": {"type":"string"},
                        "expected_version": {"type":["integer","null"],"minimum":1}
                    }),
                    vec!["observation_id", "body", "reason", "author", "claim_type", "idempotency_key"],
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
                "candidate.propose" => (
                    "Move a distilled memory candidate into the human review queue. Acceptance remains a trusted Tauri review action.",
                    json!({
                        "id": {"type":"string"},
                        "expected_version": {"type":"integer","minimum":1},
                        "idempotency_key": {"type":"string"}
                    }),
                    vec!["id", "expected_version", "idempotency_key"],
                ),
                "candidate.get" => (
                    "Read one memory candidate and its review state by ID.",
                    json!({"id":{"type":"string"}}),
                    vec!["id"],
                ),
                "candidate.list" => (
                    "List memory candidates for a review queue; this never accepts or rejects a candidate.",
                    json!({"state":{"type":["string","null"],"enum":["captured","distilled","proposed","accepted","rejected"]}}),
                    Vec::new(),
                ),
                "workflow.save" => (
                    "Create or version-update a local work item or task. Full field replacement. Work completion requires desktop review; no external changes.",
                    json!({
                        "kind":{"type":"string","enum":["work","task"]},
                        "id":{"type":["string","null"]},
                        "expected_version":{"type":["integer","null"],"minimum":1},
                        "idempotency_key":{"type":"string","minLength":1,"maxLength":200},
                        "fields":{"type":"object","additionalProperties":false,"required":["title","status"],"properties":{
                            "title":{"type":"string","minLength":1},
                            "status":{"type":"string","enum":["planned","in_progress","review","on_hold","cancelled","done"]},
                            "purpose":{"type":"string"},
                            "blocked_reason":{"type":["string","null"]},
                            "work_id":{"type":["string","null"]},
                            "target_date":{"type":["string","null"],"description":"Local target date YYYY-MM-DD, never the remote due date"},
                            "priority":{"type":"integer","minimum":0,"maximum":3},
                            "time_blocks":{"type":"array","maxItems":100,"items":{"type":"object","additionalProperties":false,"required":["start","end"],"properties":{"start":{"type":"string","format":"date-time"},"end":{"type":"string","format":"date-time"}}}}
                        }}
                    }),
                    vec!["kind","idempotency_key","fields"],
                ),
                "workflow.get" => (
                    "Get one persistent work item or task.",
                    json!({"kind":{"type":"string","enum":["work","task"]},"id":{"type":"string"}}),
                    vec!["kind","id"],
                ),
                "workflow.list" => (
                    "Page local work items or tasks, newest update first. work_id filters tasks only.",
                    json!({"kind":{"type":"string","enum":["work","task"]},"work_id":{"type":["string","null"]},"limit":{"type":"integer","minimum":1,"maximum":100,"default":50},"offset":{"type":"integer","minimum":0,"default":0}}),
                    vec!["kind"],
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
                    "additionalProperties": !(name.starts_with("plugins.") || name.starts_with("workflow."))
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
    fn tool_list_has_legacy_and_review_safe_methods() {
        let tools = tool_descriptors();
        assert_eq!(tools.len(), IPC_METHODS.len());
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            IPC_METHODS.to_vec()
        );
        assert!(tools.iter().all(|tool| tool["name"] != "candidate.accept"));
    }
}
