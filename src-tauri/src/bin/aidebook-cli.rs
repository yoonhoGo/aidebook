use aidebook_lib::core::{mcp, CoreClient, CoreEndpoint, CoreError};
use serde_json::{json, Value};
use std::env;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "mcp") {
        if let Err(error) = run_mcp(&args) {
            report_error(error);
            std::process::exit(1);
        }
        return;
    }
    match run_cli(&args) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| "null".to_string())
            );
        }
        Err(error) => {
            report_error(error);
            std::process::exit(1);
        }
    }
}

fn run_mcp(args: &[String]) -> Result<(), CoreError> {
    if !args.iter().any(|arg| arg == "serve") || !args.iter().any(|arg| arg == "--stdio") {
        return Err(CoreError::InvalidInput {
            field: "mcp".to_string(),
            message: "expected `mcp serve --stdio`".to_string(),
        });
    }
    let client = client_from_args(args)?;
    mcp::serve_stdio(&client).map_err(|error| CoreError::Provider {
        provider: "mcp".to_string(),
        code: "stdio".to_string(),
        message: error.to_string(),
        retry_at: None,
    })
}

fn run_cli(args: &[String]) -> Result<Value, CoreError> {
    let client = client_from_args(args)?;
    if let Some(raw) = flag_value(args, "--params") {
        let method = method_from_command(args)?;
        let params =
            serde_json::from_str::<Value>(&raw).map_err(|error| CoreError::InvalidInput {
                field: "--params".to_string(),
                message: error.to_string(),
            })?;
        return client.call(&method, params);
    }
    let method = method_from_command(args)?;
    let params = match method.as_str() {
        "plugins.list" => json!({}),
        "plugins.confluence.search" => json!({"id":required_flag(args,"--id")?,"query":required_flag(args,"--query")?}),
        "plugins.get" | "plugins.remove" | "plugins.refresh" => json!({"id": required_flag(args, "--id")?}),
        "plugins.add" => {
            let provider = required_flag(args, "--provider")?;
            let auth = flag_value(args, "--auth").unwrap_or_else(|| match provider.as_str() {
                "obsidian" => "local", "github" => "gh_cli", _ => "token",
            }.into());
            json!({"id":required_flag(args,"--id")?, "label":required_flag(args,"--label")?,
                "scope":required_flag(args,"--scope")?, "provider":provider, "auth":auth,
                "account":flag_value(args,"--account").unwrap_or_default(),
                "project":flag_value(args,"--project").unwrap_or_default(),
                "jira_scope":flag_value(args,"--jira-scope").unwrap_or_else(|| if flag_value(args,"--project").is_some() {"project"} else {"mine"}.into()),
                "jira_include_reporter":bool_flag(args,"--jira-include-reporter")?.unwrap_or(false),
                "jira_include_parents":bool_flag(args,"--jira-include-parents")?.unwrap_or(false),
                "confluence_mode":flag_value(args,"--confluence-mode").unwrap_or_else(|| "authored".into()),
                "confluence_page_ids":flag_value(args,"--confluence-page-ids").map(|s| s.split(',').map(str::to_string).collect::<Vec<_>>()).unwrap_or_default(),
                "jira_enabled":bool_flag(args,"--jira-enabled")?.unwrap_or(provider == "atlassian"),
                "confluence_enabled":bool_flag(args,"--confluence-enabled")?.unwrap_or(provider == "atlassian"),
                "auto_sync":bool_flag(args,"--auto-sync")?.unwrap_or(true)})
        }
        "plugins.update" => {
            let mut changes = serde_json::Map::new();
            for key in ["label", "scope", "account", "project", "auth"] {
                if let Some(value) = flag_value(args, &format!("--{key}")) { changes.insert(key.into(), json!(value)); }
            }
            if let Some(value) = bool_flag(args, "--auto-sync")? { changes.insert("auto_sync".into(), json!(value)); }
            for key in ["jira_scope", "confluence_mode"] {
                if let Some(value) = flag_value(args, &format!("--{}", key.replace('_', "-"))) { changes.insert(key.into(), json!(value)); }
            }
            for key in ["jira_include_reporter", "jira_include_parents", "jira_enabled", "confluence_enabled"] {
                if let Some(value) = bool_flag(args, &format!("--{}", key.replace('_', "-")))? { changes.insert(key.into(), json!(value)); }
            }
            if let Some(value) = flag_value(args, "--confluence-page-ids") {
                let ids: Vec<_> = value.split(',').filter(|v| !v.is_empty()).collect();
                changes.insert("confluence_page_ids".into(), json!(ids));
            }
            if changes.is_empty() { return Err(usage("plugins update requires changed fields or --params JSON")); }
            json!({"id":required_flag(args,"--id")?,"changes":changes})
        }
        "context.search" => json!({
            "query": required_flag(args, "--query")?,
            "provider": flag_value(args, "--provider"),
            "kind": flag_value(args, "--kind"),
            "limit": flag_value(args, "--limit").map(|value| value.parse::<usize>()).transpose().map_err(|_| CoreError::InvalidInput { field: "--limit".to_string(), message: "must be an integer".to_string() })?
        }),
        "context.get" => json!({
            "source_id": flag_value(args, "--source-id"),
            "max_age_seconds": flag_value(args, "--max-age-seconds").map(|value| value.parse::<i64>()).transpose().map_err(|_| CoreError::InvalidInput { field: "--max-age-seconds".to_string(), message: "must be an integer".to_string() })?
        }),
        "context.query.v1" => json!({
            "query": required_flag(args, "--query")?,
            "source_id": flag_value(args, "--source-id"),
            "provider": flag_value(args, "--provider"),
            "kind": flag_value(args, "--kind"),
            "max_age_seconds": flag_value(args, "--max-age-seconds").map(|value| value.parse::<i64>()).transpose().map_err(|_| CoreError::InvalidInput { field: "--max-age-seconds".to_string(), message: "must be an integer".to_string() })?,
            "max_depth": flag_value(args, "--max-depth").map(|value| value.parse::<usize>()).transpose().map_err(|_| CoreError::InvalidInput { field: "--max-depth".to_string(), message: "must be an integer".to_string() })?,
            "max_nodes": flag_value(args, "--max-nodes").map(|value| value.parse::<usize>()).transpose().map_err(|_| CoreError::InvalidInput { field: "--max-nodes".to_string(), message: "must be an integer".to_string() })?,
            "max_edges": flag_value(args, "--max-edges").map(|value| value.parse::<usize>()).transpose().map_err(|_| CoreError::InvalidInput { field: "--max-edges".to_string(), message: "must be an integer".to_string() })?,
            "max_sources": flag_value(args, "--max-sources").map(|value| value.parse::<usize>()).transpose().map_err(|_| CoreError::InvalidInput { field: "--max-sources".to_string(), message: "must be an integer".to_string() })?,
            "max_memories": flag_value(args, "--max-memories").map(|value| value.parse::<usize>()).transpose().map_err(|_| CoreError::InvalidInput { field: "--max-memories".to_string(), message: "must be an integer".to_string() })?
        }),
        "sources.refresh" => json!({"fixture_path": required_flag(args, "--fixture")?}),
        "connections.status" => json!({"connection_id": required_flag(args, "--connection-id")?}),
        "observation.get" | "candidate.get" => {
            json!({"id": required_flag(args, "--id")?})
        }
        "candidate.list" => json!({"state": flag_value(args, "--state")}),
        "memory.upsert"
        | "memory.retract"
        | "observation.capture"
        | "candidate.distill"
        | "candidate.propose" => {
            return Err(CoreError::InvalidInput {
                field: "--params".to_string(),
                message: "mutations require a JSON --params object so evidence and version guards are explicit".to_string(),
            })
        }
        _ => Value::Null,
    };
    client.call(&method, params)
}

