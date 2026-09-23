//! App-lifetime, read-only polling for all explicitly connected local vaults.
use super::{
    plugins::{ConnectionPatch, PluginConnection, PluginRegistry, Provider},
    *,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

pub const POLL_SECONDS: u64 = 10;
#[derive(Clone, Serialize, Default)]
pub struct LocalSyncStatus {
    pub id: String,
    pub running: bool,
    pub last_checked_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_graph_at: Option<String>,
    pub indexed: usize,
    pub inaccessible: usize,
    pub changed: usize,
    pub removed: usize,
    pub error: Option<String>,
}
fn unavailable() -> CoreError {
    CoreError::Connector {
        message: "local refresh state is unavailable".into(),
    }
}

pub struct LocalSync {
    core: Core,
    registry: Arc<Mutex<PluginRegistry>>,
    // Serializes manual/automatic local scans and disconnect/toggle operations.
    operation: Mutex<HashSet<String>>,
    statuses: Mutex<HashMap<String, LocalSyncStatus>>,
}
impl LocalSync {
    pub fn new(core: Core, registry: Arc<Mutex<PluginRegistry>>) -> Self {
        Self {
            core,
            registry,
            operation: Mutex::new(HashSet::new()),
            statuses: Mutex::new(HashMap::new()),
        }
    }
    pub fn connections(&self) -> CoreResult<Vec<PluginConnection>> {
        Ok(self.registry.lock().map_err(|_| unavailable())?.list())
    }
    pub fn connection(&self, id: &str) -> CoreResult<PluginConnection> {
        self.registry.lock().map_err(|_| unavailable())?.get(id)
    }
    pub fn add(&self, connection: PluginConnection) -> CoreResult<PluginConnection> {
        let _guard = self.operation.lock().map_err(|_| unavailable())?;
        self.registry
            .lock()
            .map_err(|_| unavailable())?
            .add(connection)
    }
    pub fn update(&self, id: &str, patch: ConnectionPatch) -> CoreResult<PluginConnection> {
        let mut ready = self.operation.lock().map_err(|_| unavailable())?;
        let connection = self
            .registry
            .lock()
            .map_err(|_| unavailable())?
            .update(id, patch)?;
        // The next scan must rebuild relations for a changed scope, even when content matches.
        ready.remove(id);
        self.statuses.lock().map_err(|_| unavailable())?.remove(id);
        Ok(connection)
    }
    pub fn statuses(&self) -> CoreResult<Vec<LocalSyncStatus>> {
        Ok(self
            .statuses
            .lock()
            .map_err(|_| unavailable())?
            .values()
            .cloned()
            .collect())
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> CoreResult<()> {
        let _guard = self.operation.lock().map_err(|_| unavailable())?;
        self.registry
            .lock()
            .map_err(|_| unavailable())?
            .set_auto_sync(id, enabled)
    }
    pub fn remove(&self, id: &str) -> CoreResult<()> {
        self.remove_with_credentials(id, false)
    }
    pub fn remove_with_credentials(&self, id: &str, delete_credential: bool) -> CoreResult<()> {
        let mut ready = self.operation.lock().map_err(|_| unavailable())?;
        let mut registry = self.registry.lock().map_err(|_| unavailable())?;
        let connection = registry.get(id)?;
        if delete_credential {
            match connection.auth {
                plugins::AuthMethod::Token => {
                    KeychainCredentialStore.delete(&plugins::credential_key(id))?
                }
                plugins::AuthMethod::Oauth => super::oauth::delete_credentials(id)?,
                _ => (),
            }
        }
        registry.remove(id)?;
        ready.remove(id);
        self.statuses.lock().map_err(|_| unavailable())?.remove(id);
        Ok(())
    }
    /// One deterministic pass, also used by integration tests without wall-clock sleeps.
    pub fn poll_once(&self, stop: &AtomicBool) -> CoreResult<()> {
        let connections = self.registry.lock().map_err(|_| unavailable())?.list();
        for connection in connections {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if connection.provider == Provider::Obsidian && connection.auto_sync {
                // A failed/offline vault must not prevent other local paths from updating.
                let _ = self.refresh(&connection.id, true);
            }
        }
        Ok(())
    }
    pub fn run(&self, stop: Arc<AtomicBool>) {
        while !stop.load(Ordering::Relaxed) {
            let _ = self.poll_once(&stop);
            for _ in 0..POLL_SECONDS {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
    pub fn refresh(&self, id: &str, automatic: bool) -> CoreResult<SourcesRefreshResult> {
        let mut ready = self.operation.lock().map_err(|_| unavailable())?;
        // Recheck after taking the operation lock: removed/paused paths cannot start a stale queued scan.
        let connection = self.registry.lock().map_err(|_| unavailable())?.get(id)?;
        if !automatic && connection.provider != Provider::Obsidian {
            return plugins::refresh(&self.core, connection);
        }
        if connection.provider != Provider::Obsidian || (automatic && !connection.auto_sync) {
            return Err(CoreError::InvalidInput {
                field: "connection".into(),
                message: "local automatic refresh is disabled".into(),
            });
        }
        {
            let mut statuses = self.statuses.lock().map_err(|_| unavailable())?;
            let status = statuses
                .entry(id.into())
                .or_insert_with(|| LocalSyncStatus {
                    id: id.into(),
                    ..Default::default()
                });
            status.running = true;
        }
        let result = refresh_vault(&self.core, &connection, !ready.contains(id));
        let mut statuses = self.statuses.lock().map_err(|_| unavailable())?;
        let status = statuses.get_mut(id).ok_or_else(unavailable)?;
        status.running = false;
        status.last_checked_at = Some(now_rfc3339());
        match result {
            Ok((refresh, changed, removed, graph_updated)) => {
                ready.insert(id.into());
                status.last_success_at = Some(refresh.completed_at.clone());
                if graph_updated {
                    status.last_graph_at = Some(now_rfc3339());
                }
                status.indexed = refresh.indexed;
                status.inaccessible = refresh.inaccessible.saturating_sub(removed);
                status.changed = changed;
                status.removed = removed;
                status.error = None;
                Ok(refresh)
            }
            Err(error) => {
                // Retry graph work even when ingestion succeeded before a graph failure.
                ready.remove(id);
                status.error = Some(error.to_string());
                let _ = self.core.record_sync_failure(
                    id,
                    "obsidian",
                    &connection.scope,
                    "local_sync_failed",
                    error.to_string(),
                    None,
                    None,
                );
                Err(error)
            }
        }
    }
}

struct ScannedVault {
    id: String,
    snapshots: Vec<Snapshot>,
}
impl ReadOnlyConnector for ScannedVault {
    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            id: "obsidian".into(),
            version: "1".into(),
            api_version: "1".into(),
            capabilities: vec!["read".into()],
            permissions: vec!["selected-vault-read".into()],
        }
    }
    fn connection_id(&self) -> &str {
        &self.id
    }
    fn list(&self) -> CoreResult<Vec<Snapshot>> {
        Ok(self.snapshots.clone())
    }
    fn fetch(&self, source: &SourceRef) -> CoreResult<Snapshot> {
        self.snapshots
            .iter()
            .find(|s| &s.source == source)
            .cloned()
            .ok_or_else(|| CoreError::NotFound {
                entity: "snapshot".into(),
                id: source.id(),
            })
    }
}
fn same_content(a: &Snapshot, b: &Snapshot) -> bool {
    if a.source != b.source || a.access_status != b.access_status || a.is_deleted != b.is_deleted {
        return false;
    }
    if !b.access_status.is_searchable() {
        return a.unavailable_reason == b.unavailable_reason;
    }
    a.content_hash == b.content_hash && a.links == b.links && a.metadata == b.metadata
}

fn refresh_vault(
    core: &Core,
    connection: &PluginConnection,
    force_graph: bool,
) -> CoreResult<(SourcesRefreshResult, usize, usize, bool)> {
    let adapter = ObsidianAdapter::open(VaultConfig::new(
        &connection.scope,
        &connection.id,
        &connection.id,
    ))?;
    if adapter.root_path() != std::path::Path::new(&connection.scope) {
        return Err(CoreError::PermissionDenied { provider: "obsidian".into(), message: "the connected vault path now resolves to a different location; reconnect it explicitly".into() });
    }
    // Finish the entire traversal before reconciling deletions. An unreadable root/subdirectory
    // or transient read failure must never be interpreted as an empty vault.
    let scan = adapter.scan()?;
    let mut snapshots = scan.snapshots;
    let present: HashSet<String> = snapshots.iter().map(|s| s.source.id()).collect();
    let mut changed = 0;
    for snapshot in &snapshots {
        match core.snapshot(&snapshot.source) {
            Ok(previous) => changed += usize::from(!same_content(&previous, snapshot)),
            Err(CoreError::NotFound { .. }) => changed += 1,
            Err(error) => return Err(error),
        }
    }
    let mut removed = 0;
    if scan.inaccessible == 0 {
        for source in core.cached_sources("obsidian", &connection.id)? {
            if !present.contains(&source.id()) {
                let mut tombstone = core.snapshot(&source)?;
                tombstone.is_deleted = true;
                tombstone.access_status = AccessStatus::NotFound;
                tombstone.unavailable_reason = Some("file removed from the connected vault".into());
                snapshots.push(tombstone);
                removed += 1;
            }
        }
    }
    let refresh = core.sources_refresh(&ScannedVault {
        id: connection.id.clone(),
        snapshots,
    })?;
    let graph_updated = force_graph || changed > 0 || removed > 0;
    if graph_updated {
        core.graph_rebuild(GraphRebuildRequest::default())?;
    }
    Ok((refresh, changed, removed, graph_updated))
}
