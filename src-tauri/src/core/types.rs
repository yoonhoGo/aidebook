//! Transport-neutral domain types for the Aidebook local core.
//!
//! These types are deliberately independent of Tauri and SQLite.  The UI,
//! future CLI, and future MCP server can all use the same request/response
//! shapes without opening the database themselves.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

pub const DEFAULT_SEARCH_LIMIT: usize = 20;
pub const MAX_SEARCH_LIMIT: usize = 50;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessStatus {
    Accessible,
    PermissionDenied,
    NotFound,
    Unavailable,
}

impl AccessStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accessible => "accessible",
            Self::PermissionDenied => "permission_denied",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn from_str(value: &str) -> CoreResult<Self> {
        match value {
            "accessible" => Ok(Self::Accessible),
            "permission_denied" => Ok(Self::PermissionDenied),
            "not_found" => Ok(Self::NotFound),
            "unavailable" => Ok(Self::Unavailable),
            other => Err(CoreError::Database {
                message: format!("unknown access status '{other}'"),
            }),
        }
    }

    pub fn is_searchable(&self) -> bool {
        matches!(self, Self::Accessible)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Stale,
    Unavailable,
    Unknown,
}

impl Freshness {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unavailable => "unavailable",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    /// Connector namespace, for example `obsidian` or `github`.
    pub provider: String,
    /// Provider account or workspace namespace.  It is never inferred from a
    /// title or URL.
    pub account_id: String,
    /// Stable provider-side identifier (a vault-relative path is valid for
    /// Obsidian).
    pub external_id: String,
    /// Canonical source URL.  Local sources may use a `file:` or
    /// `obsidian:` URL.
    pub url: String,
    /// Provider-specific source type, for example `note`, `issue`, or `pull_request`.
    pub kind: String,
}

impl SourceRef {
    pub fn new(
        provider: impl Into<String>,
        account_id: impl Into<String>,
        external_id: impl Into<String>,
        url: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            account_id: account_id.into(),
            external_id: external_id.into(),
            url: url.into(),
            kind: kind.into(),
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        for (field, value) in [
            ("provider", &self.provider),
            ("account_id", &self.account_id),
            ("external_id", &self.external_id),
            ("url", &self.url),
            ("kind", &self.kind),
        ] {
            if value.trim().is_empty() {
                return Err(CoreError::InvalidInput {
                    field: field.to_string(),
                    message: "must not be empty".to_string(),
                });
            }
        }

        let valid_scheme = ["http://", "https://", "file:", "obsidian:"]
            .iter()
            .any(|prefix| self.url.starts_with(prefix));
        if !valid_scheme {
            return Err(CoreError::InvalidInput {
                field: "url".to_string(),
                message: "must be a canonical http(s), file, or obsidian URL".to_string(),
            });
        }
        Ok(())
    }

