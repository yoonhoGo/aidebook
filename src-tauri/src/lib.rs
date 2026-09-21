pub mod agents;
pub mod core;

use core::{
    CandidateAcceptInput, CandidateDistillInput, CandidateProposeInput, CandidateRejectInput,
    CandidateState, ContextQueryRequest, ContextRequest, Core, CoreEndpoint, CoreError, CoreServer,
    CredentialStore, GitHubAdapter, GitHubConfig, GraphRebuildRequest, HttpGitHubApi,
    KeychainCredentialStore, MemoryRestoreInput, MemoryRetractInput, MemoryUpsertInput,
    ObservationCaptureInput, ObsidianAdapter, ReadOnlyConnector, RelationInput, SearchRequest,
    Snapshot, SourceRef, SourcesRefreshResult, UiMemoryUpsertInput, VaultChange, VaultConfig,
    VaultScanResult, VaultWatcher,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Manager, State, WindowEvent};

struct AppState {
    agent_installer: Arc<agents::AgentInstaller>,
    plugins: Arc<Mutex<core::plugins::PluginRegistry>>,
    local_sync: Arc<core::local_sync::LocalSync>,
    core: Core,
    core_stop: Arc<AtomicBool>,
    vault: Mutex<Option<ObsidianAdapter>>,
    vault_watcher: Mutex<Option<VaultWatcher>>,
    github: Mutex<Option<GitHubAdapter<HttpGitHubApi, KeychainCredentialStore>>>,
}

