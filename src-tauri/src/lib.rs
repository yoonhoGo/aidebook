pub mod core;

use core::{
    ContextRequest, Core, CoreError, CredentialStore, GitHubAdapter, GitHubConfig, HttpGitHubApi,
    KeychainCredentialStore, MemoryRestoreInput, MemoryRetractInput, MemoryUpsertInput,
    ObsidianAdapter, ReadOnlyConnector, SearchRequest, SourcesRefreshResult, VaultConfig,
    VaultScanResult,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;
use tauri::{Manager, State};

struct AppState {
    core: Core,
    vault: Mutex<Option<ObsidianAdapter>>,
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
    let mut vault = state.vault.lock().map_err(|_| CoreError::Database {
        message: "vault state mutex was poisoned".to_string(),
    })?;
    *vault = Some(adapter);
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
    let refresh = state.core.sources_refresh(&adapter)?;
    Ok(VaultScanResponse { scan, refresh })
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir().map_err(|error| {
                Box::new(std::io::Error::other(format!(
                    "app data directory: {error}"
                ))) as Box<dyn std::error::Error>
            })?;
            fs::create_dir_all(&data_dir)?;
            let core = Core::open(data_dir.join("aidebook.sqlite"))
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            app.manage(AppState {
                core,
                vault: Mutex::new(None),
                github: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_status,
            vault_select,
            vault_scan,
            github_select,
            github_credential_set,
            github_refresh,
            context_search,
            context_get,
            memory_upsert,
            memory_retract,
            memory_restore
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
