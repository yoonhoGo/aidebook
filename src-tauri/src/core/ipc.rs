//! Authenticated local IPC for the single Core owner.
//!
//! Tauri owns one `Core` and one `CoreServer` in the desktop process.  CLI and
//! MCP are clients of this line-delimited Unix-socket protocol; they never
//! open the SQLite file.  The socket, lock, and token files are created with
//! user-only permissions in the app data directory.

use super::fixtures::FixtureAdapter;
use super::types::{
    ContextRequest, CoreError, CoreResult, MemoryRetractInput, MemoryUpsertInput, SearchRequest,
};
use super::Core;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

pub const IPC_METHODS: [&str; 6] = [
    "context.search",
    "context.get",
    "memory.upsert",
    "memory.retract",
    "sources.refresh",
    "connections.status",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreEndpoint {
    pub socket_path: PathBuf,
    pub token_path: PathBuf,
}

impl CoreEndpoint {
    pub fn in_data_dir(data_dir: impl AsRef<Path>) -> Self {
        let data_dir = data_dir.as_ref();
        Self {
            socket_path: data_dir.join("aidebook-core.sock"),
            token_path: data_dir.join("aidebook-core.token"),
        }
    }

    pub fn from_environment() -> CoreResult<Self> {
        let socket =
            std::env::var_os("AIDEBOOK_IPC_SOCKET").ok_or_else(|| CoreError::InvalidInput {
                field: "AIDEBOOK_IPC_SOCKET".to_string(),
                message: "the Core owner socket path is required".to_string(),
            })?;
        let token =
            std::env::var_os("AIDEBOOK_IPC_TOKEN_FILE").ok_or_else(|| CoreError::InvalidInput {
                field: "AIDEBOOK_IPC_TOKEN_FILE".to_string(),
                message: "the Core owner token path is required".to_string(),
            })?;
        Ok(Self {
            socket_path: socket.into(),
            token_path: token.into(),
        })
    }

    pub fn read_token(&self) -> CoreResult<String> {
        let token = fs::read_to_string(&self.token_path).map_err(|error| CoreError::Provider {
            provider: "ipc".to_string(),
            code: "owner_unavailable".to_string(),
            message: format!("could not read the Core token file: {error}"),
            retry_at: None,
        })?;
        if token.trim().is_empty() {
            return Err(CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_unavailable".to_string(),
                message: "the Core token file is empty".to_string(),
                retry_at: None,
            });
        }
        Ok(token.trim().to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcRequest {
    pub id: String,
    pub token: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<Value>,
}

pub struct CoreServer {
    core: Core,
    endpoint: CoreEndpoint,
    listener: UnixListener,
    lock_file: Option<File>,
}

impl CoreServer {
    pub fn bind(endpoint: CoreEndpoint, core: Core, token: Option<String>) -> CoreResult<Self> {
        if endpoint.socket_path.as_os_str().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "socket_path".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        if let Some(parent) = endpoint.socket_path.parent() {
            fs::create_dir_all(parent).map_err(|error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_unavailable".to_string(),
                message: format!("could not create the Core socket directory: {error}"),
                retry_at: None,
            })?;
        }
        let lock_path = endpoint.socket_path.with_extension("lock");
        let lock_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_exists".to_string(),
                message: format!("another Core owner holds the local lock: {error}"),
                retry_at: None,
            })?;
        set_user_only(&lock_path)?;

        if endpoint.socket_path.exists() {
            // A live owner must never be replaced.  A stale socket is safe to
            // remove only after the exclusive lock was acquired.
            if UnixStream::connect(&endpoint.socket_path).is_ok() {
                return Err(CoreError::Provider {
                    provider: "ipc".to_string(),
                    code: "owner_exists".to_string(),
                    message: "another Core owner is listening on the socket".to_string(),
                    retry_at: None,
                });
            }
            fs::remove_file(&endpoint.socket_path).map_err(|error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_unavailable".to_string(),
                message: format!("could not remove a stale Core socket: {error}"),
                retry_at: None,
            })?;
        }

        let token = token.unwrap_or_else(|| Uuid::new_v4().to_string());
        if token.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "token".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        fs::write(&endpoint.token_path, format!("{token}\n")).map_err(|error| {
            CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_unavailable".to_string(),
                message: format!("could not write the Core token file: {error}"),
                retry_at: None,
            }
        })?;
        set_user_only(&endpoint.token_path)?;
        let listener =
            UnixListener::bind(&endpoint.socket_path).map_err(|error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_unavailable".to_string(),
                message: format!("could not bind the Core socket: {error}"),
                retry_at: None,
            })?;
        set_user_only(&endpoint.socket_path)?;
        Ok(Self {
            core,
            endpoint,
            listener,
            lock_file: Some(lock_file),
        })
    }

    pub fn endpoint(&self) -> &CoreEndpoint {
        &self.endpoint
    }

    pub fn serve(self) -> CoreResult<()> {
        self.serve_until(Arc::new(AtomicBool::new(false)))
    }

    pub fn serve_until(self, stop: Arc<AtomicBool>) -> CoreResult<()> {
        self.listener
            .set_nonblocking(true)
            .map_err(|error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_unavailable".to_string(),
                message: format!("could not configure the Core socket: {error}"),
                retry_at: None,
            })?;
        while !stop.load(Ordering::Relaxed) {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let core = self.core.clone();
                    let endpoint = self.endpoint.clone();
                    thread::spawn(move || handle_connection(stream, &endpoint, &core));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(CoreError::Provider {
                        provider: "ipc".to_string(),
                        code: "owner_unavailable".to_string(),
                        message: format!("Core socket accept failed: {error}"),
                        retry_at: None,
                    });
                }
            }
        }
        Ok(())
    }
}