fn method_from_command(args: &[String]) -> Result<String, CoreError> {
    let method =
        match args.first().map(String::as_str) {
            Some("plugins") => match args.get(1).map(String::as_str) {
                Some("list") => "plugins.list",
                Some("get") => "plugins.get",
                Some("add") => "plugins.add",
                Some("update") => "plugins.update",
                Some("remove") => "plugins.remove",
                Some("refresh") => "plugins.refresh",
                Some("search") => "plugins.confluence.search",
                _ => return Err(usage("plugins list|get|add|update|remove|refresh|search")),
            },
            Some("context") => match args.get(1).map(String::as_str) {
                Some("search") => "context.search",
                Some("get") => "context.get",
                Some("query") => "context.query.v1",
                _ => return Err(usage("context search|get|query")),
            },
            Some("dashboard") if args.get(1).is_some_and(|v|v=="get") => "dashboard.get",
            Some("work-link") => match args.get(1).map(String::as_str) {
                Some("add") => "work_link.add", Some("remove") => "work_link.remove", Some("list") => "work_link.list",
                _ => return Err(usage("work-link add|remove|list --params JSON")),
            },
            Some("workflow") => match args.get(1).map(String::as_str) {
                Some("activity") => "workflow.activity.list",
                Some("import-preview") => "workflow.import.preview",
                Some("import-apply") => "workflow.import.apply",
                Some("save") => "workflow.save", Some("get") => "workflow.get", Some("list") => "workflow.list",
                _ => return Err(usage("workflow save|get|list|activity|import-preview|import-apply --params JSON")),
            },
            Some("memory") => match args.get(1).map(String::as_str) {
                Some("upsert") => "memory.upsert",
                Some("retract") => "memory.retract",
                _ => return Err(usage("memory upsert|retract")),
            },
            Some("candidate") if args.get(1).is_some_and(|arg| arg == "propose") => {
                "candidate.propose"
            }
            Some("observation") => match args.get(1).map(String::as_str) {
                Some("capture") => "observation.capture",
                Some("get") => "observation.get",
                _ => return Err(usage("observation capture|get")),
            },
            Some("candidate") => match args.get(1).map(String::as_str) {
                Some("distill") => "candidate.distill",
                Some("get") => "candidate.get",
                Some("list") => "candidate.list",
                _ => return Err(usage("candidate distill|get|list|propose")),
            },
            Some("sources") if args.get(1).is_some_and(|arg| arg == "refresh") => "sources.refresh",
            Some("connections") if args.get(1).is_some_and(|arg| arg == "status") => {
                "connections.status"
            }
            _ => return Err(usage(
                "context search|get|query, memory upsert|retract, observation capture|get, candidate distill|get|list|propose, sources refresh, connections status, or plugins list|get|add|update|remove|refresh|search",
            )),
        };
    Ok(method.to_string())
}