#[derive(Debug, Clone, Serialize)]
struct VaultSelection {
    root_path: String,
    account_id: String,
    connection_id: String,
    provider: &'static str,
    read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
struct VaultScanResponse {
    scan: VaultScanResult,
    changes: Vec<VaultChange>,
    refresh: SourcesRefreshResult,
}

#[derive(Debug, Clone, Deserialize)]
struct VaultSelectInput {
    path: String,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct GitHubSelection {
    account_id: String,
    connection_id: String,
    scope: String,
    provider: &'static str,
    read_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubSelectInput {
    account_id: String,
    connection_id: String,
    owner: String,
    repository: String,
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    per_page: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubCredentialInput {
    connection_id: String,
    token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubDisconnectInput {
    connection_id: String,
    #[serde(default)]
    delete_credential: bool,
}

#[tauri::command]
async fn agent_connections_status(
    state: State<'_, AppState>,
) -> Result<Vec<agents::AgentStatus>, CoreError> {
    let installer = state.agent_installer.clone();
    tauri::async_runtime::spawn_blocking(move || installer.status())
        .await
        .map_err(|_| plugin_lock_error())
}
#[tauri::command]
async fn agent_connection_install(
    agent: agents::Agent,
    state: State<'_, AppState>,
) -> Result<agents::InstallResult, CoreError> {
    let installer = state.agent_installer.clone();
    tauri::async_runtime::spawn_blocking(move || installer.install(agent))
        .await
        .map_err(|_| plugin_lock_error())?
}
#[tauri::command]
async fn agent_connection_uninstall(
    agent: agents::Agent,
    state: State<'_, AppState>,
) -> Result<agents::InstallResult, CoreError> {
    let installer = state.agent_installer.clone();
    tauri::async_runtime::spawn_blocking(move || installer.uninstall(agent))
        .await
        .map_err(|_| plugin_lock_error())?
}
#[tauri::command]
async fn agent_connection_probe(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, CoreError> {
    let installer = state.agent_installer.clone();
    tauri::async_runtime::spawn_blocking(move || installer.probe())
        .await
        .map_err(|_| plugin_lock_error())?
}

#[tauri::command]
fn plugin_list(
    state: State<'_, AppState>,
) -> Result<Vec<core::plugins::PluginConnection>, CoreError> {
    Ok(state
        .plugins
        .lock()
        .map_err(|_| plugin_lock_error())?
        .list())
}
fn plugin_lock_error() -> CoreError {
    CoreError::Connector {
        message: "connection registry unavailable".into(),
    }
}
#[tauri::command]
fn plugin_add(
    input: core::plugins::PluginConnection,
    state: State<'_, AppState>,
) -> Result<core::plugins::PluginConnection, CoreError> {
    state
        .plugins
        .lock()
        .map_err(|_| plugin_lock_error())?
        .add(input)
}
#[tauri::command]
async fn plugin_remove(
    id: String,
    delete_credential: bool,
    state: State<'_, AppState>,
) -> Result<(), CoreError> {
    let registry = state.plugins.clone();
    let local_sync = state.local_sync.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = registry.lock().map_err(|_| plugin_lock_error())?.get(&id)?;
        if connection.provider == core::plugins::Provider::Obsidian {
            return local_sync.remove(&id);
        }
        let mut registry = registry.lock().map_err(|_| plugin_lock_error())?;
        if delete_credential && connection.auth == core::plugins::AuthMethod::Token {
            KeychainCredentialStore.delete(&core::plugins::credential_key(&id))?;
        }
        registry.remove(&id)
    })
    .await
    .map_err(|_| plugin_lock_error())?
}
#[tauri::command]
fn plugin_sync_status(
    state: State<'_, AppState>,
) -> Result<Vec<core::local_sync::LocalSyncStatus>, CoreError> {
    state.local_sync.statuses()
}
#[tauri::command]
async fn plugin_sync_configure(
    id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), CoreError> {
    let local_sync = state.local_sync.clone();
    tauri::async_runtime::spawn_blocking(move || local_sync.set_enabled(&id, enabled))
        .await
        .map_err(|_| plugin_lock_error())?
}
#[tauri::command]
fn plugin_token_set(
    id: String,
    token: String,
    state: State<'_, AppState>,
) -> Result<(), CoreError> {
    let connection = state
        .plugins
        .lock()
        .map_err(|_| plugin_lock_error())?
        .get(&id)?;
    core::plugins::set_token(&connection, &token)
}
#[tauri::command]
async fn plugin_refresh(
    id: String,
    state: State<'_, AppState>,
) -> Result<SourcesRefreshResult, CoreError> {
    let connection = state
        .plugins
        .lock()
        .map_err(|_| plugin_lock_error())?
        .get(&id)?;
    let core = state.core.clone();
    let local_sync = state.local_sync.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if connection.provider == core::plugins::Provider::Obsidian {
            local_sync.refresh(&id, false)
        } else {
            core::plugins::refresh(&core, connection)
        }
    })
    .await
    .map_err(|_| plugin_lock_error())?
}

#[tauri::command]
fn core_status() -> core::CoreStatus {
    core::status()
}

#[tauri::command]
fn vault_select(
    input: VaultSelectInput,
    state: State<'_, AppState>,
) -> Result<VaultSelection, CoreError> {
    if input.path.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "path".to_string(),
            message: "a vault path must be selected explicitly".to_string(),
        });
    }
    let account_id = input
        .account_id
        .unwrap_or_else(|| "selected-vault".to_string());
    let config = VaultConfig::new(&input.path, account_id.clone(), "obsidian-selected-vault");
    let adapter = ObsidianAdapter::open(config)?;
    let selection = VaultSelection {
        root_path: adapter.root_path().display().to_string(),
        account_id,
        connection_id: adapter.connection_id().to_string(),
        provider: "obsidian",
        read_only: true,
    };
    let watcher = adapter.clone().watcher()?;
    let mut vault = state.vault.lock().map_err(|_| CoreError::Database {
        message: "vault state mutex was poisoned".to_string(),
    })?;
    *vault = Some(adapter);
    let mut vault_watcher = state
        .vault_watcher
        .lock()
        .map_err(|_| CoreError::Database {
            message: "vault watcher mutex was poisoned".to_string(),
        })?;
    *vault_watcher = Some(watcher);
    Ok(selection)
}

