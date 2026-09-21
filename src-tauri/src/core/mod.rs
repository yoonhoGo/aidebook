//! Aidebook's transport-neutral local core.
//!
//! `Core` is the only public entry point used by future Tauri commands, CLI,
//! and MCP adapters. It owns a SQLite database and exposes read-only source
//! ingestion, FTS5 context search, freshness, synchronization state, and
//! guarded local memory mutations. No provider token or external write is
//! accepted by this API.

mod db;
pub mod fixtures;
pub mod github;
pub mod ipc;
pub mod mcp;
pub mod obsidian;
pub mod types;

use db::Database;
pub use fixtures::{
    ChangeBatch, ConnectorConnection, FixtureAdapter, FixtureFile, ReadOnlyConnector,
};
pub use github::{
    CredentialStore, FixtureGitHubApi, GitHubAdapter, GitHubApi, GitHubApiError, GitHubComment,
    GitHubConfig, GitHubIssue, GitHubPage, HttpGitHubApi, KeychainCredentialStore,
    MemoryCredentialStore,
};
pub use ipc::{
    dispatch as ipc_dispatch, CoreClient, CoreEndpoint, CoreServer, IpcError, IpcRequest,
    IpcResponse, IPC_METHODS,
};
pub use obsidian::{
    ObsidianAdapter, VaultChange, VaultChangeKind, VaultConfig, VaultIndex, VaultScanResult,
    VaultWatcher,
};
use std::path::Path;
use std::sync::Arc;
pub use types::*;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CoreStatus {
    pub product: &'static str,
    pub version: &'static str,
    pub persistence: &'static str,
    pub connectors: &'static str,
}

pub fn status() -> CoreStatus {
    CoreStatus {
        product: "Aidebook",
        version: env!("CARGO_PKG_VERSION"),
        persistence: "SQLite + FTS5 (M0)",
        connectors: "read-only Obsidian + GitHub adapters (M1/M2)",
    }
}

#[derive(Clone)]
pub struct Core {
    database: Arc<Database>,
}

impl Core {
    pub fn open(path: impl AsRef<Path>) -> CoreResult<Self> {
        Ok(Self {
            database: Arc::new(Database::open(path)?),
        })
    }

    pub fn in_memory() -> CoreResult<Self> {
        Ok(Self {
            database: Arc::new(Database::in_memory()?),
        })
    }

    pub fn ingest_snapshot(&self, snapshot: Snapshot) -> CoreResult<IngestResult> {
        self.database.ingest_snapshot(snapshot)
    }

    pub fn ingest_connector<C: ReadOnlyConnector>(
        &self,
        connector: &C,
    ) -> CoreResult<Vec<IngestResult>> {
        connector
            .list()?
            .into_iter()
            .map(|snapshot| self.ingest_snapshot(snapshot))
            .collect()
    }

    /// Common `sources.refresh` facade. The connector can only supply
    /// read-only snapshots; the core owns persistence and sync status.
    pub fn sources_refresh<C: ReadOnlyConnector>(
        &self,
        connector: &C,
    ) -> CoreResult<SourcesRefreshResult> {
        let manifest = connector.manifest();
        let connection_id = connector.connection_id().to_string();
        let snapshots = match connector.list() {
            Ok(snapshots) => snapshots,
            Err(error) => {
                let _ = self.record_sync_failure(
                    connection_id.clone(),
                    manifest.id.clone(),
                    "fixture-or-adapter-scope",
                    error_code(&error),
                    error.to_string(),
                    None,
                    None,
                );
                return Err(error);
            }
        };
        let attempted = snapshots.len();
        let mut indexed = 0;
        let mut deduplicated = 0;
        let mut inaccessible = 0;
        for snapshot in snapshots {
            let result = match self.ingest_snapshot(snapshot) {
                Ok(result) => result,
                Err(error) => {
                    let _ = self.record_sync_failure(
                        connection_id.clone(),
                        manifest.id.clone(),
                        "fixture-or-adapter-scope",
                        error_code(&error),
                        error.to_string(),
                        None,
                        None,
                    );
                    return Err(error);
                }
            };
            indexed += usize::from(result.indexed);
            deduplicated += usize::from(result.deduplicated);
            inaccessible += usize::from(!result.access_status.is_searchable());
        }
        let completed_at = now_rfc3339();
        let sync_state = self.record_sync_success(
            connection_id.clone(),
            manifest.id.clone(),
            "fixture-or-adapter-scope",
            None,
            Some(completed_at.clone()),
        )?;
        Ok(SourcesRefreshResult {
            connection_id,
            provider: manifest.id,
            attempted,
            indexed,
            deduplicated,
            inaccessible,
            completed_at,
            sync_state,
        })
    }