fn client_from_args(args: &[String]) -> Result<CoreClient, CoreError> {
    if let Some(data_dir) = flag_value(args, "--data-dir") {
        if flag_value(args, "--socket").is_some() || flag_value(args, "--token-file").is_some() {
            return Err(usage("use --data-dir or --socket/--token-file, not both"));
        }
        return CoreClient::from_endpoint(CoreEndpoint::in_data_dir(data_dir));
    }
    let endpoint = match (
        flag_value(args, "--socket"),
        flag_value(args, "--token-file"),
    ) {
        (Some(socket), Some(token_file)) => CoreEndpoint {
            socket_path: socket.into(),
            token_path: token_file.into(),
        },
        (None, None) => CoreEndpoint::from_environment()?,
        _ => {
            return Err(CoreError::InvalidInput {
                field: "ipc".to_string(),
                message: "--socket and --token-file must be supplied together".to_string(),
            })
        }
    };
    CoreClient::from_endpoint(endpoint)
}

fn bool_flag(args: &[String], flag: &str) -> Result<Option<bool>, CoreError> {
    match flag_value(args, flag).as_deref() {
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        None if !args.iter().any(|arg| arg == flag) => Ok(None),
        _ => Err(usage(&format!("{flag} must be true or false"))),
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

fn required_flag(args: &[String], flag: &str) -> Result<String, CoreError> {
    flag_value(args, flag).ok_or_else(|| usage(&format!("{flag} is required")))
}

fn usage(message: &str) -> CoreError {
    CoreError::InvalidInput {
        field: "command".to_string(),
        message: message.to_string(),
    }
}

fn report_error(error: CoreError) {
    let value = json!({
        "code": error_code(&error),
        "message": error.to_string(),
        "details": serde_json::to_value(&error).ok(),
    });
    eprintln!(
        "{}",
        serde_json::to_string(&value).unwrap_or_else(|_| "{\"code\":\"error\"}".to_string())
    );
}

fn error_code(error: &CoreError) -> String {
    match error {
        CoreError::InvalidInput { .. } => "invalid_input".to_string(),
        CoreError::NotFound { .. } => "not_found".to_string(),
        CoreError::VersionConflict { .. } => "version_conflict".to_string(),
        CoreError::IdempotencyConflict { .. } => "idempotency_conflict".to_string(),
        CoreError::PermissionDenied { .. } => "permission_denied".to_string(),
        CoreError::SensitiveDataRejected => "sensitive_data_rejected".to_string(),
        CoreError::Migration { .. } => "migration".to_string(),
        CoreError::Database { .. } => "database".to_string(),
        CoreError::Connector { .. } => "connector".to_string(),
        CoreError::Provider { code, .. } => code.clone(),
    }
}
