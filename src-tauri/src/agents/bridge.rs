use crate::core::{mcp, CoreClient, CoreEndpoint, CoreError, CoreResult, IPC_METHODS};
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
};

pub fn run_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("--aidebook-agent") {
        return None;
    }
    Some(match run(&args[1..]) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}
fn invalid(message: &str) -> CoreError {
    CoreError::InvalidInput {
        field: "agent_bridge".into(),
        message: message.into(),
    }
}
fn run(args: &[String]) -> CoreResult<()> {
    let mode = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| invalid("expected mcp, call, describe or probe"))?;
    if mode == "describe" {
        println!(
            "{}",
            json!({"bridge_version":1,"tools":mcp::tool_descriptors()})
        );
        return Ok(());
    }
    let data_dir = args
        .windows(2)
        .find(|p| p[0] == "--data-dir")
        .map(|p| PathBuf::from(&p[1]));
    let endpoint = match data_dir {
        Some(path) => CoreEndpoint::in_data_dir(path),
        None => CoreEndpoint::from_environment()?,
    };
    if mode == "mcp" {
        // Refresh IPC credentials per message, so app restart/token rotation does not require
        // reinstalling an agent. initialize/tools/list remain available while the app is closed.
        let mut stdin = io::stdin().lock();
        let mut stdout = io::stdout().lock();
        loop {
            let mut line = String::new();
            let size = (&mut stdin)
                .take(8 * 1024 * 1024 + 1)
                .read_line(&mut line)
                .map_err(|_| invalid("MCP input could not be read"))?;
            if size == 0 {
                break;
            }
            if size > 8 * 1024 * 1024 {
                return Err(invalid("MCP request is too large"));
            }
            let client = CoreClient::from_endpoint(endpoint.clone())
                .or_else(|_| CoreClient::with_token(endpoint.clone(), "app-unavailable"))?;
            if let Some(response) = mcp::handle_message(&client, &line) {
                writeln!(stdout, "{response}")
                    .and_then(|_| stdout.flush())
                    .map_err(|_| invalid("MCP output could not be written"))?;
            }
        }
        return Ok(());
    }
    let client = CoreClient::from_endpoint(endpoint)?;
    if mode == "probe" {
        client.call(
            "context.search",
            json!({"query":"aidebook-connection-probe", "limit":1}),
        )?;
        println!(
            "{}",
            json!({"connected":true,"tools":mcp::tool_descriptors().len()})
        );
        return Ok(());
    }
    if mode != "call" {
        return Err(invalid("unknown bridge mode"));
    }
    let method = args
        .get(1)
        .filter(|m| IPC_METHODS.contains(&m.as_str()))
        .ok_or_else(|| invalid("unsupported agent method"))?;
    let mut input = String::new();
    io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut input)
        .map_err(|_| invalid("tool arguments could not be read"))?;
    if input.len() > 8 * 1024 * 1024 {
        return Err(invalid("tool arguments are too large"));
    }
    let params: Value =
        serde_json::from_str(&input).map_err(|_| invalid("tool arguments must be JSON"))?;
    println!("{}", client.call(method, params)?);
    Ok(())
}
