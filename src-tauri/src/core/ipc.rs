//! Authenticated local IPC for the single Core owner.
//!
//! Tauri owns one `Core` and one `CoreServer` in the desktop process.  CLI and
//! MCP are clients of this line-delimited Unix-socket protocol; they never
//! open the SQLite file.  The socket, lock, and token files are created with
//! user-only permissions in the app data directory.

use super::fixtures::FixtureAdapter;
use super::types::{
    CandidateDistillInput, CandidateProposeInput, CandidateState, ContextQueryRequest,
    ContextRequest, CoreError, CoreResult, MemoryRetractInput, MemoryUpsertInput,
    ObservationCaptureInput, SearchRequest,
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

pub const LEGACY_IPC_METHODS: [&str; 6] = [
    "context.search",
    "context.get",
    "memory.upsert",
    "memory.retract",
    "sources.refresh",
    "connections.status",
];

pub const IPC_METHODS: [&str; 20] = [
    "context.search",
    "context.get",
    "memory.upsert",
    "memory.retract",
    "sources.refresh",
    "connections.status",
    "context.query.v1",
    "observation.capture",
    "observation.get",
    "candidate.distill",
    "candidate.propose",
    "candidate.get",
    "candidate.list",
    "plugins.list",
    "plugins.get",
    "plugins.add",
    "plugins.update",
    "plugins.remove",
    "plugins.refresh",
    "plugins.confluence.search",
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
    plugins: Option<Arc<super::local_sync::LocalSync>>,
    endpoint: CoreEndpoint,
    listener: UnixListener,
    lock_file: Option<File>,
}

/// Keeps the single-owner lock alive while bind is in progress and removes
/// it when bind fails before a `CoreServer` can take ownership of cleanup.
struct OwnerLock {
    path: PathBuf,
    file: Option<File>,
    armed: bool,
}

impl OwnerLock {
    fn disarm(mut self) -> File {
        let file = self
            .file
            .take()
            .expect("owner lock file must exist until CoreServer takes ownership");
        self.armed = false;
        file
    }
}

impl Drop for OwnerLock {
    fn drop(&mut self) {
        let _ = self.file.take();
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
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
        let lock_file = open_owner_lock(&lock_path)?;
        let mut owner_lock = OwnerLock {
            path: lock_path,
            file: Some(lock_file),
            armed: true,
        };
        if let Some(lock_file) = owner_lock.file.as_mut() {
            lock_file
                .write_all(std::process::id().to_string().as_bytes())
                .and_then(|_| lock_file.sync_all())
                .map_err(|error| CoreError::Provider {
                    provider: "ipc".to_string(),
                    code: "owner_unavailable".to_string(),
                    message: format!("could not write the Core owner lock: {error}"),
                    retry_at: None,
                })?;
        }
        set_user_only(&owner_lock.path)?;

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
            plugins: None,
            endpoint,
            listener,
            lock_file: Some(owner_lock.disarm()),
        })
    }

    /// Share the desktop registry and scan lock; never open a second settings owner.
    pub fn with_plugins(mut self, plugins: Arc<super::local_sync::LocalSync>) -> Self {
        self.plugins = Some(plugins);
        self
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
                    let plugins = self.plugins.clone();
                    thread::spawn(move || {
                        handle_connection(stream, &endpoint, &core, plugins.as_deref())
                    });
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
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(
                if method.starts_with("plugins.") {
                    120
                } else {
                    15
                },
            )))
            .map_err(|_| CoreError::Connector {
                message: "IPC read timeout configuration failed".into(),
            })?;
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(15)))
            .map_err(|_| CoreError::Connector {
                message: "IPC write timeout configuration failed".into(),
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
        "context.query.v1" => Ok(serde_json::to_value(core.context_query(
            serde_json::from_value::<ContextQueryRequest>(params).map_err(invalid_params)?,
        )?)
        .map_err(serialize_error)?),
        "observation.capture" => Ok(serde_json::to_value(core.observation_capture(
            serde_json::from_value::<ObservationCaptureInput>(params).map_err(invalid_params)?,
        )?)
        .map_err(serialize_error)?),
        "observation.get" => {
            let id = required_string_param(&params, "id")?;
            Ok(serde_json::to_value(core.observation(&id)?).map_err(serialize_error)?)
        }
        "candidate.distill" => Ok(serde_json::to_value(core.candidate_distill(
            serde_json::from_value::<CandidateDistillInput>(params).map_err(invalid_params)?,
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
        "candidate.propose" => Ok(serde_json::to_value(core.candidate_propose(
            serde_json::from_value::<CandidateProposeInput>(params).map_err(invalid_params)?,
        )?)
        .map_err(serialize_error)?),
        "candidate.get" => {
            let id = required_string_param(&params, "id")?;
            Ok(serde_json::to_value(core.candidate(&id)?).map_err(serialize_error)?)
        }
        "candidate.list" => {
            let state = match params.get("state").cloned() {
                None | Some(Value::Null) => None,
                Some(value) => {
                    Some(serde_json::from_value::<CandidateState>(value).map_err(invalid_params)?)
                }
            };
            Ok(serde_json::to_value(core.candidates(state)).map_err(serialize_error)?)
        }
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

fn dispatch_plugins(
    plugins: &super::local_sync::LocalSync,
    method: &str,
    params: Value,
) -> CoreResult<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Id {
        id: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Update {
        id: String,
        changes: super::plugins::ConnectionPatch,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    match method {
        "plugins.confluence.search" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Search {
                id: String,
                query: String,
            }
            let input: Search = serde_json::from_value(params).map_err(invalid_params)?;
            let connection = plugins.connection(&input.id)?;
            serde_json::to_value(super::confluence::search(&connection, &input.query)?)
                .map_err(serialize_error)
        }
        "plugins.list" => {
            let _: Empty = serde_json::from_value(params).map_err(invalid_params)?;
            serde_json::to_value(plugins.connections()?).map_err(serialize_error)
        }
        "plugins.add" => {
            let input = serde_json::from_value(params).map_err(invalid_params)?;
            serde_json::to_value(plugins.add(input)?).map_err(serialize_error)
        }
        "plugins.update" => {
            let input: Update = serde_json::from_value(params).map_err(invalid_params)?;
            serde_json::to_value(plugins.update(&input.id, input.changes)?).map_err(serialize_error)
        }
        "plugins.get" | "plugins.remove" | "plugins.refresh" => {
            let input: Id = serde_json::from_value(params).map_err(invalid_params)?;
            match method {
                "plugins.get" => {
                    serde_json::to_value(plugins.connection(&input.id)?).map_err(serialize_error)
                }
                "plugins.refresh" => serde_json::to_value(plugins.refresh(&input.id, false)?)
                    .map_err(serialize_error),
                _ => {
                    plugins.remove(&input.id)?;
                    Ok(
                        serde_json::json!({"id":input.id,"removed":true,"cache_preserved":true,"credentials_preserved":true}),
                    )
                }
            }
        }
        _ => Err(CoreError::InvalidInput {
            field: "method".into(),
            message: "unsupported plugin method".into(),
        }),
    }
}

fn handle_connection(
    mut stream: UnixStream,
    endpoint: &CoreEndpoint,
    core: &Core,
    plugins: Option<&super::local_sync::LocalSync>,
) {
    let mut line = String::new();
    let response = match BufReader::new(&mut stream).read_line(&mut line) {
        Ok(_) => match serde_json::from_str::<IpcRequest>(&line) {
            Ok(request) => authenticate_and_dispatch(endpoint, core, request, plugins),
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
    plugins: Option<&super::local_sync::LocalSync>,
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
    let result = if request.method.starts_with("plugins.") {
        plugins
            .ok_or_else(|| CoreError::Provider {
                provider: "plugins".into(),
                code: "unavailable".into(),
                message:
                    "this Core owner has no connection registry; restart the updated desktop app"
                        .into(),
                retry_at: None,
            })
            .and_then(|plugins| dispatch_plugins(plugins, &request.method, request.params))
    } else {
        dispatch(core, &request.method, request.params)
    };
    match result {
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

fn required_string_param(params: &Value, field: &str) -> CoreResult<String> {
    params
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| CoreError::InvalidInput {
            field: field.to_string(),
            message: "is required".to_string(),
        })
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    for (a, b) in left.bytes().zip(right.bytes()) {
        difference |= usize::from(a ^ b);
    }
    difference == 0
}

fn open_owner_lock(path: &Path) -> CoreResult<File> {
    let open = || OpenOptions::new().write(true).create_new(true).open(path);
    match open() {
        Ok(file) => Ok(file),
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists && !owner_lock_is_live(path) =>
        {
            fs::remove_file(path).map_err(|remove_error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_exists".to_string(),
                message: format!("stale Core lock could not be removed: {remove_error}"),
                retry_at: None,
            })?;
            open().map_err(|retry_error| CoreError::Provider {
                provider: "ipc".to_string(),
                code: "owner_exists".to_string(),
                message: format!("another Core owner holds the local lock: {retry_error}"),
                retry_at: None,
            })
        }
        Err(error) => Err(CoreError::Provider {
            provider: "ipc".to_string(),
            code: "owner_exists".to_string(),
            message: format!("another Core owner holds the local lock: {error}"),
            retry_at: None,
        }),
    }
}

fn owner_lock_is_live(path: &Path) -> bool {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return true,
    };
    let Ok(pid) = contents.trim().parse::<libc::pid_t>() else {
        return true;
    };
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
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
    fn dispatch_preserves_legacy_methods_and_rejects_unknown_method() {
        assert_eq!(LEGACY_IPC_METHODS.len(), 6);
        assert!(LEGACY_IPC_METHODS
            .iter()
            .all(|method| IPC_METHODS.contains(method)));
        assert!(IPC_METHODS.contains(&"context.query.v1"));
        assert!(IPC_METHODS.contains(&"candidate.propose"));
        let core = Core::in_memory().unwrap();
        let error = dispatch(&core, "database.open", Value::Null).expect_err("unknown method");
        assert!(matches!(error, CoreError::InvalidInput { field, .. } if field == "method"));
    }

    #[test]
    fn second_owner_is_rejected_and_owner_files_are_cleaned_up() {
        let directory = PathBuf::from(format!("/tmp/aidebook-ipc-owner-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let endpoint = CoreEndpoint::in_data_dir(&directory);
        let server = CoreServer::bind(endpoint.clone(), Core::in_memory().unwrap(), None).unwrap();
        let second = CoreServer::bind(endpoint.clone(), Core::in_memory().unwrap(), None);
        assert!(matches!(
            second,
            Err(CoreError::Provider { code, .. }) if code == "owner_exists"
        ));
        drop(server);
        assert!(!endpoint.socket_path.exists());
        assert!(!endpoint.token_path.exists());
        assert!(!endpoint.socket_path.with_extension("lock").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_owner_lock_is_recovered_after_an_unclean_exit() {
        let directory = PathBuf::from(format!("/tmp/aidebook-ipc-stale-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let endpoint = CoreEndpoint::in_data_dir(&directory);
        let lock_path = endpoint.socket_path.with_extension("lock");
        fs::write(&lock_path, format!("{}\n", libc::pid_t::MAX)).unwrap();
        let server = CoreServer::bind(endpoint.clone(), Core::in_memory().unwrap(), None)
            .expect("stale owner lock should be recoverable");
        drop(server);
        fs::remove_dir_all(directory).unwrap();
    }
}