#[tauri::command]
fn vault_scan(state: State<'_, AppState>) -> Result<VaultScanResponse, CoreError> {
    let adapter = state
        .vault
        .lock()
        .map_err(|_| CoreError::Database {
            message: "vault state mutex was poisoned".to_string(),
        })?
        .clone()
        .ok_or_else(|| CoreError::InvalidInput {
            field: "vault".to_string(),
            message: "select a vault before scanning".to_string(),
        })?;
    let scan = adapter.scan()?;
    let changes = state
        .vault_watcher
        .lock()
        .map_err(|_| CoreError::Database {
            message: "vault watcher mutex was poisoned".to_string(),
        })?
        .as_mut()
        .map(|watcher| watcher.observe(scan.clone()))
        .unwrap_or_default();
    let refresh = state.core.sources_refresh(&adapter)?;
    Ok(VaultScanResponse {
        scan,
        changes,
        refresh,
    })
}

#[tauri::command]
fn github_select(
    input: GitHubSelectInput,
    state: State<'_, AppState>,
) -> Result<GitHubSelection, CoreError> {
    let mut config = GitHubConfig::new(
        input.account_id,
        input.connection_id,
        input.owner,
        input.repository,
    );
    if let Some(api_base_url) = input.api_base_url {
        config.api_base_url = api_base_url;
    }
    if let Some(per_page) = input.per_page {
        config.per_page = per_page;
    }
    let adapter = GitHubAdapter::new(config.clone(), HttpGitHubApi, KeychainCredentialStore)?;
    let scope = config.scope();
    let selection = GitHubSelection {
        account_id: config.account_id,
        connection_id: config.connection_id,
        scope,
        provider: "github",
        read_only: true,
    };
    let mut github = state.github.lock().map_err(|_| CoreError::Database {
        message: "github state mutex was poisoned".to_string(),
    })?;
    *github = Some(adapter);
    Ok(selection)
}

#[tauri::command]
fn github_credential_set(
    input: GitHubCredentialInput,
    state: State<'_, AppState>,
) -> Result<(), CoreError> {
    let selected = state
        .github
        .lock()
        .map_err(|_| CoreError::Database {
            message: "github state mutex was poisoned".to_string(),
        })?
        .clone()
        .ok_or_else(|| CoreError::InvalidInput {
            field: "github".to_string(),
            message: "select a repository before storing its credential".to_string(),
        })?;
    if input.connection_id != selected.config().connection_id {
        return Err(CoreError::PermissionDenied {
            provider: "github".to_string(),
            message: "credential connection does not match the selected repository".to_string(),
        });
    }
    selected
        .credentials()
        .set(&input.connection_id, &input.token)
}

#[tauri::command]
fn github_refresh(state: State<'_, AppState>) -> Result<SourcesRefreshResult, CoreError> {
    let adapter = state
        .github
        .lock()
        .map_err(|_| CoreError::Database {
            message: "github state mutex was poisoned".to_string(),
        })?
        .clone()
        .ok_or_else(|| CoreError::InvalidInput {
            field: "github".to_string(),
            message: "select a repository before refreshing".to_string(),
        })?;
    state.core.sources_refresh(&adapter)
}

#[tauri::command]
fn github_disconnect(
    input: GitHubDisconnectInput,
    state: State<'_, AppState>,
) -> Result<(), CoreError> {
    let mut github = state.github.lock().map_err(|_| CoreError::Database {
        message: "github state mutex was poisoned".to_string(),
    })?;
    let selected = github.as_ref().ok_or_else(|| CoreError::InvalidInput {
        field: "github".to_string(),
        message: "no GitHub repository is selected".to_string(),
    })?;
    if selected.config().connection_id != input.connection_id {
        return Err(CoreError::PermissionDenied {
            provider: "github".to_string(),
            message: "disconnect scope does not match the selected repository".to_string(),
        });
    }
    if input.delete_credential {
        selected.credentials().delete(&input.connection_id)?;
    }
    *github = None;
    Ok(())
}

#[tauri::command]
fn context_search(
    request: SearchRequest,
    state: State<'_, AppState>,
) -> Result<core::SearchResponse, CoreError> {
    state.core.context_search(request)
}

#[tauri::command]
fn context_get(
    request: ContextRequest,
    state: State<'_, AppState>,
) -> Result<core::ContextResponse, CoreError> {
    state.core.context_get(request)
}