impl Drop for CoreServer {
    fn drop(&mut self) {
        let lock_path = self.endpoint.socket_path.with_extension("lock");
        let _ = fs::remove_file(&self.endpoint.socket_path);
        let _ = fs::remove_file(&self.endpoint.token_path);
        let _ = fs::remove_file(lock_path);
        let _ = self.lock_file.take();
    }
}

#[derive(Debug, Clone)]
pub struct CoreClient {
    endpoint: CoreEndpoint,
    token: String,
}

impl CoreClient {
    pub fn from_endpoint(endpoint: CoreEndpoint) -> CoreResult<Self> {
        let token = endpoint.read_token()?;
        Ok(Self { endpoint, token })
    }

    pub fn with_token(endpoint: CoreEndpoint, token: impl Into<String>) -> CoreResult<Self> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "token".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        Ok(Self { endpoint, token })
    }

    pub fn endpoint(&self) -> &CoreEndpoint {
        &self.endpoint
    }

    pub fn call(&self, method: &str, params: Value) -> CoreResult<Value> {
        if !IPC_METHODS.contains(&method) {
            return Err(CoreError::InvalidInput {
                field: "method".to_string(),
                message: format!("unsupported IPC method '{method}'"),
            });
        }
        let mut stream = UnixStream::connect(&self.endpoint.socket_path).map_err(|error| {
            CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_unavailable".to_string(),
                message: format!("could not connect to the Core owner: {error}"),
                retry_at: None,
            }
        })?;
        let request = IpcRequest {
            id: Uuid::new_v4().to_string(),
            token: self.token.clone(),
            method: method.to_string(),
            params,
        };
        let encoded = serde_json::to_string(&request).map_err(|error| CoreError::Database {
            message: error.to_string(),
        })?;
        writeln!(stream, "{encoded}").map_err(|error| CoreError::Provider {
            provider: "ipc".to_string(),
            code: "owner_unavailable".to_string(),
            message: format!("could not write to the Core owner: {error}"),
            retry_at: None,
        })?;
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .map_err(|error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_unavailable".to_string(),
                message: format!("Core owner did not respond: {error}"),
                retry_at: None,
            })?;
        let response: IpcResponse =
            serde_json::from_str(&line).map_err(|error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "malformed_response".to_string(),
                message: format!("Core owner response was malformed: {error}"),
                retry_at: None,
            })?;
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (_, Some(error)) => Err(core_error_from_ipc(error)),
            _ => Err(CoreError::Provider {
                provider: "ipc".to_string(),
                code: "malformed_response".to_string(),
                message: "Core owner returned neither result nor error".to_string(),
                retry_at: None,
            }),
        }
    }
}