    pub fn record_source_failure(
        &self,
        source: SourceRef,
        access_status: AccessStatus,
        reason: impl Into<String>,
        failed_at: Option<String>,
    ) -> CoreResult<IngestResult> {
        self.database
            .record_source_failure(source, access_status, reason, failed_at)
    }

    pub fn add_relation(&self, input: RelationInput) -> CoreResult<()> {
        self.database.add_relation(input)
    }

    pub fn remove_relation(&self, input: RelationInput) -> CoreResult<bool> {
        self.database.remove_relation(input)
    }

    pub fn search(&self, request: SearchRequest) -> CoreResult<SearchResponse> {
        self.database.search(request)
    }

    pub fn context_search(&self, request: SearchRequest) -> CoreResult<SearchResponse> {
        self.search(request)
    }

    pub fn context(&self, request: ContextRequest) -> CoreResult<ContextResponse> {
        self.database.context(request)
    }

    pub fn context_get(&self, request: ContextRequest) -> CoreResult<ContextResponse> {
        self.context(request)
    }

    pub fn record_sync_success(
        &self,
        connection_id: impl Into<String>,
        provider: impl Into<String>,
        scope: impl Into<String>,
        cursor: Option<String>,
        completed_at: Option<String>,
    ) -> CoreResult<SyncState> {
        self.database
            .record_sync_success(connection_id, provider, scope, cursor, completed_at)
    }

    pub fn record_sync_failure(
        &self,
        connection_id: impl Into<String>,
        provider: impl Into<String>,
        scope: impl Into<String>,
        error_code: impl Into<String>,
        error: impl Into<String>,
        failed_at: Option<String>,
        retry_at: Option<String>,
    ) -> CoreResult<SyncState> {
        self.database.record_sync_failure(
            connection_id,
            provider,
            scope,
            error_code,
            error,
            failed_at,
            retry_at,
        )
    }

    pub fn sync_state(&self, connection_id: &str) -> CoreResult<SyncState> {
        self.database.sync_state(connection_id)
    }

    pub fn connections_status(&self, connection_id: &str) -> CoreResult<SyncState> {
        self.sync_state(connection_id)
    }

    pub fn snapshot(&self, source: &SourceRef) -> CoreResult<Snapshot> {
        self.database.snapshot(source)
    }

    pub fn rebuild_graph(&self, request: GraphRebuildRequest) -> CoreResult<GraphRebuildResponse> {
        self.database.rebuild_graph(request)
    }

    pub fn graph_rebuild(&self, request: GraphRebuildRequest) -> CoreResult<GraphRebuildResponse> {
        self.rebuild_graph(request)
    }

    pub fn graph_traverse(
        &self,
        request: GraphTraversalRequest,
    ) -> CoreResult<GraphTraversalResponse> {
        self.database.graph_traverse(request)
    }

    pub fn graph_query(
        &self,
        request: GraphTraversalRequest,
    ) -> CoreResult<GraphTraversalResponse> {
        self.graph_traverse(request)
    }

    pub fn upsert_memory(&self, input: MemoryUpsertInput) -> CoreResult<MemoryMutation> {
        self.database.upsert_memory(input)
    }

    pub fn memory_upsert(&self, input: MemoryUpsertInput) -> CoreResult<MemoryMutation> {
        self.upsert_memory(input)
    }

    pub fn retract_memory(&self, input: MemoryRetractInput) -> CoreResult<MemoryMutation> {
        self.database.retract_memory(input)
    }

    pub fn memory_retract(&self, input: MemoryRetractInput) -> CoreResult<MemoryMutation> {
        self.retract_memory(input)
    }