#[tauri::command]
fn context_query(
    request: ContextQueryRequest,
    state: State<'_, AppState>,
) -> Result<core::ContextQueryResponse, CoreError> {
    state.core.context_query(request)
}

#[tauri::command]
fn graph_rebuild(
    request: GraphRebuildRequest,
    state: State<'_, AppState>,
) -> Result<core::GraphRebuildResponse, CoreError> {
    state.core.graph_rebuild(request)
}

#[tauri::command]
fn relation_add(input: RelationInput, state: State<'_, AppState>) -> Result<(), CoreError> {
    state.core.add_relation(input)
}

#[tauri::command]
fn relation_remove(input: RelationInput, state: State<'_, AppState>) -> Result<bool, CoreError> {
    state.core.remove_relation(input)
}

#[tauri::command]
fn memory_upsert(
    request: MemoryUpsertInput,
    state: State<'_, AppState>,
) -> Result<core::MemoryMutation, CoreError> {
    state.core.memory_upsert(request)
}

#[tauri::command]
fn memory_retract(
    request: MemoryRetractInput,
    state: State<'_, AppState>,
) -> Result<core::MemoryMutation, CoreError> {
    state.core.memory_retract(request)
}

#[tauri::command]
fn memory_restore(
    request: MemoryRestoreInput,
    state: State<'_, AppState>,
) -> Result<core::MemoryMutation, CoreError> {
    state.core.restore_memory(request)
}

#[tauri::command]
fn observation_capture(
    request: ObservationCaptureInput,
    state: State<'_, AppState>,
) -> Result<core::ObservationMutation, CoreError> {
    state.core.observation_capture(request)
}

#[tauri::command]
fn candidate_distill(
    request: CandidateDistillInput,
    state: State<'_, AppState>,
) -> Result<core::CandidateMutation, CoreError> {
    state.core.candidate_distill(request)
}

#[tauri::command]
fn candidate_propose(
    request: CandidateProposeInput,
    state: State<'_, AppState>,
) -> Result<core::CandidateMutation, CoreError> {
    state.core.candidate_propose(request)
}

/// Human review boundary. This command is intentionally not part of the
/// authenticated generic IPC/MCP method list, so an agent cannot auto-accept
/// a candidate without an explicit trusted UI action.
#[tauri::command]
fn candidate_accept(
    request: CandidateAcceptInput,
    state: State<'_, AppState>,
) -> Result<core::CandidateAcceptance, CoreError> {
    state.core.candidate_accept(request)
}

#[tauri::command]
fn candidate_reject(
    request: CandidateRejectInput,
    state: State<'_, AppState>,
) -> Result<core::CandidateMutation, CoreError> {
    state.core.candidate_reject(request)
}

#[tauri::command]
fn candidate_list(
    state_filter: Option<CandidateState>,
    state: State<'_, AppState>,
) -> Result<Vec<core::MemoryCandidate>, CoreError> {
    state.core.candidates(state_filter)
}

#[tauri::command]
fn observation_get(id: String, state: State<'_, AppState>) -> Result<core::Observation, CoreError> {
    state.core.observation(&id)
}

#[tauri::command]
fn candidate_get(
    id: String,
    state: State<'_, AppState>,
) -> Result<core::MemoryCandidate, CoreError> {
    state.core.candidate(&id)
}

#[tauri::command]
fn memory_export_markdown(state: State<'_, AppState>) -> Result<String, CoreError> {
    state.core.memory_export_markdown()
}

#[tauri::command]
fn memory_import_markdown(
    markdown: String,
    state: State<'_, AppState>,
) -> Result<core::MemoryMarkdownImportResult, CoreError> {
    state.core.memory_import_markdown(markdown)
}