pub fn dispatch(core: &Core, method: &str, params: Value) -> CoreResult<Value> {
    match method {
        "context.search" => Ok(serde_json::to_value(core.context_search(
            serde_json::from_value::<SearchRequest>(params).map_err(invalid_params)?,
        )?)
        .map_err(serialize_error)?),
        "context.get" => Ok(serde_json::to_value(core.context_get(
            serde_json::from_value::<ContextRequest>(params).map_err(invalid_params)?,
        )?)
        .map_err(serialize_error)?),
        "memory.upsert" => Ok(serde_json::to_value(core.memory_upsert(
            serde_json::from_value::<MemoryUpsertInput>(params).map_err(invalid_params)?,
        )?)
        .map_err(serialize_error)?),
        "memory.retract" => Ok(serde_json::to_value(core.memory_retract(
            serde_json::from_value::<MemoryRetractInput>(params).map_err(invalid_params)?,
        )?)
        .map_err(serialize_error)?),
        "sources.refresh" => {
            let fixture_path = params
                .get("fixture_path")
                .and_then(Value::as_str)
                .ok_or_else(|| CoreError::InvalidInput {
                    field: "fixture_path".to_string(),
                    message: "an explicit fixture path is required for an IPC refresh".to_string(),
                })?;
            let fixture = FixtureAdapter::from_path(fixture_path)?;
            Ok(serde_json::to_value(core.sources_refresh(&fixture)?).map_err(serialize_error)?)
        }
        "connections.status" => {
            let connection_id = params
                .get("connection_id")
                .and_then(Value::as_str)
                .ok_or_else(|| CoreError::InvalidInput {
                    field: "connection_id".to_string(),
                    message: "is required".to_string(),
                })?;
            Ok(
                serde_json::to_value(core.connections_status(connection_id)?)
                    .map_err(serialize_error)?,
            )
        }
        other => Err(CoreError::InvalidInput {
            field: "method".to_string(),
            message: format!("unsupported IPC method '{other}'"),
        }),
    }
}

fn handle_connection(mut stream: UnixStream, endpoint: &CoreEndpoint, core: &Core) {
    let mut line = String::new();
    let response = match BufReader::new(&mut stream).read_line(&mut line) {
        Ok(_) => match serde_json::from_str::<IpcRequest>(&line) {
            Ok(request) => authenticate_and_dispatch(endpoint, core, request),
            Err(error) => error_response(
                "invalid_request",
                format!("invalid IPC request: {error}"),
                None,
            ),
        },
        Err(error) => error_response(
            "invalid_request",
            format!("could not read IPC request: {error}"),
            None,
        ),
    };
    if let Ok(encoded) = serde_json::to_string(&response) {
        let _ = writeln!(stream, "{encoded}");
    }
}

fn authenticate_and_dispatch(
    endpoint: &CoreEndpoint,
    core: &Core,
    request: IpcRequest,
) -> IpcResponse {
    let token = match endpoint.read_token() {
        Ok(token) => token,
        Err(error) => return error_response("owner_unavailable", error.to_string(), Some(error)),
    };
    if !constant_time_equal(&token, &request.token) {
        return error_response(
            "unauthenticated",
            "the local Core credential was rejected".to_string(),
            None,
        );
    }
    match dispatch(core, &request.method, request.params) {
        Ok(result) => IpcResponse {
            id: request.id,
            result: Some(result),
            error: None,
        },
        Err(error) => error_response_for_id(request.id, error),
    }
}