    pub fn restore_memory(&self, input: MemoryRestoreInput) -> CoreResult<MemoryMutation> {
        self.database.restore_memory(input)
    }

    pub fn ui_memory_upsert(&self, input: UiMemoryUpsertInput) -> CoreResult<UiMemoryMutation> {
        self.database.ui_memory_upsert(input)
    }

    pub fn ui_memory_retract(&self, input: MemoryRetractInput) -> CoreResult<UiMemoryMutation> {
        self.database.ui_memory_retract(input)
    }

    pub fn ui_memory_restore(&self, input: MemoryRestoreInput) -> CoreResult<UiMemoryMutation> {
        self.database.ui_memory_restore(input)
    }

    pub fn ui_memories(&self) -> CoreResult<Vec<UiMemory>> {
        self.database.ui_memories()
    }

    pub fn clear_cache(&self) -> CoreResult<CacheClearResult> {
        self.database.clear_cache()
    }

    pub fn backup_to(&self, destination: impl AsRef<Path>) -> CoreResult<BackupResult> {
        self.database.backup_to(destination)
    }

    pub fn restore_from(&self, backup: impl AsRef<Path>) -> CoreResult<BackupResult> {
        self.database.restore_from(backup)
    }

    pub fn memory(&self, id: &str) -> CoreResult<Memory> {
        self.database.memory(id)
    }

    pub fn memory_history(&self, id: &str) -> CoreResult<Vec<MemoryRevision>> {
        self.database.memory_history(id)
    }

    pub fn capture_observation(
        &self,
        input: ObservationCaptureInput,
    ) -> CoreResult<ObservationMutation> {
        self.database.capture_observation(input)
    }

    pub fn observation_capture(
        &self,
        input: ObservationCaptureInput,
    ) -> CoreResult<ObservationMutation> {
        self.capture_observation(input)
    }

    pub fn distill_candidate(&self, input: CandidateDistillInput) -> CoreResult<CandidateMutation> {
        self.database.distill_candidate(input)
    }

    pub fn candidate_distill(&self, input: CandidateDistillInput) -> CoreResult<CandidateMutation> {
        self.distill_candidate(input)
    }

    pub fn propose_candidate(&self, input: CandidateProposeInput) -> CoreResult<CandidateMutation> {
        self.database.propose_candidate(input)
    }

    pub fn candidate_propose(&self, input: CandidateProposeInput) -> CoreResult<CandidateMutation> {
        self.propose_candidate(input)
    }

    pub fn accept_candidate(&self, input: CandidateAcceptInput) -> CoreResult<CandidateAcceptance> {
        self.database.accept_candidate(input)
    }

    pub fn candidate_accept(&self, input: CandidateAcceptInput) -> CoreResult<CandidateAcceptance> {
        self.accept_candidate(input)
    }

    pub fn reject_candidate(&self, input: CandidateRejectInput) -> CoreResult<CandidateMutation> {
        self.database.reject_candidate(input)
    }

    pub fn candidate_reject(&self, input: CandidateRejectInput) -> CoreResult<CandidateMutation> {
        self.reject_candidate(input)
    }

    pub fn observation(&self, id: &str) -> CoreResult<Observation> {
        self.database.observation(id)
    }

    pub fn candidate(&self, id: &str) -> CoreResult<MemoryCandidate> {
        self.database.candidate(id)
    }

    pub fn candidates(&self, state: Option<CandidateState>) -> CoreResult<Vec<MemoryCandidate>> {
        self.database.candidates(state)
    }
}

fn error_code(error: &CoreError) -> &'static str {
    match error {
        CoreError::InvalidInput { .. } => "invalid_input",
        CoreError::NotFound { .. } => "not_found",
        CoreError::VersionConflict { .. } => "version_conflict",
        CoreError::IdempotencyConflict { .. } => "idempotency_conflict",
        CoreError::PermissionDenied { .. } => "permission_denied",
        CoreError::SensitiveDataRejected => "sensitive_data_rejected",
        CoreError::Migration { .. } => "migration",
        CoreError::Database { .. } => "database",
        CoreError::Connector { .. } => "connector",
        CoreError::Provider { .. } => "provider",
    }
}