fn ensure_ui_evidence(core: &Core, evidence: &[SourceRef]) -> Result<(), CoreError> {
    for source in evidence {
        match core.snapshot(source) {
            Ok(_) => {}
            Err(CoreError::NotFound { .. }) => {
                core.ingest_snapshot(Snapshot::new(
                    source.clone(),
                    source.external_id.clone(),
                    "Evidence captured by an explicit local UI action.",
                    None,
                    core::now_rfc3339(),
                ))?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[tauri::command]
fn ui_memory_upsert(
    request: UiMemoryUpsertInput,
    state: State<'_, AppState>,
) -> Result<core::UiMemoryMutation, CoreError> {
    ensure_ui_evidence(&state.core, &request.memory.evidence)?;
    state.core.ui_memory_upsert(request)
}

#[tauri::command]
fn ui_memory_retract(
    request: MemoryRetractInput,
    state: State<'_, AppState>,
) -> Result<core::UiMemoryMutation, CoreError> {
    state.core.ui_memory_retract(request)
}

#[tauri::command]
fn ui_memory_restore(
    request: MemoryRestoreInput,
    state: State<'_, AppState>,
) -> Result<core::UiMemoryMutation, CoreError> {
    state.core.ui_memory_restore(request)
}

#[tauri::command]
fn ui_memory_list(state: State<'_, AppState>) -> Result<Vec<core::UiMemory>, CoreError> {
    state.core.ui_memories()
}

#[tauri::command]
fn cache_clear(state: State<'_, AppState>) -> Result<core::CacheClearResult, CoreError> {
    state.core.clear_cache()
}

#[tauri::command]
fn core_backup(path: String, state: State<'_, AppState>) -> Result<core::BackupResult, CoreError> {
    state.core.backup_to(path)
}

#[tauri::command]
fn core_restore(path: String, state: State<'_, AppState>) -> Result<core::BackupResult, CoreError> {
    state.core.restore_from(path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Destroyed) {
                if let Some(state) = window.try_state::<AppState>() {
                    state.core_stop.store(true, Ordering::Relaxed);
                }
            }
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir().map_err(|error| {
                Box::new(std::io::Error::other(format!(
                    "app data directory: {error}"
                ))) as Box<dyn std::error::Error>
            })?;
            fs::create_dir_all(&data_dir)?;
            let core = Core::open(data_dir.join("aidebook.sqlite"))
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            let endpoint = CoreEndpoint::in_data_dir(&data_dir);
            let server = CoreServer::bind(endpoint.clone(), core.clone(), None)
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            let core_stop = Arc::new(AtomicBool::new(false));
            let core_stop_for_server = core_stop.clone();
            std::thread::spawn(move || {
                let _ = server.serve_until(core_stop_for_server);
            });
            app.manage(endpoint);
            let plugins = Arc::new(Mutex::new(core::plugins::PluginRegistry::open(
                data_dir.join("connections.json"),
            )?));
            let local_sync = Arc::new(core::local_sync::LocalSync::new(
                core.clone(),
                plugins.clone(),
            ));
            let background_sync = local_sync.clone();
            let sync_stop = core_stop.clone();
            std::thread::spawn(move || background_sync.run(sync_stop));
            let agent_installer = Arc::new(agents::AgentInstaller::new(
                app.path().home_dir()?,
                data_dir.clone(),
                std::env::current_exe()?,
            ));
            app.manage(AppState {
                agent_installer,
                plugins,
                local_sync,
                core,
                core_stop,
                vault: Mutex::new(None),
                vault_watcher: Mutex::new(None),
                github: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            agent_connections_status,
            agent_connection_install,
            agent_connection_uninstall,
            agent_connection_probe,
            plugin_list,
            plugin_sync_status,
            plugin_sync_configure,
            plugin_add,
            plugin_remove,
            plugin_token_set,
            plugin_refresh,
            core_status,
            vault_select,
            vault_scan,
            github_select,
            github_credential_set,
            github_refresh,
            github_disconnect,
            context_search,
            context_get,
            context_query,
            graph_rebuild,
            relation_add,
            relation_remove,
            memory_upsert,
            memory_retract,
            memory_restore,
            observation_capture,
            candidate_distill,
            candidate_propose,
            candidate_accept,
            candidate_reject,
            candidate_list,
            observation_get,
            candidate_get,
            memory_export_markdown,
            memory_import_markdown,
            ui_memory_upsert,
            ui_memory_retract,
            ui_memory_restore,
            ui_memory_list,
            cache_clear,
            core_backup,
            core_restore
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