    pub fn id(&self) -> String {
        source_ref_id(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub source: SourceRef,
    pub title: String,
    pub body: String,
    pub source_updated_at: Option<String>,
    pub fetched_at: String,
    /// SHA-256 of the canonical title/body payload.  It is computed by the
    /// core if a connector leaves it empty.
    pub content_hash: String,
    pub access_status: AccessStatus,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    #[serde(default)]
    pub is_deleted: bool,
    /// Allowed connector metadata. Provider-specific unknown frontmatter is
    /// never promoted into the common contract.
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
    /// Explicit links discovered by a connector. They are candidates for a
    /// relation, never an implicit title-based merge.
    #[serde(default)]
    pub links: Vec<SourceLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLink {
    pub target: String,
    pub kind: String,
}

/// Provenance for a derived graph edge. `explicit` mirrors the user's
/// canonical relation table; `extracted` comes from a snapshot link. The
/// current local implementation never invents `inferred` edges, but the
/// value is part of the versioned graph contract for future adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphProvenance {
    Explicit,
    Extracted,
    Inferred,
}

impl GraphProvenance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Extracted => "extracted",
            Self::Inferred => "inferred",
        }
    }

    pub fn from_str(value: &str) -> CoreResult<Self> {
        match value {
            "explicit" => Ok(Self::Explicit),
            "extracted" => Ok(Self::Extracted),
            "inferred" => Ok(Self::Inferred),
            other => Err(CoreError::Database {
                message: format!("unknown graph provenance '{other}'"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub source: SourceRef,
    pub title: String,
    pub snapshot_hash: String,
    pub link_digest: String,
    pub build_id: String,
    pub access_status: AccessStatus,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub from_source_id: String,
    pub to_source_id: String,
    pub from: SourceRef,
    pub to: SourceRef,
    pub relation_type: String,
    pub provenance: GraphProvenance,
    pub evidence_location: Option<String>,
    pub confidence: Option<f64>,
    pub source_url: String,
    pub target_url: Option<String>,
    pub source_hash: String,
    pub target_hash: Option<String>,
    pub build_id: String,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphBuild {
    pub build_id: String,
    pub digest: String,
    pub created_at: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub skipped_links: usize,
    pub ambiguous_links: usize,
    pub is_current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GraphRebuildRequest {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

impl GraphRebuildRequest {
    pub fn validate(&self) -> CoreResult<()> {
        if self
            .provider
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(CoreError::InvalidInput {
                field: "provider".to_string(),
                message: "must not be empty when supplied".to_string(),
            });
        }
        if self
            .account_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(CoreError::InvalidInput {
                field: "account_id".to_string(),
                message: "must not be empty when supplied".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphRebuildResponse {
    pub build: GraphBuild,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GraphTraversalRequest {
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source: Option<SourceRef>,
    #[serde(default)]
    pub max_depth: Option<usize>,
    #[serde(default)]
    pub max_nodes: Option<usize>,
    #[serde(default)]
    pub max_edges: Option<usize>,
}

impl GraphTraversalRequest {
    pub fn validate(&self) -> CoreResult<()> {
        if self.source_id.is_none() && self.source.is_none() {
            return Err(CoreError::InvalidInput {
                field: "source_id".to_string(),
                message: "one of source_id or source is required".to_string(),
            });
        }
        for (field, value, maximum) in [
            ("max_depth", self.max_depth, 8),
            ("max_nodes", self.max_nodes, 200),
            ("max_edges", self.max_edges, 400),
        ] {
            if value == Some(0) {
                return Err(CoreError::InvalidInput {
                    field: field.to_string(),
                    message: "must be greater than zero".to_string(),
                });
            }
            if value.is_some_and(|value| value > maximum) {
                return Err(CoreError::InvalidInput {
                    field: field.to_string(),
                    message: format!("must not exceed {maximum}"),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphTraversalResponse {
    pub build_id: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub unavailable_sources: Vec<SourceRef>,
    pub stale_sources: Vec<SourceRef>,
    pub diagnostics: Vec<String>,
    pub truncated: bool,
}

impl Snapshot {
    pub fn new(
        source: SourceRef,
        title: impl Into<String>,
        body: impl Into<String>,
        source_updated_at: Option<String>,
        fetched_at: impl Into<String>,
    ) -> Self {
        let title = title.into();
        let body = body.into();
        let content_hash = content_hash(&title, &body);
        Self {
            source,
            title,
            body,
            source_updated_at,
            fetched_at: fetched_at.into(),
            content_hash,
            access_status: AccessStatus::Accessible,
            unavailable_reason: None,
            is_deleted: false,
            metadata: BTreeMap::new(),
            links: Vec::new(),
        }
    }

    pub fn normalize(mut self) -> CoreResult<Self> {
        self.source.validate()?;
        if self.fetched_at.trim().is_empty() {
            self.fetched_at = now_rfc3339();
        }
        parse_timestamp(&self.fetched_at)?;
        if let Some(source_updated_at) = self.source_updated_at.as_deref() {
            parse_timestamp(source_updated_at)?;
        }
        let computed = content_hash(&self.title, &self.body);
        if self.content_hash.trim().is_empty() {
            self.content_hash = computed;
        } else if self.content_hash != computed {
            return Err(CoreError::InvalidInput {
                field: "content_hash".to_string(),
                message: "does not match the title/body payload".to_string(),
            });
        }
        if self.is_deleted {
            self.access_status = AccessStatus::NotFound;
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub source_updated_after: Option<String>,
    #[serde(default)]
    pub source_updated_before: Option<String>,
    #[serde(default)]
    pub max_age_seconds: Option<i64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl SearchRequest {
    pub fn effective_limit(&self) -> CoreResult<usize> {
        let limit = self.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        if limit == 0 {
            return Err(CoreError::InvalidInput {
                field: "limit".to_string(),
                message: "must be greater than zero".to_string(),
            });
        }
        if limit > MAX_SEARCH_LIMIT {
            return Err(CoreError::InvalidInput {
                field: "limit".to_string(),
                message: format!("must not exceed {MAX_SEARCH_LIMIT}"),
            });
        }
        if self.max_age_seconds.is_some_and(|age| age < 0) {
            return Err(CoreError::InvalidInput {
                field: "max_age_seconds".to_string(),
                message: "must not be negative".to_string(),
            });
        }
        Ok(limit)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub source: SourceRef,
    pub title: String,
    pub snippet: String,
    pub source_updated_at: Option<String>,
    pub fetched_at: String,
    pub freshness: Freshness,
    pub access_status: AccessStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub limit: usize,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRequest {
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source: Option<SourceRef>,
    #[serde(default)]
    pub max_age_seconds: Option<i64>,
}

impl ContextRequest {
    pub fn validate(&self) -> CoreResult<()> {
        if self.source_id.is_none() && self.source.is_none() {
            return Err(CoreError::InvalidInput {
                field: "source_id".to_string(),
                message: "one of source_id or source is required".to_string(),
            });
        }
        if self.max_age_seconds.is_some_and(|age| age < 0) {
            return Err(CoreError::InvalidInput {
                field: "max_age_seconds".to_string(),
                message: "must not be negative".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextResponse {
    pub context_id: String,
    pub sources: Vec<SearchResult>,
    pub memories: Vec<Memory>,
    pub unavailable_sources: Vec<SourceRef>,
    pub stale: bool,
    pub conflicts: Vec<String>,
    pub missing_providers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub body: String,
    pub reason: String,
    pub evidence: Vec<SourceRef>,
    pub author: String,
    pub claim_type: String,
    pub version: i64,
    pub idempotency_key: String,
    pub created_at: String,
    pub updated_at: String,
    pub retracted_at: Option<String>,
    pub supersedes_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryUpsertInput {
    #[serde(default)]
    pub id: Option<String>,
    pub body: String,
    pub reason: String,
    pub evidence: Vec<SourceRef>,
    pub author: String,
    pub claim_type: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub expected_version: Option<i64>,
    #[serde(default)]
    pub supersedes_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRetractInput {
    pub id: String,
    pub expected_version: i64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRestoreInput {
    pub id: String,
    pub expected_version: i64,
    pub revision_version: i64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRevision {
    pub memory_id: String,
    pub version: i64,
    pub body: String,
    pub reason: String,
    pub evidence: Vec<SourceRef>,
    pub author: String,
    pub claim_type: String,
    pub supersedes_id: Option<String>,
    pub action: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMutation {
    pub memory: Memory,
    pub created: bool,
    pub idempotent_replay: bool,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Captured,
    Distilled,
    Proposed,
    Accepted,
    Rejected,
}

impl CandidateState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Captured => "captured",
            Self::Distilled => "distilled",
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    pub fn from_str(value: &str) -> CoreResult<Self> {
        match value {
            "captured" => Ok(Self::Captured),
            "distilled" => Ok(Self::Distilled),
            "proposed" => Ok(Self::Proposed),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            other => Err(CoreError::Database {
                message: format!("unknown candidate state '{other}'"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub session_id: String,
    pub body: String,
    pub evidence: Vec<SourceRef>,
    pub actor: String,
    pub state: CandidateState,
    pub version: i64,
    pub idempotency_key: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationCaptureInput {
    #[serde(default)]
    pub id: Option<String>,
    pub session_id: String,
    pub body: String,
    pub evidence: Vec<SourceRef>,
    pub actor: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationMutation {
    pub observation: Observation,
    pub idempotent_replay: bool,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateDistillInput {
    pub observation_id: String,
    pub body: String,
    pub reason: String,
    pub author: String,
    pub claim_type: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub expected_version: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateProposeInput {
    pub id: String,
    pub expected_version: i64,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateAcceptInput {
    pub id: String,
    pub expected_version: i64,
    pub idempotency_key: String,
    #[serde(default)]
    pub memory_id: Option<String>,
    #[serde(default)]
    pub expected_memory_version: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRejectInput {
    pub id: String,
    pub expected_version: i64,
    pub idempotency_key: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCandidate {
    pub id: String,
    pub observation_id: String,
    pub body: String,
    pub reason: String,
    pub evidence: Vec<SourceRef>,
    pub author: String,
    pub claim_type: String,
    pub state: CandidateState,
    pub version: i64,
    pub idempotency_key: String,
    pub created_at: String,
    pub updated_at: String,
    pub accepted_memory_id: Option<String>,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateMutation {
    pub candidate: MemoryCandidate,
    pub idempotent_replay: bool,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateAcceptance {
    pub candidate: MemoryCandidate,
    pub memory: Memory,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiMemory {
    pub id: String,
    pub title: String,
    pub work: i64,
    pub kind: String,
    pub memory: Memory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiMemoryUpsertInput {
    pub title: String,
    pub work: i64,
    pub kind: String,
    pub memory: MemoryUpsertInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiMemoryMutation {
    pub memory: UiMemory,
    pub created: bool,
    pub idempotent_replay: bool,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationInput {
    pub from: SourceRef,
    pub to: SourceRef,
    pub relation_type: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestResult {
    pub source_id: String,
    pub deduplicated: bool,
    pub indexed: bool,
    pub access_status: AccessStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcesRefreshResult {
    pub connection_id: String,
    pub provider: String,
    pub attempted: usize,
    pub indexed: usize,
    pub deduplicated: usize,
    pub inaccessible: usize,
    pub completed_at: String,
    pub sync_state: SyncState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheClearResult {
    pub snapshots_removed: usize,
    pub sources_preserved: bool,
    pub memories_preserved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupResult {
    pub path: String,
    pub schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    pub connection_id: String,
    pub provider: String,
    pub scope: String,
    pub status: String,
    pub last_success_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub failure_at: Option<String>,
    pub cursor: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error: Option<String>,
    pub retry_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorManifest {
    pub id: String,
    pub version: String,
    pub api_version: String,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum CoreError {
    #[error("invalid input for {field}: {message}")]
    InvalidInput { field: String, message: String },
    #[error("{entity} '{id}' was not found")]
    NotFound { entity: String, id: String },
    #[error("version conflict for {entity} '{id}': expected {expected}, actual {actual}")]
    VersionConflict {
        entity: String,
        id: String,
        expected: i64,
        actual: i64,
    },
    #[error("idempotency key conflict for '{key}'")]
    IdempotencyConflict { key: String },
    #[error("permission denied for {provider}: {message}")]
    PermissionDenied { provider: String, message: String },
    #[error("sensitive credential-like data is not allowed in a memory")]
    SensitiveDataRejected,
    #[error("migration {version} failed: {message}")]
    Migration { version: i64, message: String },
    #[error("database error: {message}")]
    Database { message: String },
    #[error("connector error: {message}")]
    Connector { message: String },
    #[error("{provider} provider error ({code}): {message}")]
    Provider {
        provider: String,
        code: String,
        message: String,
        retry_at: Option<String>,
    },
}

impl fmt::Display for AccessStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn parse_timestamp(value: &str) -> CoreResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(|error| CoreError::InvalidInput {
            field: "timestamp".to_string(),
            message: error.to_string(),
        })
}

pub fn source_ref_id(source: &SourceRef) -> String {
    // URL changes must update metadata on the same source rather than create a
    // duplicate cache row.  Identity is provider/account/external_id/kind;
    // the URL remains part of the returned SourceRef and can be corrected by a
    // later read-only refresh.
    let canonical = format!(
        "{}\0{}\0{}\0{}",
        source.provider, source.account_id, source.external_id, source.kind
    );
    format!("src_{}", sha256_hex(canonical.as_bytes())[..24].to_string())
}

pub fn content_hash(title: &str, body: &str) -> String {
    let mut payload = Vec::with_capacity(title.len() + body.len() + 1);
    payload.extend_from_slice(title.as_bytes());
    payload.push(0);
    payload.extend_from_slice(body.as_bytes());
    sha256_hex(&payload)
}

pub fn sha256_hex(payload: &[u8]) -> String {
    let digest = Sha256::digest(payload);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn contains_credential_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let token_prefixes = ["ghp_", "github_pat_", "xoxb-", "xoxp-", "sk-"];
    if token_prefixes.iter().any(|marker| {
        lower.starts_with(marker)
            || [" ", "\n", "\t", "=", ":", "\"", "'"]
                .iter()
                .any(|boundary| lower.contains(&format!("{boundary}{marker}")))
    }) {
        return true;
    }
    ["password=", "token=", "secret=", "api_key=", "apikey="]
        .iter()
        .any(|marker| lower.contains(marker))
}
