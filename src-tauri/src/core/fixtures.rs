//! Read-only connector contracts and deterministic fixtures used by M0 tests.
//!
//! A fixture implements the same boundary that a real Obsidian or GitHub
//! adapter will implement later.  It cannot write to a provider and it never
//! receives credentials.  Keeping the fixture at this boundary makes tests
//! useful without touching a user's vault or account.

use super::types::{ConnectorManifest, CoreError, CoreResult, Snapshot, SourceRef};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub trait ReadOnlyConnector {
    fn manifest(&self) -> ConnectorManifest;
    fn connection_id(&self) -> &str;
    fn list(&self) -> CoreResult<Vec<Snapshot>>;
    fn fetch(&self, source: &SourceRef) -> CoreResult<Snapshot>;

    /// Establishes a read-only scope. Real adapters will validate account and
    /// repository/vault scope here; fixtures only expose their declared scope.
    fn connect(&self, scope: &str) -> CoreResult<ConnectorConnection> {
        if scope.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "scope".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        Ok(ConnectorConnection {
            connection_id: self.connection_id().to_string(),
            provider: self.manifest().id,
            scope: scope.to_string(),
        })
    }

    /// Returns a read-only change batch. A live connector may use a cursor;
    /// the deterministic fixture returns all of its snapshots.
    fn sync(&self, _cursor: Option<&str>) -> CoreResult<ChangeBatch> {
        Ok(ChangeBatch {
            snapshots: self.list()?,
            tombstones: Vec::new(),
            next_cursor: None,
        })
    }

    fn disconnect(&self) -> CoreResult<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorConnection {
    pub connection_id: String,
    pub provider: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeBatch {
    pub snapshots: Vec<Snapshot>,
    pub tombstones: Vec<SourceRef>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureFile {
    pub provider: String,
    pub connection_id: String,
    pub snapshots: Vec<Snapshot>,
}

#[derive(Debug, Clone)]
pub struct FixtureAdapter {
    fixture: FixtureFile,
}

impl FixtureAdapter {
    pub fn from_json(json: &str) -> CoreResult<Self> {
        let fixture: FixtureFile =
            serde_json::from_str(json).map_err(|error| CoreError::Connector {
                message: format!("invalid fixture JSON: {error}"),
            })?;
        if fixture.provider.trim().is_empty() || fixture.connection_id.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "fixture".to_string(),
                message: "provider and connection_id are required".to_string(),
            });
        }
        for snapshot in &fixture.snapshots {
            if snapshot.source.provider != fixture.provider {
                return Err(CoreError::InvalidInput {
                    field: "fixture.snapshots.source.provider".to_string(),
                    message: "must match fixture.provider".to_string(),
                });
            }
            snapshot.source.validate()?;
        }
        Ok(Self { fixture })
    }

    pub fn from_path(path: impl AsRef<Path>) -> CoreResult<Self> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path).map_err(|error| CoreError::Connector {
            message: format!("could not read fixture {}: {error}", path.display()),
        })?;
        Self::from_json(&json)
    }
}

impl ReadOnlyConnector for FixtureAdapter {
    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            id: self.fixture.provider.clone(),
            version: "fixture-1".to_string(),
            api_version: "1".to_string(),
            capabilities: vec![
                "read".to_string(),
                "search".to_string(),
                "delta".to_string(),
            ],
            permissions: vec!["read-only".to_string()],
        }
    }

    fn connection_id(&self) -> &str {
        &self.fixture.connection_id
    }

    fn list(&self) -> CoreResult<Vec<Snapshot>> {
        self.fixture
            .snapshots
            .iter()
            .cloned()
            .map(Snapshot::normalize)
            .collect()
    }

    fn fetch(&self, source: &SourceRef) -> CoreResult<Snapshot> {
        self.fixture
            .snapshots
            .iter()
            .find(|snapshot| snapshot.source == *source)
            .cloned()
            .ok_or_else(|| CoreError::NotFound {
                entity: "fixture source".to_string(),
                id: source.id(),
            })?
            .normalize()
    }
}
