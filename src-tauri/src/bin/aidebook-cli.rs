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
        "sources.refresh" => json!({"fixture_path": required_flag(args, "--fixture")?}),
        "connections.status" => json!({"connection_id": required_flag(args, "--connection-id")?}),
        "memory.upsert" | "memory.retract" => {
            return Err(CoreError::InvalidInput {
                field: "--params".to_string(),
                message: "memory mutations require a JSON --params object so evidence and version guards are explicit".to_string(),
            })
        }
        _ => Value::Null,
    };
    client.call(&method, params)
}

fn method_from_command(args: &[String]) -> Result<String, CoreError> {
    let method =
        match args.first().map(String::as_str) {
            Some("context") => match args.get(1).map(String::as_str) {
                Some("search") => "context.search",
                Some("get") => "context.get",
                _ => return Err(usage("context search|get")),
            },
            Some("memory") => match args.get(1).map(String::as_str) {
                Some("upsert") => "memory.upsert",
                Some("retract") => "memory.retract",
                _ => return Err(usage("memory upsert|retract")),
            },
            Some("sources") if args.get(1).is_some_and(|arg| arg == "refresh") => "sources.refresh",
            Some("connections") if args.get(1).is_some_and(|arg| arg == "status") => {
                "connections.status"
            }
            _ => return Err(usage(
                "context search|get, memory upsert|retract, sources refresh, or connections status",
            )),
        };
    Ok(method.to_string())
}

fn client_from_args(args: &[String]) -> Result<CoreClient, CoreError> {
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