fn error_response(code: &str, message: String, error: Option<CoreError>) -> IpcResponse {
    IpcResponse {
        id: String::new(),
        result: None,
        error: Some(IpcError {
            code: code.to_string(),
            message,
            details: error.and_then(|error| serde_json::to_value(error).ok()),
        }),
    }
}

fn error_response_for_id(id: String, error: CoreError) -> IpcResponse {
    let code = match &error {
        CoreError::InvalidInput { .. } => "invalid_input",
        CoreError::NotFound { .. } => "not_found",
        CoreError::VersionConflict { .. } => "version_conflict",
        CoreError::IdempotencyConflict { .. } => "idempotency_conflict",
        CoreError::PermissionDenied { .. } => "permission_denied",
        CoreError::SensitiveDataRejected => "sensitive_data_rejected",
        CoreError::Migration { .. } => "migration",
        CoreError::Database { .. } => "database",
        CoreError::Connector { .. } => "connector",
        CoreError::Provider { code, .. } => code.as_str(),
    };
    IpcResponse {
        id,
        result: None,
        error: Some(IpcError {
            code: code.to_string(),
            message: error.to_string(),
            details: serde_json::to_value(error).ok(),
        }),
    }
}

fn core_error_from_ipc(error: IpcError) -> CoreError {
    if let Some(details) = error.details {
        if let Ok(core_error) = serde_json::from_value::<CoreError>(details) {
            return core_error;
        }
    }
    CoreError::Provider {
        provider: "ipc".to_string(),
        code: error.code,
        message: error.message,
        retry_at: None,
    }
}

fn invalid_params(error: serde_json::Error) -> CoreError {
    CoreError::InvalidInput {
        field: "params".to_string(),
        message: error.to_string(),
    }
}

fn serialize_error(error: serde_json::Error) -> CoreError {
    CoreError::Database {
        message: error.to_string(),
    }
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    for (a, b) in left.bytes().zip(right.bytes()) {
        difference |= usize::from(a ^ b);
    }
    difference == 0
}

fn set_user_only(path: &Path) -> CoreResult<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        CoreError::Provider {
            provider: "ipc".to_string(),
            code: "owner_unavailable".to_string(),
            message: format!("could not restrict local IPC permissions: {error}"),
            retry_at: None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn authenticated_client_uses_one_owner_and_rejects_wrong_token() {
        let directory = PathBuf::from(format!("/tmp/aidebook-ipc-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let endpoint = CoreEndpoint::in_data_dir(&directory);
        let core = Core::in_memory().unwrap();
        let server =
            CoreServer::bind(endpoint.clone(), core, Some("test-secret".to_string())).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = stop.clone();
        let thread = std::thread::spawn(move || server.serve_until(stop_for_thread));
        let client = CoreClient::with_token(endpoint.clone(), "test-secret").unwrap();
        let status = client
            .call(
                "connections.status",
                serde_json::json!({"connection_id":"missing"}),
            )
            .expect_err("missing connection should be a structured core error");
        assert!(matches!(status, CoreError::NotFound { .. }));
        let wrong = CoreClient::with_token(endpoint.clone(), "wrong").unwrap();
        let error = wrong
            .call(
                "connections.status",
                serde_json::json!({"connection_id":"missing"}),
            )
            .expect_err("wrong token must be rejected");
        assert!(matches!(error, CoreError::Provider { code, .. } if code == "unauthenticated"));
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap().unwrap();
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn dispatch_exposes_exactly_six_methods_and_no_unknown_method() {
        assert_eq!(IPC_METHODS.len(), 6);
        let core = Core::in_memory().unwrap();
        let error = dispatch(&core, "database.open", Value::Null).expect_err("unknown method");
        assert!(matches!(error, CoreError::InvalidInput { field, .. } if field == "method"));
    }
}
