//! SQLite/FTS5 persistence for the local core.
//!
//! Only this module owns SQLite connections.  Callers operate through
//! `Core`'s request/response methods, which is the boundary that a Tauri
//! command, CLI process, or MCP stdio server will use in later milestones.

use super::types::*;
use chrono::Utc;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Deref;
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

pub mod workflow;
pub mod workflow_activity;
pub mod workflow_dashboard;
pub mod workflow_links;

const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
        CREATE TABLE IF NOT EXISTS sources (
            id TEXT PRIMARY KEY NOT NULL,
            provider TEXT NOT NULL,
            account_id TEXT NOT NULL,
            external_id TEXT NOT NULL,
            url TEXT NOT NULL,
            kind TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(provider, account_id, external_id, kind)
        );

        CREATE TABLE IF NOT EXISTS snapshots (
            source_id TEXT PRIMARY KEY NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            title TEXT NOT NULL,
            body TEXT NOT NULL,
            source_updated_at TEXT,
            fetched_at TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            access_status TEXT NOT NULL DEFAULT 'accessible',
            unavailable_reason TEXT,
            is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1))
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS snapshot_fts USING fts5(
            source_id UNINDEXED,
            title,
            body,
            tokenize = 'unicode61'
        );

        CREATE TABLE IF NOT EXISTS relations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            from_source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            to_source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            relation_type TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(from_source_id, to_source_id, relation_type)
        );

        CREATE TABLE IF NOT EXISTS sync_states (
            connection_id TEXT PRIMARY KEY NOT NULL,
            provider TEXT NOT NULL,
            scope TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'never',
            last_success_at TEXT,
            last_attempt_at TEXT,
            failure_at TEXT,
            cursor TEXT,
            last_error_code TEXT,
            last_error TEXT,
            retry_at TEXT
        );

        CREATE TABLE IF NOT EXISTS memories (
            id TEXT PRIMARY KEY NOT NULL,
            body TEXT NOT NULL,
            reason TEXT NOT NULL,
            author TEXT NOT NULL,
            claim_type TEXT NOT NULL,
            version INTEGER NOT NULL CHECK (version > 0),
            idempotency_key TEXT NOT NULL,
            idempotency_digest TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            retracted_at TEXT,
            supersedes_id TEXT REFERENCES memories(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS memory_evidence (
            memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE RESTRICT,
            PRIMARY KEY(memory_id, source_id)
        );

        CREATE INDEX IF NOT EXISTS idx_snapshots_source_updated_at
            ON snapshots(source_updated_at);
        CREATE INDEX IF NOT EXISTS idx_snapshots_access_status
            ON snapshots(access_status, is_deleted);
        CREATE INDEX IF NOT EXISTS idx_memory_evidence_source_id
            ON memory_evidence(source_id);
        CREATE INDEX IF NOT EXISTS idx_relations_from_source
            ON relations(from_source_id);
        CREATE INDEX IF NOT EXISTS idx_relations_to_source
            ON relations(to_source_id);
        "#,
    ),
    (
        2,
        r#"
        CREATE TABLE IF NOT EXISTS memory_revisions (
            memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            version INTEGER NOT NULL,
            body TEXT NOT NULL,
            reason TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            author TEXT NOT NULL,
            claim_type TEXT NOT NULL,
            supersedes_id TEXT,
            action TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY(memory_id, version)
        );

        CREATE TABLE IF NOT EXISTS idempotency_records (
            idempotency_key TEXT PRIMARY KEY NOT NULL,
            operation TEXT NOT NULL,
            request_digest TEXT NOT NULL,
            response_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    ),
    (
        3,
        r#"
        ALTER TABLE snapshots ADD COLUMN metadata_json TEXT NOT NULL DEFAULT '{}';
        ALTER TABLE snapshots ADD COLUMN links_json TEXT NOT NULL DEFAULT '[]';
        "#,
    ),
    (
        4,
        r#"
        CREATE TABLE IF NOT EXISTS ui_memories (
            memory_id TEXT PRIMARY KEY NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            title TEXT NOT NULL,
            work INTEGER NOT NULL CHECK (work >= 0),
            kind TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ui_memories_work_updated
            ON ui_memories(work, updated_at DESC);
        "#,
    ),
    (
        5,
        r#"
        CREATE TABLE IF NOT EXISTS graph_builds (
            build_id TEXT PRIMARY KEY NOT NULL,
            scope_provider TEXT,
            scope_account_id TEXT,
            digest TEXT NOT NULL,
            created_at TEXT NOT NULL,
            node_count INTEGER NOT NULL DEFAULT 0,
            edge_count INTEGER NOT NULL DEFAULT 0,
            skipped_links INTEGER NOT NULL DEFAULT 0,
            ambiguous_links INTEGER NOT NULL DEFAULT 0,
            diagnostics_json TEXT NOT NULL DEFAULT '[]',
            is_current INTEGER NOT NULL DEFAULT 0 CHECK (is_current IN (0, 1))
        );

        CREATE TABLE IF NOT EXISTS graph_nodes (
            build_id TEXT NOT NULL REFERENCES graph_builds(build_id) ON DELETE CASCADE,
            source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            title TEXT NOT NULL,
            snapshot_hash TEXT NOT NULL,
            link_digest TEXT NOT NULL,
            access_status TEXT NOT NULL,
            is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1)),
            PRIMARY KEY(build_id, source_id)
        );

        CREATE TABLE IF NOT EXISTS graph_edges (
            id TEXT PRIMARY KEY NOT NULL,
            build_id TEXT NOT NULL REFERENCES graph_builds(build_id) ON DELETE CASCADE,
            from_source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            to_source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
            relation_type TEXT NOT NULL,
            provenance TEXT NOT NULL,
            evidence_location TEXT,
            confidence REAL,
            source_hash TEXT NOT NULL,
            target_hash TEXT,
            stale INTEGER NOT NULL DEFAULT 0 CHECK (stale IN (0, 1)),
            UNIQUE(build_id, from_source_id, to_source_id, relation_type, provenance)
        );

        CREATE INDEX IF NOT EXISTS idx_graph_builds_current
            ON graph_builds(is_current, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_graph_nodes_source
            ON graph_nodes(build_id, source_id);
        CREATE INDEX IF NOT EXISTS idx_graph_edges_from
            ON graph_edges(build_id, from_source_id, stale);
        CREATE INDEX IF NOT EXISTS idx_graph_edges_to
            ON graph_edges(build_id, to_source_id, stale);
        "#,
    ),
    (
        6,
        r#"
        ALTER TABLE graph_edges ADD COLUMN source_url TEXT NOT NULL DEFAULT '';
        ALTER TABLE graph_edges ADD COLUMN target_url TEXT;
        "#,
    ),
    (
        7,
        r#"
        CREATE TABLE IF NOT EXISTS observations (
            id TEXT PRIMARY KEY NOT NULL,
            session_id TEXT NOT NULL,
            body TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            actor TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'captured'
                CHECK (state IN ('captured', 'distilled')),
            version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
            idempotency_key TEXT NOT NULL,
            idempotency_digest TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS memory_candidates (
            id TEXT PRIMARY KEY NOT NULL,
            observation_id TEXT NOT NULL UNIQUE REFERENCES observations(id) ON DELETE CASCADE,
            body TEXT NOT NULL,
            reason TEXT NOT NULL,
            evidence_json TEXT NOT NULL,
            author TEXT NOT NULL,
            claim_type TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'distilled'
                CHECK (state IN ('distilled', 'proposed', 'accepted', 'rejected')),
            version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
            idempotency_key TEXT NOT NULL,
            idempotency_digest TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            accepted_memory_id TEXT REFERENCES memories(id) ON DELETE SET NULL,
            rejection_reason TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_observations_state_updated
            ON observations(state, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_candidates_state_updated
            ON memory_candidates(state, updated_at DESC);
        "#,
    ),
    (8, workflow::SCHEMA),
    (9, workflow_links::SCHEMA),
];

#[derive(Debug)]
pub struct Database {
    connection: Mutex<Connection>,
    path: Option<std::path::PathBuf>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> CoreResult<Self> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path).map_err(database_error)?;
        Self::from_connection(connection, None, Some(path))
    }

    pub fn in_memory() -> CoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(database_error)?;
        Self::from_connection(connection, None, None)
    }

    fn from_connection(
        mut connection: Connection,
        fail_at: Option<i64>,
        path: Option<std::path::PathBuf>,
    ) -> CoreResult<Self> {
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(database_error)?;
        migrate(&mut connection, fail_at)?;
        Ok(Self {
            connection: Mutex::new(connection),
            path,
        })
    }

    /// Test-only constructor used to prove that a failed migration does not
    /// leave a partially applied schema behind.
    #[cfg(test)]
    fn in_memory_with_migration_failure(version: i64) -> CoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(database_error)?;
        Self::from_connection(connection, Some(version), None)
    }

    fn lock(&self) -> CoreResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| CoreError::Database {
            message: "database mutex was poisoned".to_string(),
        })
    }

    pub fn ingest_snapshot(&self, input: Snapshot) -> CoreResult<IngestResult> {
        let snapshot = input.normalize()?;
        let source = snapshot.source.clone();
        let metadata_json =
            serde_json::to_string(&snapshot.metadata).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?;
        let links_json =
            serde_json::to_string(&snapshot.links).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let existing_id = find_source_id(&transaction, &source)?;
        let deduplicated = existing_id.is_some();
        let source_id = ensure_source(&transaction, &source)?;

        if snapshot.access_status.is_searchable() && !snapshot.is_deleted {
            transaction
                .execute(
                    "INSERT INTO snapshots
                        (source_id, title, body, source_updated_at, fetched_at, content_hash,
                         access_status, unavailable_reason, is_deleted, metadata_json, links_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10)
                     ON CONFLICT(source_id) DO UPDATE SET
                        title = excluded.title,
                        body = excluded.body,
                        source_updated_at = excluded.source_updated_at,
                        fetched_at = excluded.fetched_at,
                        content_hash = excluded.content_hash,
                        access_status = excluded.access_status,
                        unavailable_reason = excluded.unavailable_reason,
                        is_deleted = excluded.is_deleted,
                        metadata_json = excluded.metadata_json,
                        links_json = excluded.links_json",
                    params![
                        source_id,
                        snapshot.title,
                        snapshot.body,
                        snapshot.source_updated_at,
                        snapshot.fetched_at,
                        snapshot.content_hash,
                        snapshot.access_status.as_str(),
                        snapshot.unavailable_reason,
                        metadata_json,
                        links_json,
                    ],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "DELETE FROM snapshot_fts WHERE source_id = ?1",
                    params![source_id],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "INSERT INTO snapshot_fts(source_id, title, body) VALUES (?1, ?2, ?3)",
                    params![source_id, snapshot.title, snapshot.body],
                )
                .map_err(database_error)?;
        } else {
            // A failed refresh must not overwrite the last good body or its
            // fetched_at.  It only changes the access marker.  If this is the
            // first observation, an empty cache row is created for auditability.
            transaction
                .execute(
                    "INSERT INTO snapshots
                        (source_id, title, body, source_updated_at, fetched_at, content_hash,
                         access_status, unavailable_reason, is_deleted, metadata_json, links_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                     ON CONFLICT(source_id) DO UPDATE SET
                        access_status = excluded.access_status,
                        unavailable_reason = excluded.unavailable_reason,
                        is_deleted = excluded.is_deleted",
                    params![
                        source_id,
                        snapshot.title,
                        snapshot.body,
                        snapshot.source_updated_at,
                        snapshot.fetched_at,
                        snapshot.content_hash,
                        snapshot.access_status.as_str(),
                        snapshot.unavailable_reason,
                        snapshot.is_deleted as i64,
                        metadata_json,
                        links_json,
                    ],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "DELETE FROM snapshot_fts WHERE source_id = ?1",
                    params![source_id],
                )
                .map_err(database_error)?;
        }
        transaction.commit().map_err(database_error)?;

        Ok(IngestResult {
            source_id,
            deduplicated,
            indexed: snapshot.access_status.is_searchable() && !snapshot.is_deleted,
            access_status: snapshot.access_status,
        })
    }

    pub fn record_source_failure(
        &self,
        source: SourceRef,
        access_status: AccessStatus,
        reason: impl Into<String>,
        failed_at: Option<String>,
    ) -> CoreResult<IngestResult> {
        if matches!(access_status, AccessStatus::Accessible) {
            return Err(CoreError::InvalidInput {
                field: "access_status".to_string(),
                message: "record_source_failure requires an inaccessible status".to_string(),
            });
        }
        source.validate()?;
        let failed_at = failed_at.unwrap_or_else(now_rfc3339);
        let snapshot = Snapshot {
            source,
            title: String::new(),
            body: String::new(),
            source_updated_at: None,
            fetched_at: failed_at,
            content_hash: content_hash("", ""),
            access_status,
            unavailable_reason: Some(reason.into()),
            is_deleted: false,
            metadata: std::collections::BTreeMap::new(),
            links: Vec::new(),
        };
        self.ingest_snapshot(snapshot)
    }

    pub fn add_relation(&self, input: RelationInput) -> CoreResult<()> {
        input.from.validate()?;
        input.to.validate()?;
        if input.relation_type.trim().is_empty() || input.reason.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "relation".to_string(),
                message: "relation_type and reason are required".to_string(),
            });
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let from_id =
            find_source_id(&transaction, &input.from)?.ok_or_else(|| CoreError::NotFound {
                entity: "source".to_string(),
                id: input.from.id(),
            })?;
        let to_id =
            find_source_id(&transaction, &input.to)?.ok_or_else(|| CoreError::NotFound {
                entity: "source".to_string(),
                id: input.to.id(),
            })?;
        transaction
            .execute(
                "INSERT INTO relations(from_source_id, to_source_id, relation_type, reason, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(from_source_id, to_source_id, relation_type) DO UPDATE SET reason = excluded.reason",
                params![from_id, to_id, input.relation_type, input.reason, now_rfc3339()],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)
    }

    pub fn remove_relation(&self, input: RelationInput) -> CoreResult<bool> {
        input.from.validate()?;
        input.to.validate()?;
        if input.relation_type.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "relation_type".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let from_id =
            find_source_id(&transaction, &input.from)?.ok_or_else(|| CoreError::NotFound {
                entity: "source".to_string(),
                id: input.from.id(),
            })?;
        let to_id =
            find_source_id(&transaction, &input.to)?.ok_or_else(|| CoreError::NotFound {
                entity: "source".to_string(),
                id: input.to.id(),
            })?;
        let removed = transaction
            .execute(
                "DELETE FROM relations
                 WHERE from_source_id = ?1 AND to_source_id = ?2 AND relation_type = ?3",
                params![from_id, to_id, input.relation_type],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        Ok(removed > 0)
    }

    pub fn search(&self, request: SearchRequest) -> CoreResult<SearchResponse> {
        let limit = request.effective_limit()?;
        if let Some(value) = request.source_updated_after.as_deref() {
            parse_timestamp(value)?;
        }
        if let Some(value) = request.source_updated_before.as_deref() {
            parse_timestamp(value)?;
        }
        let connection = self.lock()?;
        let mut sql = String::from(
            "SELECT s.provider, s.account_id, s.external_id, s.url, s.kind,
                    sn.title, snippet(snapshot_fts, 2, '<mark>', '</mark>', '…', 20),
                    sn.source_updated_at, sn.fetched_at, sn.access_status
             FROM snapshot_fts
             JOIN snapshots sn ON sn.source_id = snapshot_fts.source_id
             JOIN sources s ON s.id = sn.source_id
             WHERE sn.access_status = 'accessible' AND sn.is_deleted = 0",
        );
        let mut values: Vec<Value> = Vec::new();
        let query = safe_fts_query(&request.query);
        if let Some(query) = query {
            sql.push_str(" AND snapshot_fts MATCH ?");
            values.push(Value::Text(query));
        }
        if let Some(provider) = request.provider {
            sql.push_str(" AND s.provider = ?");
            values.push(Value::Text(provider));
        }
        if let Some(kind) = request.kind {
            sql.push_str(" AND s.kind = ?");
            values.push(Value::Text(kind));
        }
        if let Some(after) = request.source_updated_after {
            sql.push_str(" AND sn.source_updated_at >= ?");
            values.push(Value::Text(after));
        }
        if let Some(before) = request.source_updated_before {
            sql.push_str(" AND sn.source_updated_at <= ?");
            values.push(Value::Text(before));
        }
        sql.push_str(" ORDER BY bm25(snapshot_fts), sn.fetched_at DESC LIMIT ?");
        values.push(Value::Integer(limit as i64));

        let mut statement = connection.prepare(&sql).map_err(database_error)?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                Ok((
                    SourceRef {
                        provider: row.get(0)?,
                        account_id: row.get(1)?,
                        external_id: row.get(2)?,
                        url: row.get(3)?,
                        kind: row.get(4)?,
                    },
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            })
            .map_err(database_error)?;
        let mut results = Vec::new();
        for row in rows {
            let (source, title, snippet, source_updated_at, fetched_at, access_status) =
                row.map_err(database_error)?;
            let access_status = AccessStatus::from_str(&access_status)?;
            results.push(SearchResult {
                source,
                title,
                snippet,
                source_updated_at,
                freshness: freshness(&access_status, &fetched_at, request.max_age_seconds)?,
                fetched_at,
                access_status,
            });
        }
        Ok(SearchResponse {
            results,
            limit,
            next_cursor: None,
        })
    }

    pub fn context(&self, request: ContextRequest) -> CoreResult<ContextResponse> {
        request.validate()?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let root_id = if let Some(source_id) = request.source_id {
            source_id
        } else {
            let source = request.source.expect("validated source or source_id");
            find_source_id(&transaction, &source)?.ok_or_else(|| CoreError::NotFound {
                entity: "source".to_string(),
                id: source.id(),
            })?
        };
        let _root = load_source(&transaction, &root_id)?;
        let mut source_ids = vec![root_id.clone()];
        let mut relation_statement = transaction
            .prepare(
                "SELECT from_source_id, to_source_id FROM relations
                 WHERE from_source_id = ?1 OR to_source_id = ?1",
            )
            .map_err(database_error)?;
        let related = relation_statement
            .query_map(params![root_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error)?;
        for relation in related {
            let (from, to) = relation.map_err(database_error)?;
            if !source_ids.contains(&from) {
                source_ids.push(from);
            }
            if !source_ids.contains(&to) {
                source_ids.push(to);
            }
        }
        drop(relation_statement);

        let mut sources = Vec::new();
        let mut unavailable_sources = Vec::new();
        let mut stale = false;
        let mut missing_providers = HashSet::new();
        for source_id in &source_ids {
            let (maybe_result, maybe_source) =
                load_source_result(&transaction, source_id, request.max_age_seconds)?;
            if let Some(result) = maybe_result {
                stale |= matches!(result.freshness, Freshness::Stale);
                sources.push(result);
            } else if let Some(source) = maybe_source {
                missing_providers.insert(source.provider.clone());
                unavailable_sources.push(source);
            }
        }

        let mut memory_ids = HashSet::new();
        let mut evidence_statement = transaction
            .prepare(
                "SELECT DISTINCT memory_id FROM memory_evidence
                 WHERE source_id IN (SELECT value FROM json_each(?1))",
            )
            .map_err(database_error)?;
        let source_ids_json =
            serde_json::to_string(&source_ids).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?;
        let memory_rows = evidence_statement
            .query_map(params![source_ids_json], |row| row.get::<_, String>(0))
            .map_err(database_error)?;
        for memory_id in memory_rows {
            memory_ids.insert(memory_id.map_err(database_error)?);
        }
        drop(evidence_statement);
        let mut memories = Vec::new();
        for memory_id in memory_ids {
            memories.push(load_memory(&transaction, &memory_id)?);
        }
        memories.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        transaction.commit().map_err(database_error)?;

        Ok(ContextResponse {
            context_id: root_id,
            sources,
            memories,
            unavailable_sources,
            stale,
            conflicts: Vec::new(),
            missing_providers: {
                let mut values = missing_providers.into_iter().collect::<Vec<_>>();
                values.sort();
                values
            },
        })
    }

    pub fn context_query(&self, request: ContextQueryRequest) -> CoreResult<ContextQueryResponse> {
        request.validate()?;
        let max_depth = request.max_depth.unwrap_or(1);
        let max_nodes = request.max_nodes.unwrap_or(50);
        let max_edges = request.max_edges.unwrap_or(100);
        let max_sources = request.max_sources.unwrap_or(DEFAULT_SEARCH_LIMIT);
        let max_memories = request.max_memories.unwrap_or(50);
        let sources = self.search(SearchRequest {
            query: request.query.clone(),
            provider: request.provider.clone(),
            kind: request.kind.clone(),
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: request.max_age_seconds,
            limit: Some(max_sources),
        })?;
        let explicit_root_id = if let Some(source_id) = request.source_id.clone() {
            Some(source_id)
        } else if let Some(source) = request.source.clone() {
            Some(source.id())
        } else {
            None
        };
        let graph_root_id = explicit_root_id
            .clone()
            .or_else(|| sources.results.first().map(|result| result.source.id()));
        let mut conflicts = Vec::new();
        let graph = if let Some(root_id) = graph_root_id {
            match self.graph_traverse(GraphTraversalRequest {
                source_id: Some(root_id),
                source: None,
                max_depth: Some(max_depth),
                max_nodes: Some(max_nodes),
                max_edges: Some(max_edges),
            }) {
                Ok(graph) => Some(filter_graph_response(
                    graph,
                    request.provider.as_deref(),
                    request.kind.as_deref(),
                )),
                Err(CoreError::NotFound { entity, id })
                    if entity == "graph build" || entity == "graph node" =>
                {
                    conflicts.push(format!("graph {entity} unavailable for {id}"));
                    None
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        let memories = self.search_memories(
            &request.query,
            explicit_root_id.as_deref(),
            request.provider.as_deref(),
            request.kind.as_deref(),
            max_memories,
        )?;
        let mut unavailable_sources = Vec::new();
        let mut stale_sources = sources
            .results
            .iter()
            .filter(|result| matches!(result.freshness, Freshness::Stale))
            .map(|result| result.source.clone())
            .collect::<Vec<_>>();
        if let Some(graph) = &graph {
            unavailable_sources.extend(graph.unavailable_sources.clone());
            stale_sources.extend(graph.stale_sources.clone());
        }
        for memory in &memories {
            for evidence in &memory.evidence {
                match self.snapshot(evidence) {
                    Ok(snapshot) => {
                        if !snapshot.access_status.is_searchable() || snapshot.is_deleted {
                            unavailable_sources.push(snapshot.source);
                        } else if matches!(
                            freshness(
                                &snapshot.access_status,
                                &snapshot.fetched_at,
                                request.max_age_seconds,
                            )?,
                            Freshness::Stale
                        ) {
                            stale_sources.push(snapshot.source);
                        }
                    }
                    Err(CoreError::NotFound { .. }) => unavailable_sources.push(evidence.clone()),
                    Err(error) => return Err(error),
                }
            }
        }
        unavailable_sources.sort_by_key(SourceRef::id);
        unavailable_sources.dedup_by(|left, right| left.id() == right.id());
        stale_sources.sort_by_key(SourceRef::id);
        stale_sources.dedup_by(|left, right| left.id() == right.id());
        let mut missing_providers = unavailable_sources
            .iter()
            .map(|source| source.provider.clone())
            .collect::<Vec<_>>();
        missing_providers.sort();
        missing_providers.dedup();
        let graph_truncated = graph.as_ref().is_some_and(|graph| graph.truncated);
        let source_truncated = sources.results.len() >= max_sources;
        let memory_truncated = memories.len() >= max_memories;
        Ok(ContextQueryResponse {
            api_version: "context.query.v1".to_string(),
            query: request.query,
            sources: sources.results,
            memories,
            graph,
            unavailable_sources,
            stale_sources,
            conflicts,
            missing_providers,
            bounds: ContextQueryBounds {
                max_depth,
                max_nodes,
                max_edges,
                max_sources,
                max_memories,
                truncated: graph_truncated || source_truncated || memory_truncated,
            },
        })
    }

    fn search_memories(
        &self,
        query: &str,
        root_id: Option<&str>,
        provider: Option<&str>,
        kind: Option<&str>,
        limit: usize,
    ) -> CoreResult<Vec<Memory>> {
        let connection = self.lock()?;
        let mut sql = String::from(
            "SELECT DISTINCT m.id FROM memories m
             WHERE m.retracted_at IS NULL",
        );
        let mut values = Vec::new();
        let trimmed = query.trim();
        if !trimmed.is_empty() {
            sql.push_str(" AND (m.body LIKE ? OR m.reason LIKE ? OR m.claim_type LIKE ?)");
            let value = Value::Text(format!("%{trimmed}%"));
            values.extend([value.clone(), value.clone(), value]);
        }
        if provider.is_some() || kind.is_some() || root_id.is_some() {
            sql.push_str(
                " AND EXISTS (
                    SELECT 1 FROM memory_evidence me
                    JOIN sources es ON es.id = me.source_id
                    WHERE me.memory_id = m.id",
            );
            if let Some(provider) = provider {
                sql.push_str(" AND es.provider = ?");
                values.push(Value::Text(provider.to_string()));
            }
            if let Some(kind) = kind {
                sql.push_str(" AND es.kind = ?");
                values.push(Value::Text(kind.to_string()));
            }
            if let Some(root_id) = root_id {
                sql.push_str(" AND es.id = ?");
                values.push(Value::Text(root_id.to_string()));
            }
            sql.push(')');
        }
        sql.push_str(" ORDER BY m.updated_at DESC, m.id LIMIT ?");
        values.push(Value::Integer(limit as i64));
        let mut statement = connection.prepare(&sql).map_err(database_error)?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                row.get::<_, String>(0)
            })
            .map_err(database_error)?;
        let ids = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        drop(statement);
        ids.into_iter()
            .map(|id| load_memory(&connection, &id))
            .collect()
    }

    pub fn record_sync_success(
        &self,
        connection_id: impl Into<String>,
        provider: impl Into<String>,
        scope: impl Into<String>,
        cursor: Option<String>,
        completed_at: Option<String>,
    ) -> CoreResult<SyncState> {
        let connection_id = connection_id.into();
        let provider = provider.into();
        let scope = scope.into();
        if connection_id.trim().is_empty() || provider.trim().is_empty() || scope.trim().is_empty()
        {
            return Err(CoreError::InvalidInput {
                field: "sync_state".to_string(),
                message: "connection_id, provider, and scope are required".to_string(),
            });
        }
        let completed_at = completed_at.unwrap_or_else(now_rfc3339);
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO sync_states
                    (connection_id, provider, scope, status, last_success_at, last_attempt_at,
                     failure_at, cursor, last_error_code, last_error, retry_at)
                 VALUES (?1, ?2, ?3, 'succeeded', ?4, ?4, NULL, ?5, NULL, NULL, NULL)
                 ON CONFLICT(connection_id) DO UPDATE SET
                    provider = excluded.provider,
                    scope = excluded.scope,
                    status = 'succeeded',
                    last_success_at = excluded.last_success_at,
                    last_attempt_at = excluded.last_attempt_at,
                    failure_at = NULL,
                    cursor = excluded.cursor,
                    last_error_code = NULL,
                    last_error = NULL,
                    retry_at = NULL",
                params![connection_id, provider, scope, completed_at, cursor],
            )
            .map_err(database_error)?;
        load_sync_state(&connection, &connection_id)
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
        let connection_id = connection_id.into();
        let provider = provider.into();
        let scope = scope.into();
        let failed_at = failed_at.unwrap_or_else(now_rfc3339);
        let error_code = error_code.into();
        let error = error.into();
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO sync_states
                    (connection_id, provider, scope, status, last_success_at, last_attempt_at,
                     failure_at, cursor, last_error_code, last_error, retry_at)
                 VALUES (?1, ?2, ?3, 'failed', NULL, ?4, ?4, NULL, ?5, ?6, ?7)
                 ON CONFLICT(connection_id) DO UPDATE SET
                    provider = excluded.provider,
                    scope = excluded.scope,
                    status = 'failed',
                    last_attempt_at = excluded.last_attempt_at,
                    failure_at = excluded.failure_at,
                    last_error_code = excluded.last_error_code,
                    last_error = excluded.last_error,
                    retry_at = excluded.retry_at",
                params![
                    connection_id,
                    provider,
                    scope,
                    failed_at,
                    error_code,
                    error,
                    retry_at
                ],
            )
            .map_err(database_error)?;
        load_sync_state(&connection, &connection_id)
    }

    pub fn sync_state(&self, connection_id: &str) -> CoreResult<SyncState> {
        let connection = self.lock()?;
        load_sync_state(&connection, connection_id)
    }

    /// Cached, non-deleted sources in one explicitly selected namespace.
    pub fn cached_sources(&self, provider: &str, account_id: &str) -> CoreResult<Vec<SourceRef>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT s.provider, s.account_id, s.external_id, s.url, s.kind FROM sources s
             JOIN snapshots sn ON sn.source_id = s.id
             WHERE s.provider = ?1 AND s.account_id = ?2 AND sn.is_deleted = 0",
            )
            .map_err(database_error)?;
        let sources = statement
            .query_map(params![provider, account_id], |row| {
                Ok(SourceRef {
                    provider: row.get(0)?,
                    account_id: row.get(1)?,
                    external_id: row.get(2)?,
                    url: row.get(3)?,
                    kind: row.get(4)?,
                })
            })
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        Ok(sources)
    }

    pub fn snapshot(&self, source: &SourceRef) -> CoreResult<Snapshot> {
        source.validate()?;
        let connection = self.lock()?;
        let row = connection
            .query_row(
                "SELECT s.provider, s.account_id, s.external_id, s.url, s.kind,
                        sn.title, sn.body, sn.source_updated_at, sn.fetched_at,
                        sn.content_hash, sn.access_status, sn.unavailable_reason, sn.is_deleted,
                        sn.metadata_json, sn.links_json
                 FROM sources s JOIN snapshots sn ON sn.source_id = s.id
                 WHERE s.provider = ?1 AND s.account_id = ?2 AND s.external_id = ?3 AND s.kind = ?4",
                params![source.provider, source.account_id, source.external_id, source.kind],
                |row| {
                    Ok((
                        SourceRef {
                            provider: row.get(0)?,
                            account_id: row.get(1)?,
                            external_id: row.get(2)?,
                            url: row.get(3)?,
                            kind: row.get(4)?,
                        },
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, i64>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, String>(14)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| CoreError::NotFound {
                entity: "snapshot".to_string(),
                id: source.id(),
            })?;
        Ok(Snapshot {
            source: row.0,
            title: row.1,
            body: row.2,
            source_updated_at: row.3,
            fetched_at: row.4,
            content_hash: row.5,
            access_status: AccessStatus::from_str(&row.6)?,
            unavailable_reason: row.7,
            is_deleted: row.8 != 0,
            metadata: serde_json::from_str(&row.9).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?,
            links: serde_json::from_str(&row.10).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?,
        })
    }

    pub fn capture_observation(
        &self,
        input: ObservationCaptureInput,
    ) -> CoreResult<ObservationMutation> {
        validate_observation_capture(&input)?;
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        if let Some(replay) = check_idempotency::<ObservationMutation>(
            &transaction,
            &input.idempotency_key,
            "observation.capture",
            &digest,
        )? {
            transaction.commit().map_err(database_error)?;
            return Ok(replay);
        }
        let id = input
            .id
            .clone()
            .unwrap_or_else(|| format!("obs_{}", Uuid::new_v4()));
        let already_exists = transaction
            .query_row(
                "SELECT 1 FROM observations WHERE id = ?1",
                params![id],
                |_| Ok(()),
            )
            .optional()
            .map_err(database_error)?
            .is_some();
        if already_exists {
            return Err(CoreError::InvalidInput {
                field: "id".to_string(),
                message: "an observation with this id already exists".to_string(),
            });
        }
        let evidence_json =
            serde_json::to_string(&input.evidence).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?;
        let now = now_rfc3339();
        transaction
            .execute(
                "INSERT INTO observations
                    (id, session_id, body, evidence_json, actor, state, version,
                     idempotency_key, idempotency_digest, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'captured', 1, ?6, ?7, ?8, ?8)",
                params![
                    id,
                    input.session_id,
                    input.body,
                    evidence_json,
                    input.actor,
                    input.idempotency_key,
                    digest,
                    now,
                ],
            )
            .map_err(database_error)?;
        let observation = load_observation(&transaction, &id)?;
        let mutation = ObservationMutation {
            observation,
            idempotent_replay: false,
            action: "captured".to_string(),
        };
        store_idempotency(
            &transaction,
            &input.idempotency_key,
            "observation.capture",
            &digest,
            &mutation,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(mutation)
    }

    pub fn distill_candidate(&self, input: CandidateDistillInput) -> CoreResult<CandidateMutation> {
        validate_candidate_distill(&input)?;
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        if let Some(replay) = check_idempotency::<CandidateMutation>(
            &transaction,
            &input.idempotency_key,
            "candidate.distill",
            &digest,
        )? {
            transaction.commit().map_err(database_error)?;
            return Ok(replay);
        }
        let observation = load_observation(&transaction, &input.observation_id)?;
        let expected = input.expected_version.unwrap_or(observation.version);
        if observation.version != expected {
            return Err(CoreError::VersionConflict {
                entity: "observation".to_string(),
                id: observation.id.clone(),
                expected,
                actual: observation.version,
            });
        }
        if observation.state != CandidateState::Captured {
            return Err(CoreError::InvalidInput {
                field: "observation_id".to_string(),
                message: "only a captured observation can be distilled".to_string(),
            });
        }
        let existing_candidate = transaction
            .query_row(
                "SELECT id FROM memory_candidates WHERE observation_id = ?1",
                params![observation.id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?;
        if let Some(id) = existing_candidate {
            return Err(CoreError::InvalidInput {
                field: "observation_id".to_string(),
                message: format!("observation already has candidate '{id}'"),
            });
        }
        let evidence_ids = source_ids_for_memory(&transaction, &observation.evidence)?;
        let evidence_json =
            serde_json::to_string(&observation.evidence).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?;
        let candidate_id = format!("cand_{}", Uuid::new_v4());
        let now = now_rfc3339();
        transaction
            .execute(
                "INSERT INTO memory_candidates
                    (id, observation_id, body, reason, evidence_json, author, claim_type,
                     state, version, idempotency_key, idempotency_digest, created_at,
                     updated_at, accepted_memory_id, rejection_reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'distilled', 1, ?8, ?9, ?10, ?10, NULL, NULL)",
                params![
                    candidate_id,
                    observation.id,
                    input.body,
                    input.reason,
                    evidence_json,
                    input.author,
                    input.claim_type,
                    input.idempotency_key,
                    digest,
                    now,
                ],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "UPDATE observations SET state = 'distilled', version = version + 1,
                    idempotency_key = ?1, idempotency_digest = ?2, updated_at = ?3
                 WHERE id = ?4",
                params![input.idempotency_key, digest, now, observation.id],
            )
            .map_err(database_error)?;
        let _ = evidence_ids;
        let candidate = load_candidate(&transaction, &candidate_id)?;
        let mutation = CandidateMutation {
            candidate,
            idempotent_replay: false,
            action: "distilled".to_string(),
        };
        store_idempotency(
            &transaction,
            &input.idempotency_key,
            "candidate.distill",
            &digest,
            &mutation,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(mutation)
    }

    pub fn propose_candidate(&self, input: CandidateProposeInput) -> CoreResult<CandidateMutation> {
        validate_candidate_transition(&input.id, input.expected_version, &input.idempotency_key)?;
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        if let Some(replay) = check_idempotency::<CandidateMutation>(
            &transaction,
            &input.idempotency_key,
            "candidate.propose",
            &digest,
        )? {
            transaction.commit().map_err(database_error)?;
            return Ok(replay);
        }
        let candidate = load_candidate(&transaction, &input.id)?;
        check_candidate_version(&candidate, input.expected_version)?;
        if candidate.state != CandidateState::Distilled {
            return Err(CoreError::InvalidInput {
                field: "id".to_string(),
                message: "only a distilled candidate can be proposed".to_string(),
            });
        }
        let now = now_rfc3339();
        transaction
            .execute(
                "UPDATE memory_candidates SET state = 'proposed', version = version + 1,
                    idempotency_key = ?1, idempotency_digest = ?2, updated_at = ?3
                 WHERE id = ?4",
                params![input.idempotency_key, digest, now, input.id],
            )
            .map_err(database_error)?;
        let candidate = load_candidate(&transaction, &input.id)?;
        let mutation = CandidateMutation {
            candidate,
            idempotent_replay: false,
            action: "proposed".to_string(),
        };
        store_idempotency(
            &transaction,
            &input.idempotency_key,
            "candidate.propose",
            &digest,
            &mutation,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(mutation)
    }

    pub fn accept_candidate(&self, input: CandidateAcceptInput) -> CoreResult<CandidateAcceptance> {
        validate_candidate_transition(&input.id, input.expected_version, &input.idempotency_key)?;
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        if let Some(replay) = check_idempotency::<CandidateAcceptance>(
            &transaction,
            &input.idempotency_key,
            "candidate.accept",
            &digest,
        )? {
            transaction.commit().map_err(database_error)?;
            return Ok(replay);
        }
        let candidate = load_candidate(&transaction, &input.id)?;
        check_candidate_version(&candidate, input.expected_version)?;
        if candidate.state != CandidateState::Proposed {
            return Err(CoreError::InvalidInput {
                field: "id".to_string(),
                message: "only a proposed candidate can be accepted".to_string(),
            });
        }
        let evidence_ids = source_ids_for_memory(&transaction, &candidate.evidence)?;
        let memory_input = MemoryUpsertInput {
            id: input.memory_id,
            body: candidate.body.clone(),
            reason: candidate.reason.clone(),
            evidence: candidate.evidence.clone(),
            author: candidate.author.clone(),
            claim_type: candidate.claim_type.clone(),
            idempotency_key: format!("candidate:{}:{}", candidate.id, input.idempotency_key),
            expected_version: input.expected_memory_version,
            supersedes_id: None,
        };
        let memory = Self::upsert_memory_in_transaction(&transaction, memory_input)?;
        let now = now_rfc3339();
        transaction
            .execute(
                "UPDATE memory_candidates SET state = 'accepted', version = version + 1,
                    idempotency_key = ?1, idempotency_digest = ?2, updated_at = ?3,
                    accepted_memory_id = ?4, rejection_reason = NULL
                 WHERE id = ?5",
                params![
                    input.idempotency_key,
                    digest,
                    now,
                    memory.memory.id,
                    input.id,
                ],
            )
            .map_err(database_error)?;
        let _ = evidence_ids;
        let candidate = load_candidate(&transaction, &input.id)?;
        let acceptance = CandidateAcceptance {
            candidate,
            memory: memory.memory,
            idempotent_replay: false,
        };
        store_idempotency(
            &transaction,
            &input.idempotency_key,
            "candidate.accept",
            &digest,
            &acceptance,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(acceptance)
    }

    pub fn reject_candidate(&self, input: CandidateRejectInput) -> CoreResult<CandidateMutation> {
        validate_candidate_transition(&input.id, input.expected_version, &input.idempotency_key)?;
        if input
            .reason
            .as_deref()
            .is_some_and(|reason| reason.trim().is_empty())
        {
            return Err(CoreError::InvalidInput {
                field: "reason".to_string(),
                message: "must not be empty when supplied".to_string(),
            });
        }
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        if let Some(replay) = check_idempotency::<CandidateMutation>(
            &transaction,
            &input.idempotency_key,
            "candidate.reject",
            &digest,
        )? {
            transaction.commit().map_err(database_error)?;
            return Ok(replay);
        }
        let candidate = load_candidate(&transaction, &input.id)?;
        check_candidate_version(&candidate, input.expected_version)?;
        if candidate.state != CandidateState::Proposed {
            return Err(CoreError::InvalidInput {
                field: "id".to_string(),
                message: "only a proposed candidate can be rejected".to_string(),
            });
        }
        let now = now_rfc3339();
        let reason = input
            .reason
            .unwrap_or_else(|| "rejected during review".to_string());
        transaction
            .execute(
                "UPDATE memory_candidates SET state = 'rejected', version = version + 1,
                    idempotency_key = ?1, idempotency_digest = ?2, updated_at = ?3,
                    rejection_reason = ?4 WHERE id = ?5",
                params![input.idempotency_key, digest, now, reason, input.id],
            )
            .map_err(database_error)?;
        let candidate = load_candidate(&transaction, &input.id)?;
        let mutation = CandidateMutation {
            candidate,
            idempotent_replay: false,
            action: "rejected".to_string(),
        };
        store_idempotency(
            &transaction,
            &input.idempotency_key,
            "candidate.reject",
            &digest,
            &mutation,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(mutation)
    }

    pub fn observation(&self, id: &str) -> CoreResult<Observation> {
        let connection = self.lock()?;
        load_observation(&connection, id)
    }

    pub fn candidate(&self, id: &str) -> CoreResult<MemoryCandidate> {
        let connection = self.lock()?;
        load_candidate(&connection, id)
    }

    pub fn candidates(&self, state: Option<CandidateState>) -> CoreResult<Vec<MemoryCandidate>> {
        let connection = self.lock()?;
        let mut statement = if state.is_some() {
            connection
                .prepare(
                    "SELECT id FROM memory_candidates WHERE state = ?1
                     ORDER BY updated_at DESC, id",
                )
                .map_err(database_error)?
        } else {
            connection
                .prepare("SELECT id FROM memory_candidates ORDER BY updated_at DESC, id")
                .map_err(database_error)?
        };
        let ids = if let Some(state) = state {
            statement
                .query_map(params![state.as_str()], |row| row.get::<_, String>(0))
                .map_err(database_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        } else {
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(database_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        };
        drop(statement);
        ids.into_iter()
            .map(|id| load_candidate(&connection, &id))
            .collect()
    }

    /// Rebuilds the derived document graph from the current snapshot cache.
    /// The graph is deliberately a separate projection: explicit relations
    /// remain canonical, while extracted links are rebuilt from snapshot
    /// content and carry their own hashes and build ID.
    pub fn rebuild_graph(&self, request: GraphRebuildRequest) -> CoreResult<GraphRebuildResponse> {
        request.validate()?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;

        let mut sql = String::from(
            "SELECT s.id, s.provider, s.account_id, s.external_id, s.url, s.kind,
                    sn.title, sn.body, sn.content_hash, sn.access_status,
                    sn.is_deleted, sn.links_json
             FROM sources s JOIN snapshots sn ON sn.source_id = s.id",
        );
        let mut values = Vec::new();
        let mut clauses = Vec::new();
        if let Some(provider) = request.provider.as_deref() {
            clauses.push("s.provider = ?".to_string());
            values.push(Value::Text(provider.to_string()));
        }
        if let Some(account_id) = request.account_id.as_deref() {
            clauses.push("s.account_id = ?".to_string());
            values.push(Value::Text(account_id.to_string()));
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY s.id");
        let mut statement = transaction.prepare(&sql).map_err(database_error)?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                let links_json: String = row.get(11)?;
                let links =
                    serde_json::from_str::<Vec<SourceLink>>(&links_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            11,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                Ok(GraphSnapshotRow {
                    source_id: row.get(0)?,
                    source: SourceRef {
                        provider: row.get(1)?,
                        account_id: row.get(2)?,
                        external_id: row.get(3)?,
                        url: row.get(4)?,
                        kind: row.get(5)?,
                    },
                    title: row.get(6)?,
                    body: row.get(7)?,
                    content_hash: row.get(8)?,
                    access_status: row.get(9)?,
                    is_deleted: row.get::<_, i64>(10)? != 0,
                    links,
                })
            })
            .map_err(database_error)?;
        let mut snapshots = Vec::new();
        for row in rows {
            snapshots.push(row.map_err(database_error)?);
        }
        drop(statement);

        let mut link_sets = Vec::with_capacity(snapshots.len());
        for snapshot in &snapshots {
            link_sets.push(extract_graph_links(&snapshot.body, &snapshot.links));
        }
        let mut digest_parts = Vec::with_capacity(snapshots.len() + 2);
        digest_parts.push(format!(
            "provider={};account={}",
            request.provider.as_deref().unwrap_or("*"),
            request.account_id.as_deref().unwrap_or("*")
        ));
        for (snapshot, links) in snapshots.iter().zip(&link_sets) {
            digest_parts.push(format!(
                "{}|{}|{}|{}|{}|{}|{}",
                snapshot.source_id,
                snapshot.source.provider,
                snapshot.source.account_id,
                snapshot.source.url,
                snapshot.content_hash,
                graph_link_digest(links),
                snapshot.access_status,
            ));
            if snapshot.is_deleted {
                digest_parts.push(format!("{}|deleted", snapshot.source_id));
            }
        }
        let mut digest_relation_statement = transaction
            .prepare(
                "SELECT from_source_id, to_source_id, relation_type, reason
                 FROM relations ORDER BY from_source_id, to_source_id, relation_type",
            )
            .map_err(database_error)?;
        let digest_relations = digest_relation_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(database_error)?;
        for relation in digest_relations {
            let (from, to, relation_type, reason) = relation.map_err(database_error)?;
            digest_parts.push(format!("relation|{from}|{to}|{relation_type}|{reason}"));
        }
        drop(digest_relation_statement);
        let digest = sha256_hex(digest_parts.join("\n").as_bytes());
        let build_id = format!("graph_{}", &digest[..24]);
        let created_at = now_rfc3339();
        let scope_provider = request.provider.clone();
        let scope_account_id = request.account_id.clone();

        transaction
            .execute(
                "UPDATE graph_builds SET is_current = 0 WHERE is_current = 1",
                [],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO graph_builds
                    (build_id, scope_provider, scope_account_id, digest, created_at,
                     node_count, edge_count, skipped_links, ambiguous_links,
                     diagnostics_json, is_current)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0, 0, '[]', 1)
                 ON CONFLICT(build_id) DO UPDATE SET
                    scope_provider = excluded.scope_provider,
                    scope_account_id = excluded.scope_account_id,
                    digest = excluded.digest,
                    created_at = excluded.created_at,
                    node_count = 0,
                    edge_count = 0,
                    skipped_links = 0,
                    ambiguous_links = 0,
                    diagnostics_json = '[]',
                    is_current = 1",
                params![
                    build_id,
                    scope_provider,
                    scope_account_id,
                    digest,
                    created_at,
                ],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "DELETE FROM graph_edges WHERE build_id = ?1",
                params![build_id],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "DELETE FROM graph_nodes WHERE build_id = ?1",
                params![build_id],
            )
            .map_err(database_error)?;

        let mut diagnostics = Vec::new();
        for snapshot in &snapshots {
            let links = extract_graph_links(&snapshot.body, &snapshot.links);
            transaction
                .execute(
                    "INSERT INTO graph_nodes
                        (build_id, source_id, title, snapshot_hash, link_digest,
                         access_status, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        build_id,
                        snapshot.source_id,
                        snapshot.title,
                        snapshot.content_hash,
                        graph_link_digest(&links),
                        snapshot.access_status,
                        snapshot.is_deleted as i64,
                    ],
                )
                .map_err(database_error)?;
        }

        let rows_by_id = snapshots
            .iter()
            .map(|snapshot| (snapshot.source_id.clone(), snapshot))
            .collect::<HashMap<_, _>>();
        let mut unique_urls: HashMap<&str, Vec<&GraphSnapshotRow>> = HashMap::new();
        for snapshot in &snapshots {
            unique_urls
                .entry(snapshot.source.url.as_str())
                .or_default()
                .push(snapshot);
        }
        let mut by_namespace: HashMap<(&str, &str, String), Vec<&GraphSnapshotRow>> =
            HashMap::new();
        for snapshot in &snapshots {
            by_namespace
                .entry((
                    snapshot.source.provider.as_str(),
                    snapshot.source.account_id.as_str(),
                    normalize_path(&snapshot.source.external_id),
                ))
                .or_default()
                .push(snapshot);
        }

        let mut skipped_links = 0usize;
        let mut ambiguous_links = 0usize;
        for (snapshot, links) in snapshots.iter().zip(&link_sets) {
            if snapshot.access_status != "accessible" || snapshot.is_deleted {
                continue;
            }
            for link in links {
                let kind = if link.kind.trim().is_empty() {
                    "reference"
                } else {
                    link.kind.as_str()
                };
                let candidates = if matches!(kind, "url" | "reference")
                    && (link.target.starts_with("http://")
                        || link.target.starts_with("https://")
                        || link.target.starts_with("obsidian://"))
                {
                    unique_urls
                        .get(link.target.as_str())
                        .cloned()
                        .unwrap_or_default()
                } else {
                    let target = normalize_wikilink_target(&link.target);
                    by_namespace
                        .get(&(
                            snapshot.source.provider.as_str(),
                            snapshot.source.account_id.as_str(),
                            target,
                        ))
                        .cloned()
                        .unwrap_or_default()
                };
                if candidates.len() != 1 {
                    if candidates.is_empty() {
                        skipped_links += 1;
                        diagnostics.push(format!(
                            "unresolved graph link: {} -> {}",
                            snapshot.source.external_id, link.target
                        ));
                    } else {
                        ambiguous_links += 1;
                        diagnostics.push(format!(
                            "ambiguous graph link: {} -> {}",
                            snapshot.source.external_id, link.target
                        ));
                    }
                    continue;
                }
                let target = candidates[0];
                let edge_id = graph_edge_id(
                    &build_id,
                    &snapshot.source_id,
                    &target.source_id,
                    kind,
                    GraphProvenance::Extracted.as_str(),
                );
                let stale = target.access_status != "accessible" || target.is_deleted;
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO graph_edges
                            (id, build_id, from_source_id, to_source_id, relation_type,
                             provenance, evidence_location, confidence, source_hash,
                             target_hash, stale, source_url, target_url)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 1.0, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            edge_id,
                            build_id,
                            snapshot.source_id,
                            target.source_id,
                            kind,
                            GraphProvenance::Extracted.as_str(),
                            snapshot.content_hash,
                            target.content_hash,
                            stale as i64,
                            snapshot.source.url,
                            target.source.url,
                        ],
                    )
                    .map_err(database_error)?;
            }
        }

        let mut relation_statement = transaction
            .prepare(
                "SELECT r.from_source_id, r.to_source_id, r.relation_type, r.reason
                 FROM relations r ORDER BY r.from_source_id, r.to_source_id, r.relation_type",
            )
            .map_err(database_error)?;
        let relations = relation_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(database_error)?;
        let mut relation_values = Vec::new();
        for relation in relations {
            relation_values.push(relation.map_err(database_error)?);
        }
        drop(relation_statement);
        for (from_id, to_id, relation_type, reason) in relation_values {
            let (Some(from), Some(to)) = (rows_by_id.get(&from_id), rows_by_id.get(&to_id)) else {
                continue;
            };
            let edge_id = graph_edge_id(
                &build_id,
                &from_id,
                &to_id,
                &relation_type,
                GraphProvenance::Explicit.as_str(),
            );
            let stale = from.access_status != "accessible"
                || from.is_deleted
                || to.access_status != "accessible"
                || to.is_deleted;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO graph_edges
                        (id, build_id, from_source_id, to_source_id, relation_type,
                         provenance, evidence_location, confidence, source_hash,
                         target_hash, stale, source_url, target_url)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        edge_id,
                        build_id,
                        from_id,
                        to_id,
                        relation_type,
                        GraphProvenance::Explicit.as_str(),
                        reason,
                        from.content_hash,
                        to.content_hash,
                        stale as i64,
                        from.source.url,
                        to.source.url,
                    ],
                )
                .map_err(database_error)?;
        }

        let node_count = transaction
            .query_row(
                "SELECT COUNT(*) FROM graph_nodes WHERE build_id = ?1",
                params![build_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(database_error)? as usize;
        let edge_count = transaction
            .query_row(
                "SELECT COUNT(*) FROM graph_edges WHERE build_id = ?1 AND stale = 0",
                params![build_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(database_error)? as usize;
        let diagnostics_json =
            serde_json::to_string(&diagnostics).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?;
        transaction
            .execute(
                "UPDATE graph_builds SET node_count = ?1, edge_count = ?2,
                    skipped_links = ?3, ambiguous_links = ?4,
                    diagnostics_json = ?5 WHERE build_id = ?6",
                params![
                    node_count as i64,
                    edge_count as i64,
                    skipped_links as i64,
                    ambiguous_links as i64,
                    diagnostics_json,
                    build_id,
                ],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;

        Ok(GraphRebuildResponse {
            build: GraphBuild {
                build_id,
                digest,
                created_at,
                node_count,
                edge_count,
                skipped_links,
                ambiguous_links,
                is_current: true,
            },
            diagnostics,
        })
    }

    pub fn graph_traverse(
        &self,
        request: GraphTraversalRequest,
    ) -> CoreResult<GraphTraversalResponse> {
        request.validate()?;
        let max_depth = request.max_depth.unwrap_or(1);
        let max_nodes = request.max_nodes.unwrap_or(50);
        let max_edges = request.max_edges.unwrap_or(100);
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let root_id = if let Some(source_id) = request.source_id {
            source_id
        } else {
            let source = request.source.expect("validated source or source_id");
            find_source_id(&transaction, &source)?.ok_or_else(|| CoreError::NotFound {
                entity: "source".to_string(),
                id: source.id(),
            })?
        };
        let build = transaction
            .query_row(
                "SELECT build_id FROM graph_builds WHERE is_current = 1
                 ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| CoreError::NotFound {
                entity: "graph build".to_string(),
                id: "current".to_string(),
            })?;

        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut unavailable_sources = Vec::new();
        let mut stale_sources = Vec::new();
        let mut diagnostics = Vec::new();
        let mut visited = HashSet::new();
        let mut seen_edges = HashSet::new();
        let mut frontier = VecDeque::from([(root_id.clone(), 0usize)]);
        let mut truncated = false;
        while let Some((current_id, depth)) = frontier.pop_front() {
            if !visited.insert(current_id.clone()) {
                continue;
            }
            let node = load_graph_node(&transaction, &build, &current_id)?;
            let live = live_graph_snapshot(&transaction, &current_id)?;
            let Some((live_hash, live_link_digest, live_access, live_deleted)) = live else {
                stale_sources.push(node.source);
                continue;
            };
            if !live_access.is_searchable() || live_deleted {
                unavailable_sources.push(node.source);
                continue;
            }
            if node.snapshot_hash != live_hash
                || node.link_digest != live_link_digest
                || node.access_status != live_access
                || node.is_deleted != live_deleted
            {
                stale_sources.push(node.source);
                continue;
            }
            if nodes.len() >= max_nodes {
                truncated = true;
                break;
            }
            let current_source = node.source.clone();
            nodes.push(node);
            if depth >= max_depth {
                continue;
            }
            let mut statement = transaction
                .prepare(
                    "SELECT id, from_source_id, to_source_id, relation_type, provenance,
                            evidence_location, confidence, source_hash, target_hash, stale
                            , source_url, target_url
                     FROM graph_edges WHERE build_id = ?1 AND stale = 0
                       AND (from_source_id = ?2 OR to_source_id = ?2)
                     ORDER BY id",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map(params![build, current_id], |row| {
                    Ok(GraphEdgeRow {
                        id: row.get(0)?,
                        from_source_id: row.get(1)?,
                        to_source_id: row.get(2)?,
                        relation_type: row.get(3)?,
                        provenance: row.get(4)?,
                        evidence_location: row.get(5)?,
                        confidence: row.get(6)?,
                        source_hash: row.get(7)?,
                        target_hash: row.get(8)?,
                        stale: row.get::<_, i64>(9)? != 0,
                        source_url: row.get(10)?,
                        target_url: row.get(11)?,
                    })
                })
                .map_err(database_error)?;
            for row in rows {
                let row = row.map_err(database_error)?;
                if !seen_edges.insert(row.id.clone()) {
                    continue;
                }
                if edges.len() >= max_edges {
                    truncated = true;
                    break;
                }
                let neighbor_id = if row.from_source_id == current_id {
                    row.to_source_id.clone()
                } else {
                    row.from_source_id.clone()
                };
                let neighbor = load_graph_node(&transaction, &build, &neighbor_id)?;
                let neighbor_live = live_graph_snapshot(&transaction, &neighbor_id)?;
                let Some((neighbor_hash, neighbor_link_digest, neighbor_access, neighbor_deleted)) =
                    neighbor_live
                else {
                    stale_sources.push(neighbor.source);
                    continue;
                };
                if !neighbor_access.is_searchable() || neighbor_deleted {
                    if !unavailable_sources
                        .iter()
                        .any(|source| source.id() == neighbor.source.id())
                    {
                        unavailable_sources.push(neighbor.source);
                    }
                    continue;
                }
                if neighbor.snapshot_hash != neighbor_hash
                    || neighbor.link_digest != neighbor_link_digest
                    || neighbor.access_status != neighbor_access
                    || neighbor.is_deleted != neighbor_deleted
                {
                    stale_sources.push(neighbor.source);
                    continue;
                }
                let neighbor_edge_hash = if row.from_source_id == current_id {
                    row.target_hash.as_deref().unwrap_or_default()
                } else {
                    row.source_hash.as_str()
                };
                if neighbor_edge_hash != neighbor_hash {
                    stale_sources.push(neighbor.source);
                    continue;
                }
                let current_edge_hash = if row.from_source_id == current_id {
                    row.source_hash.as_str()
                } else {
                    row.target_hash.as_deref().unwrap_or_default()
                };
                if current_edge_hash != live_hash {
                    stale_sources.push(current_source.clone());
                    continue;
                }
                let edge = load_graph_edge(&transaction, &build, row)?;
                if edge.source_url != edge.from.url
                    || edge.target_url.as_deref() != Some(edge.to.url.as_str())
                {
                    stale_sources.push(current_source.clone());
                    stale_sources.push(neighbor.source.clone());
                    continue;
                }
                if edge.provenance == GraphProvenance::Explicit {
                    let still_present: bool = transaction
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM relations
                             WHERE from_source_id = ?1 AND to_source_id = ?2
                               AND relation_type = ?3 AND reason = ?4)",
                            params![
                                edge.from_source_id,
                                edge.to_source_id,
                                edge.relation_type,
                                edge.evidence_location.as_deref().unwrap_or("")
                            ],
                            |row| row.get(0),
                        )
                        .map_err(database_error)?;
                    if !still_present {
                        stale_sources.push(current_source.clone());
                        continue;
                    }
                }
                // Only a validated edge may make a neighbor reachable. Otherwise
                // a revoked relation or changed URL could still return its node.
                if !visited.contains(&neighbor_id)
                    && !frontier
                        .iter()
                        .any(|(source_id, _)| source_id == &neighbor_id)
                {
                    if nodes.len() + frontier.len() >= max_nodes {
                        truncated = true;
                        break;
                    }
                    frontier.push_back((neighbor_id.clone(), depth + 1));
                }
                edges.push(edge);
            }
            drop(statement);
        }
        stale_sources.sort_by_key(SourceRef::id);
        stale_sources.dedup_by(|left, right| left.id() == right.id());
        unavailable_sources.sort_by_key(SourceRef::id);
        unavailable_sources.dedup_by(|left, right| left.id() == right.id());
        if truncated {
            diagnostics.push("graph traversal bounds reached".to_string());
        }
        transaction.commit().map_err(database_error)?;
        Ok(GraphTraversalResponse {
            build_id: build,
            nodes,
            edges,
            unavailable_sources,
            stale_sources,
            diagnostics,
            truncated,
        })
    }

    pub fn upsert_memory(&self, input: MemoryUpsertInput) -> CoreResult<MemoryMutation> {
        validate_memory_input(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let mutation = Self::upsert_memory_in_transaction(&transaction, input)?;
        transaction.commit().map_err(database_error)?;
        Ok(mutation)
    }

    fn upsert_memory_in_transaction(
        transaction: &Transaction<'_>,
        input: MemoryUpsertInput,
    ) -> CoreResult<MemoryMutation> {
        validate_memory_input(&input)?;
        let digest = request_digest(&input)?;
        if let Some(replay) = check_idempotency::<MemoryMutation>(
            transaction,
            &input.idempotency_key,
            "memory.upsert",
            &digest,
        )? {
            return Ok(replay);
        }

        let (memory_id, created, version, created_at, previous) = if let Some(id) = &input.id {
            match load_memory(transaction, id) {
                Ok(current) => {
                    let expected =
                        input
                            .expected_version
                            .ok_or_else(|| CoreError::InvalidInput {
                                field: "expected_version".to_string(),
                                message: "is required when updating an existing memory".to_string(),
                            })?;
                    if expected != current.version {
                        return Err(CoreError::VersionConflict {
                            entity: "memory".to_string(),
                            id: id.clone(),
                            expected,
                            actual: current.version,
                        });
                    }
                    if current.retracted_at.is_some() {
                        return Err(CoreError::InvalidInput {
                            field: "id".to_string(),
                            message: "a retracted memory must be restored before editing"
                                .to_string(),
                        });
                    }
                    (
                        id.clone(),
                        false,
                        current.version + 1,
                        current.created_at.clone(),
                        Some(current),
                    )
                }
                Err(CoreError::NotFound { .. }) => {
                    if input.expected_version.unwrap_or(0) != 0 {
                        return Err(CoreError::InvalidInput {
                            field: "expected_version".to_string(),
                            message: "a new memory must use version 0 or omit expected_version"
                                .to_string(),
                        });
                    }
                    (id.clone(), true, 1, now_rfc3339(), None)
                }
                Err(error) => return Err(error),
            }
        } else {
            if input.expected_version.is_some_and(|version| version != 0) {
                return Err(CoreError::InvalidInput {
                    field: "expected_version".to_string(),
                    message: "a new memory must use version 0 or omit expected_version".to_string(),
                });
            }
            (
                format!("mem_{}", Uuid::new_v4()),
                true,
                1,
                now_rfc3339(),
                None,
            )
        };

        let evidence_ids = source_ids_for_memory(transaction, &input.evidence)?;
        let updated_at = now_rfc3339();
        if created {
            transaction
                .execute(
                    "INSERT INTO memories
                        (id, body, reason, author, claim_type, version, idempotency_key,
                         idempotency_digest, created_at, updated_at, retracted_at, supersedes_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11)",
                    params![
                        memory_id,
                        input.body,
                        input.reason,
                        input.author,
                        input.claim_type,
                        version,
                        input.idempotency_key,
                        digest,
                        created_at,
                        updated_at,
                        input.supersedes_id,
                    ],
                )
                .map_err(database_error)?;
        } else {
            transaction
                .execute(
                    "UPDATE memories SET body = ?1, reason = ?2, author = ?3, claim_type = ?4,
                        version = ?5, idempotency_key = ?6, idempotency_digest = ?7,
                        updated_at = ?8, supersedes_id = ?9
                     WHERE id = ?10",
                    params![
                        input.body,
                        input.reason,
                        input.author,
                        input.claim_type,
                        version,
                        input.idempotency_key,
                        digest,
                        updated_at,
                        input.supersedes_id,
                        memory_id,
                    ],
                )
                .map_err(database_error)?;
        }
        replace_memory_evidence(transaction, &memory_id, &evidence_ids)?;
        let memory = load_memory(transaction, &memory_id)?;
        insert_revision(
            transaction,
            &memory,
            if previous.is_some() {
                "updated"
            } else {
                "created"
            },
        )?;
        let mutation = MemoryMutation {
            memory,
            created,
            idempotent_replay: false,
            action: if previous.is_some() {
                "updated".to_string()
            } else {
                "created".to_string()
            },
        };
        store_idempotency(
            transaction,
            &input.idempotency_key,
            "memory.upsert",
            &digest,
            &mutation,
        )?;
        Ok(mutation)
    }

    pub fn retract_memory(&self, input: MemoryRetractInput) -> CoreResult<MemoryMutation> {
        validate_mutation_key(&input.id, &input.idempotency_key)?;
        if input.expected_version < 1 {
            return Err(CoreError::InvalidInput {
                field: "expected_version".to_string(),
                message: "must be greater than zero".to_string(),
            });
        }
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        if let Some(replay) = check_idempotency::<MemoryMutation>(
            &transaction,
            &input.idempotency_key,
            "memory.retract",
            &digest,
        )? {
            transaction.commit().map_err(database_error)?;
            return Ok(replay);
        }
        let current = load_memory(&transaction, &input.id)?;
        check_expected_version(&current, input.expected_version)?;
        if current.retracted_at.is_some() {
            return Err(CoreError::InvalidInput {
                field: "id".to_string(),
                message: "memory is already retracted".to_string(),
            });
        }
        let updated_at = now_rfc3339();
        transaction
            .execute(
                "UPDATE memories SET version = ?1, updated_at = ?2, retracted_at = ?2,
                    idempotency_key = ?3, idempotency_digest = ?4 WHERE id = ?5",
                params![
                    current.version + 1,
                    updated_at,
                    input.idempotency_key,
                    digest,
                    input.id,
                ],
            )
            .map_err(database_error)?;
        let memory = load_memory(&transaction, &input.id)?;
        insert_revision(&transaction, &memory, "retracted")?;
        let mutation = MemoryMutation {
            memory,
            created: false,
            idempotent_replay: false,
            action: "retracted".to_string(),
        };
        store_idempotency(
            &transaction,
            &input.idempotency_key,
            "memory.retract",
            &digest,
            &mutation,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(mutation)
    }

    pub fn restore_memory(&self, input: MemoryRestoreInput) -> CoreResult<MemoryMutation> {
        validate_mutation_key(&input.id, &input.idempotency_key)?;
        if input.expected_version < 1 || input.revision_version < 1 {
            return Err(CoreError::InvalidInput {
                field: "version".to_string(),
                message: "expected_version and revision_version must be greater than zero"
                    .to_string(),
            });
        }
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        if let Some(replay) = check_idempotency::<MemoryMutation>(
            &transaction,
            &input.idempotency_key,
            "memory.restore",
            &digest,
        )? {
            transaction.commit().map_err(database_error)?;
            return Ok(replay);
        }
        let current = load_memory(&transaction, &input.id)?;
        check_expected_version(&current, input.expected_version)?;
        let revision = load_revision(&transaction, &input.id, input.revision_version)?;
        let evidence_ids = source_ids_for_memory(&transaction, &revision.evidence)?;
        let updated_at = now_rfc3339();
        transaction
            .execute(
                "UPDATE memories SET body = ?1, reason = ?2, author = ?3, claim_type = ?4,
                    version = ?5, idempotency_key = ?6, idempotency_digest = ?7,
                    updated_at = ?8, retracted_at = NULL, supersedes_id = ?9 WHERE id = ?10",
                params![
                    revision.body,
                    revision.reason,
                    revision.author,
                    revision.claim_type,
                    current.version + 1,
                    input.idempotency_key,
                    digest,
                    updated_at,
                    revision.supersedes_id,
                    input.id,
                ],
            )
            .map_err(database_error)?;
        replace_memory_evidence(&transaction, &input.id, &evidence_ids)?;
        let memory = load_memory(&transaction, &input.id)?;
        insert_revision(
            &transaction,
            &memory,
            &format!("restored_from:{}", input.revision_version),
        )?;
        let mutation = MemoryMutation {
            memory,
            created: false,
            idempotent_replay: false,
            action: "restored".to_string(),
        };
        store_idempotency(
            &transaction,
            &input.idempotency_key,
            "memory.restore",
            &digest,
            &mutation,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(mutation)
    }

    pub fn ui_memory_upsert(&self, input: UiMemoryUpsertInput) -> CoreResult<UiMemoryMutation> {
        validate_ui_memory_input(&input)?;
        let UiMemoryUpsertInput {
            title,
            work,
            kind,
            memory,
        } = input;
        let mutation = self.upsert_memory(memory)?;
        let now = now_rfc3339();
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO ui_memories(memory_id, title, work, kind, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(memory_id) DO UPDATE SET
                    title = excluded.title,
                    work = excluded.work,
                    kind = excluded.kind,
                    updated_at = excluded.updated_at",
                params![mutation.memory.id, title, work, kind, now, now,],
            )
            .map_err(database_error)?;
        let ui_memory = load_ui_memory(&connection, &mutation.memory.id)?;
        Ok(UiMemoryMutation {
            memory: ui_memory,
            created: mutation.created,
            idempotent_replay: mutation.idempotent_replay,
            action: mutation.action,
        })
    }

    pub fn ui_memory_retract(&self, input: MemoryRetractInput) -> CoreResult<UiMemoryMutation> {
        let mutation = self.retract_memory(input)?;
        let connection = self.lock()?;
        Ok(UiMemoryMutation {
            memory: load_ui_memory(&connection, &mutation.memory.id)?,
            created: false,
            idempotent_replay: mutation.idempotent_replay,
            action: mutation.action,
        })
    }

    pub fn ui_memory_restore(&self, input: MemoryRestoreInput) -> CoreResult<UiMemoryMutation> {
        let mutation = self.restore_memory(input)?;
        let connection = self.lock()?;
        Ok(UiMemoryMutation {
            memory: load_ui_memory(&connection, &mutation.memory.id)?,
            created: false,
            idempotent_replay: mutation.idempotent_replay,
            action: mutation.action,
        })
    }

    pub fn ui_memories(&self) -> CoreResult<Vec<UiMemory>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT memory_id FROM ui_memories ORDER BY updated_at DESC")
            .map_err(database_error)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error)?;
        let ids = ids.collect::<Result<Vec<_>, _>>().map_err(database_error)?;
        drop(statement);
        let mut memories = Vec::new();
        for id in ids {
            memories.push(load_ui_memory(&connection, &id)?);
        }
        Ok(memories)
    }

    pub fn clear_cache(&self) -> CoreResult<CacheClearResult> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        let snapshots_removed = transaction
            .query_row("SELECT COUNT(*) FROM snapshots", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(database_error)? as usize;
        transaction
            .execute("DELETE FROM snapshot_fts", [])
            .map_err(database_error)?;
        transaction
            .execute("DELETE FROM snapshots", [])
            .map_err(database_error)?;
        transaction
            .execute("DELETE FROM graph_builds", [])
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        Ok(CacheClearResult {
            snapshots_removed,
            sources_preserved: true,
            memories_preserved: true,
        })
    }

    pub fn backup_to(&self, destination: impl AsRef<Path>) -> CoreResult<BackupResult> {
        let destination = destination.as_ref().to_path_buf();
        if destination.exists() {
            return Err(CoreError::InvalidInput {
                field: "backup_path".to_string(),
                message: "destination already exists; choose an explicit new path".to_string(),
            });
        }
        if let Some(parent) = destination.parent() {
            if !parent.exists() {
                return Err(CoreError::NotFound {
                    entity: "backup directory".to_string(),
                    id: parent.display().to_string(),
                });
            }
        }
        let temporary = destination.with_file_name(format!(
            ".{}.tmp-{}",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("aidebook-backup"),
            Uuid::new_v4()
        ));
        {
            let connection = self.lock()?;
            connection
                .execute(
                    "VACUUM INTO ?1",
                    params![temporary.to_string_lossy().to_string()],
                )
                .map_err(database_error)?;
        }
        let result = match validate_database_file(&temporary) {
            Ok(schema_version) => {
                std::fs::rename(&temporary, &destination).map_err(|error| CoreError::Database {
                    message: format!("could not finalize backup: {error}"),
                })?;
                BackupResult {
                    path: destination.display().to_string(),
                    schema_version,
                }
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        };
        Ok(result)
    }

    pub fn restore_from(&self, backup: impl AsRef<Path>) -> CoreResult<BackupResult> {
        let backup = backup.as_ref().to_path_buf();
        let target = self.path.clone().ok_or_else(|| CoreError::InvalidInput {
            field: "restore_path".to_string(),
            message: "an on-disk Core database is required for restore".to_string(),
        })?;
        if backup == target {
            return Err(CoreError::InvalidInput {
                field: "restore_path".to_string(),
                message: "backup and active database must be different paths".to_string(),
            });
        }
        let _backup_schema_version = validate_database_file(&backup)?;
        let temporary = target.with_file_name(format!(
            ".{}.restore-{}",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("aidebook"),
            Uuid::new_v4()
        ));
        std::fs::copy(&backup, &temporary).map_err(|error| CoreError::Database {
            message: format!("could not stage restore: {error}"),
        })?;
        let staged_schema_version = match open_database_connection(&temporary) {
            Ok(connection) => {
                drop(connection);
                validate_database_file(&temporary)
            }
            Err(error) => Err(error),
        };
        let schema_version = match staged_schema_version {
            Ok(version) => version,
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        };
        let old_path = target.with_file_name(format!(
            ".{}.before-restore-{}",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("aidebook"),
            Uuid::new_v4()
        ));
        let mut connection = self.lock()?;
        let replacement = Connection::open_in_memory().map_err(database_error)?;
        let old_connection = std::mem::replace(&mut *connection, replacement);
        drop(old_connection);
        let had_target = target.exists();
        if had_target {
            if let Err(error) = std::fs::rename(&target, &old_path) {
                let restored = open_database_connection(&target);
                if let Ok(restored) = restored {
                    *connection = restored;
                }
                let _ = std::fs::remove_file(&temporary);
                return Err(CoreError::Database {
                    message: format!("could not stage active database for restore: {error}"),
                });
            }
        }
        if let Err(error) = std::fs::rename(&temporary, &target) {
            if had_target {
                let _ = std::fs::rename(&old_path, &target);
            }
            if let Ok(restored) = open_database_connection(&target) {
                *connection = restored;
            }
            return Err(CoreError::Database {
                message: format!("could not install restored database: {error}"),
            });
        }
        match open_database_connection(&target) {
            Ok(restored) => {
                *connection = restored;
                drop(connection);
                if had_target {
                    let _ = std::fs::remove_file(&old_path);
                }
                Ok(BackupResult {
                    path: target.display().to_string(),
                    schema_version,
                })
            }
            Err(error) => {
                let failed_path = target.with_file_name(format!(
                    ".{}.failed-restore-{}",
                    target
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("aidebook"),
                    Uuid::new_v4()
                ));
                let _ = std::fs::rename(&target, &failed_path);
                if had_target {
                    let _ = std::fs::rename(&old_path, &target);
                }
                if let Ok(restored) = open_database_connection(&target) {
                    *connection = restored;
                }
                Err(error)
            }
        }
    }

    pub fn memory(&self, id: &str) -> CoreResult<Memory> {
        let connection = self.lock()?;
        load_memory(&connection, id)
    }

    pub fn memory_history(&self, id: &str) -> CoreResult<Vec<MemoryRevision>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT version, body, reason, evidence_json, author, claim_type,
                        supersedes_id, action, created_at
                 FROM memory_revisions WHERE memory_id = ?1 ORDER BY version",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(database_error)?;
        let mut revisions = Vec::new();
        for row in rows {
            let (
                version,
                body,
                reason,
                evidence_json,
                author,
                claim_type,
                supersedes_id,
                action,
                created_at,
            ) = row.map_err(database_error)?;
            let evidence =
                serde_json::from_str(&evidence_json).map_err(|error| CoreError::Database {
                    message: error.to_string(),
                })?;
            revisions.push(MemoryRevision {
                memory_id: id.to_string(),
                version,
                body,
                reason,
                evidence,
                author,
                claim_type,
                supersedes_id,
                action,
                created_at,
            });
        }
        if revisions.is_empty() {
            return Err(CoreError::NotFound {
                entity: "memory".to_string(),
                id: id.to_string(),
            });
        }
        Ok(revisions)
    }

    /// Export canonical memories as a deterministic, text-only Markdown
    /// exchange. The body is kept in an explicit editable fence; metadata and
    /// evidence remain readable while retaining a lossless JSON line for
    /// round-tripping SourceRef identity.
    pub fn memory_export_markdown(&self) -> CoreResult<String> {
        let ids = {
            let connection = self.lock()?;
            let mut statement = connection
                .prepare("SELECT id FROM memories ORDER BY id")
                .map_err(database_error)?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(database_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?;
            ids
        };
        let mut output =
            String::from("<!-- aidebook-memory-exchange:v1 -->\n# Aidebook Memory Exchange\n\n");
        for id in ids {
            let memory = self.memory(&id)?;
            let mut history = self.memory_history(&id)?;
            history.sort_by_key(|revision| revision.version);
            let mut evidence = memory.evidence.clone();
            evidence.sort_by_key(SourceRef::id);
            let retracted_at = serde_json::to_string(&memory.retracted_at).map_err(|error| {
                CoreError::Database {
                    message: error.to_string(),
                }
            })?;
            let supersedes_id = serde_json::to_string(&memory.supersedes_id).map_err(|error| {
                CoreError::Database {
                    message: error.to_string(),
                }
            })?;
            output.push_str(&format!("## Memory: {}\n", memory.id));
            append_markdown_scalar(&mut output, "id", &memory.id)?;
            output.push_str(&format!("- version: {}\n", memory.version));
            append_markdown_scalar(&mut output, "author", &memory.author)?;
            append_markdown_scalar(&mut output, "claim_type", &memory.claim_type)?;
            append_markdown_scalar(&mut output, "reason", &memory.reason)?;
            output.push_str(&format!("- retracted_at: {retracted_at}\n"));
            output.push_str(&format!("- supersedes_id: {supersedes_id}\n\n"));
            output.push_str("### Aidebook Evidence\n");
            for source in evidence {
                let mut value =
                    serde_json::to_value(&source).map_err(|error| CoreError::Database {
                        message: error.to_string(),
                    })?;
                if let Some(object) = value.as_object_mut() {
                    object.insert("id".to_string(), serde_json::Value::String(source.id()));
                }
                let encoded =
                    serde_json::to_string(&value).map_err(|error| CoreError::Database {
                        message: error.to_string(),
                    })?;
                output.push_str(&format!("- source: {encoded}\n"));
                output.push_str(&format!("  wikilink: {}\n", source_wikilink(&source)));
            }
            output.push_str("### Aidebook Body\n");
            let fence = markdown_fence(&memory.body);
            output.push_str(&format!("{fence}markdown\n"));
            // Keep the body byte-for-byte intact. The one newline below is a
            // delimiter before the closing fence, so the importer can remove
            // exactly that delimiter without trimming user Markdown.
            output.push_str(&memory.body);
            output.push('\n');
            output.push_str(&format!("{fence}\n"));
            output.push_str("### Aidebook Revisions\n");
            for mut revision in history {
                revision.evidence.sort_by_key(SourceRef::id);
                let encoded =
                    serde_json::to_string(&revision).map_err(|error| CoreError::Database {
                        message: error.to_string(),
                    })?;
                output.push_str(&format!("- revision: {encoded}\n"));
            }
            output.push('\n');
        }
        Ok(output)
    }

    /// Import only into proposed review candidates. Parsing and source
    /// validation happen before the transaction writes anything, and all
    /// candidate rows are committed together so a malformed document cannot
    /// leave a partial review queue.
    pub fn memory_import_markdown(
        &self,
        markdown: String,
    ) -> CoreResult<MemoryMarkdownImportResult> {
        let documents = parse_memory_exchange(&markdown)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        for document in &documents {
            for source in &document.evidence {
                source_ids_for_memory(&transaction, std::slice::from_ref(source))?;
            }
        }
        let mut candidates = Vec::with_capacity(documents.len());
        let mut imported = 0;
        let mut idempotent = 0;
        for document in documents {
            let digest = exchange_document_digest(&document)?;
            let candidate_id = format!("cand_import_{}", &digest[..24]);
            if transaction
                .query_row(
                    "SELECT 1 FROM memory_candidates WHERE id = ?1",
                    params![candidate_id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(database_error)?
                .is_some()
            {
                candidates.push(load_candidate(&transaction, &candidate_id)?);
                idempotent += 1;
                continue;
            }
            let observation_id = format!("obs_import_{}", &digest[..24]);
            let observation_key = format!("markdown-observation:{digest}");
            let candidate_key = format!("markdown-import:{digest}");
            let evidence_json =
                serde_json::to_string(&document.evidence).map_err(|error| CoreError::Database {
                    message: error.to_string(),
                })?;
            let now = now_rfc3339();
            transaction
                .execute(
                    "INSERT INTO observations
                        (id, session_id, body, evidence_json, actor, state, version,
                         idempotency_key, idempotency_digest, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, 'markdown-import', 'distilled', 2, ?5, ?6, ?7, ?7)",
                    params![
                        observation_id,
                        format!("markdown:{digest}"),
                        document.body.clone(),
                        evidence_json,
                        observation_key,
                        digest,
                        now,
                    ],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "INSERT INTO memory_candidates
                        (id, observation_id, body, reason, evidence_json, author, claim_type,
                         state, version, idempotency_key, idempotency_digest, created_at,
                         updated_at, accepted_memory_id, rejection_reason)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'proposed', 2, ?8, ?9, ?10, ?10, NULL, NULL)",
                    params![
                        candidate_id,
                        observation_id,
                        document.body,
                        document.reason,
                        serde_json::to_string(&document.evidence).map_err(|error| {
                            CoreError::Database {
                                message: error.to_string(),
                            }
                        })?,
                        document.author,
                        document.claim_type,
                        candidate_key,
                        digest,
                        now,
                    ],
                )
                .map_err(database_error)?;
            candidates.push(load_candidate(&transaction, &candidate_id)?);
            imported += 1;
        }
        transaction.commit().map_err(database_error)?;
        candidates.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(MemoryMarkdownImportResult {
            imported,
            idempotent,
            candidates,
        })
    }
}

fn migrate(connection: &mut Connection, fail_at: Option<i64>) -> CoreResult<()> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY NOT NULL,
                applied_at TEXT NOT NULL
            );",
        )
        .map_err(database_error)?;
    let current: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    for (version, sql) in MIGRATIONS
        .iter()
        .copied()
        .filter(|(version, _)| *version > current)
    {
        let transaction = connection.transaction().map_err(database_error)?;
        if fail_at == Some(version) {
            return Err(CoreError::Migration {
                version,
                message: "injected failure; transaction rolled back".to_string(),
            });
        }
        transaction
            .execute_batch(sql)
            .map_err(|error| CoreError::Migration {
                version,
                message: error.to_string(),
            })?;
        transaction
            .execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
                params![version, now_rfc3339()],
            )
            .map_err(|error| CoreError::Migration {
                version,
                message: error.to_string(),
            })?;
        transaction.commit().map_err(|error| CoreError::Migration {
            version,
            message: error.to_string(),
        })?;
    }
    Ok(())
}

fn open_database_connection(path: &Path) -> CoreResult<Connection> {
    let mut connection = Connection::open(path).map_err(database_error)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
        .map_err(database_error)?;
    migrate(&mut connection, None)?;
    Ok(connection)
}

fn validate_database_file(path: &Path) -> CoreResult<i64> {
    if !path.exists() {
        return Err(CoreError::NotFound {
            entity: "backup".to_string(),
            id: path.display().to_string(),
        });
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(database_error)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(database_error)?;
    if integrity != "ok" {
        return Err(CoreError::Database {
            message: format!("SQLite integrity check failed: {integrity}"),
        });
    }
    connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)
}

fn database_error(error: rusqlite::Error) -> CoreError {
    CoreError::Database {
        message: error.to_string(),
    }
}

#[derive(Debug)]
struct GraphSnapshotRow {
    source_id: String,
    source: SourceRef,
    title: String,
    body: String,
    content_hash: String,
    access_status: String,
    is_deleted: bool,
    links: Vec<SourceLink>,
}

#[derive(Debug)]
struct GraphEdgeRow {
    id: String,
    from_source_id: String,
    to_source_id: String,
    relation_type: String,
    provenance: String,
    evidence_location: Option<String>,
    confidence: Option<f64>,
    source_url: String,
    target_url: Option<String>,
    source_hash: String,
    target_hash: Option<String>,
    stale: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ImportedMemoryDocument {
    id: String,
    body: String,
    reason: String,
    author: String,
    claim_type: String,
    evidence: Vec<SourceRef>,
}

fn append_markdown_scalar(output: &mut String, key: &str, value: &str) -> CoreResult<()> {
    let encoded = serde_json::to_string(value).map_err(|error| CoreError::Database {
        message: error.to_string(),
    })?;
    output.push_str(&format!("- {key}: {encoded}\n"));
    Ok(())
}

fn source_wikilink(source: &SourceRef) -> String {
    let identity = format!(
        "{}/{}/{}",
        source.provider, source.account_id, source.external_id
    );
    format!("[[{}]]", identity.replace(']', "\\]"))
}

fn markdown_fence(body: &str) -> String {
    let longest = body
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.chars().all(|character| character == '~') {
                Some(trimmed.chars().count())
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);
    "~".repeat(longest.max(2) + 1)
}

fn exchange_document_digest(document: &ImportedMemoryDocument) -> CoreResult<String> {
    let encoded = serde_json::to_vec(document).map_err(|error| CoreError::Database {
        message: error.to_string(),
    })?;
    Ok(sha256_hex(&encoded))
}

fn markdown_import_error(field: &str, message: impl Into<String>) -> CoreError {
    CoreError::InvalidInput {
        field: field.to_string(),
        message: message.into(),
    }
}

#[derive(Clone, Copy)]
struct ExchangeLine {
    start: usize,
    end: usize,
    next: usize,
}

impl ExchangeLine {
    fn text<'a>(self, markdown: &'a str) -> &'a str {
        &markdown[self.start..self.end]
    }
}

#[derive(Clone, Copy)]
struct ExchangeFence {
    marker: u8,
    length: usize,
}

fn exchange_lines(markdown: &str) -> Vec<ExchangeLine> {
    let bytes = markdown.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let mut end = index;
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        lines.push(ExchangeLine {
            start,
            end,
            next: index + 1,
        });
        start = index + 1;
    }
    if start < bytes.len() {
        let mut end = bytes.len();
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        lines.push(ExchangeLine {
            start,
            end,
            next: bytes.len(),
        });
    }
    lines
}

fn exchange_fence_marker(line: &str) -> Option<ExchangeFence> {
    let trimmed = line.trim_start();
    let marker = *trimmed.as_bytes().first()?;
    if marker != b'~' && marker != b'`' {
        return None;
    }
    let length = trimmed
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    (length >= 3).then_some(ExchangeFence { marker, length })
}

fn exchange_fence_closes(line: &str, fence: ExchangeFence) -> bool {
    let trimmed = line.trim();
    let bytes = trimmed.as_bytes();
    if bytes.first().copied() != Some(fence.marker) {
        return false;
    }
    let length = bytes
        .iter()
        .take_while(|byte| **byte == fence.marker)
        .count();
    length >= fence.length && trimmed[length..].trim().is_empty()
}

fn top_level_memory_starts(lines: &[ExchangeLine], markdown: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut fence = None;
    for (index, line) in lines.iter().copied().enumerate() {
        let text = line.text(markdown);
        if let Some(active) = fence {
            if exchange_fence_closes(text, active) {
                fence = None;
            }
            continue;
        }
        if let Some(opening) = exchange_fence_marker(text) {
            fence = Some(opening);
            continue;
        }
        if text.starts_with("## Memory: ") {
            starts.push(index);
        }
    }
    starts
}

fn top_level_heading_position(
    lines: &[ExchangeLine],
    markdown: &str,
    heading: &str,
) -> Option<usize> {
    let mut fence = None;
    for (index, line) in lines.iter().copied().enumerate() {
        let text = line.text(markdown);
        if let Some(active) = fence {
            if exchange_fence_closes(text, active) {
                fence = None;
            }
            continue;
        }
        if let Some(opening) = exchange_fence_marker(text) {
            fence = Some(opening);
            continue;
        }
        if text == heading {
            return Some(index);
        }
    }
    None
}

fn parse_memory_exchange(markdown: &str) -> CoreResult<Vec<ImportedMemoryDocument>> {
    if markdown.len() > 2 * 1024 * 1024 {
        return Err(markdown_import_error(
            "markdown",
            "exchange text must not exceed 2 MiB",
        ));
    }
    let lines = exchange_lines(markdown);
    if lines
        .first()
        .is_none_or(|line| line.text(markdown) != "<!-- aidebook-memory-exchange:v1 -->")
    {
        return Err(markdown_import_error(
            "markdown",
            "the v1 exchange header is required",
        ));
    }
    let starts = top_level_memory_starts(&lines, markdown);
    if starts.is_empty() {
        return Err(markdown_import_error(
            "markdown",
            "at least one memory section is required",
        ));
    }
    let mut seen_ids = HashSet::new();
    let mut documents = Vec::with_capacity(starts.len());
    for (position, start) in starts.iter().enumerate() {
        let end = starts.get(position + 1).copied().unwrap_or(lines.len());
        let header_id = lines[*start]
            .text(markdown)
            .strip_prefix("## Memory: ")
            .unwrap_or_default()
            .trim();
        if header_id.is_empty() || header_id.chars().count() > 200 {
            return Err(markdown_import_error(
                "memory.id",
                "must be a non-empty value no longer than 200 characters",
            ));
        }
        if !seen_ids.insert(header_id.to_string()) {
            return Err(markdown_import_error(
                "memory.id",
                "duplicate memory sections are not allowed",
            ));
        }
        let section = &lines[*start + 1..end];
        let evidence_heading =
            top_level_heading_position(section, markdown, "### Aidebook Evidence")
                .ok_or_else(|| markdown_import_error("evidence", "evidence heading is required"))?;
        let body_heading = top_level_heading_position(section, markdown, "### Aidebook Body")
            .ok_or_else(|| markdown_import_error("body", "body heading is required"))?;
        if evidence_heading >= body_heading {
            return Err(markdown_import_error(
                "markdown",
                "evidence must appear before the body",
            ));
        }
        let mut metadata = HashMap::new();
        for line in &section[..evidence_heading] {
            let line = line.text(markdown);
            let Some(value) = line.strip_prefix("- ") else {
                if line.trim().is_empty() {
                    continue;
                }
                return Err(markdown_import_error(
                    "metadata",
                    "metadata must use `- key: JSON value` lines",
                ));
            };
            let Some((key, raw)) = value.split_once(": ") else {
                return Err(markdown_import_error(
                    "metadata",
                    "metadata line is malformed",
                ));
            };
            if metadata.insert(key.to_string(), raw.to_string()).is_some() {
                return Err(markdown_import_error(
                    "metadata",
                    format!("duplicate metadata key '{key}'"),
                ));
            }
        }
        let metadata_id = parse_required_string(&metadata, "id")?;
        if metadata_id != header_id {
            return Err(markdown_import_error(
                "memory.id",
                "header and metadata IDs must match",
            ));
        }
        let version = parse_required_i64(&metadata, "version")?;
        if version < 1 {
            return Err(markdown_import_error(
                "version",
                "must be greater than zero",
            ));
        }
        let author = parse_required_string(&metadata, "author")?;
        let claim_type = parse_required_string(&metadata, "claim_type")?;
        let reason = parse_required_string(&metadata, "reason")?;
        let _ = parse_optional_string(&metadata, "retracted_at")?;
        let _ = parse_optional_string(&metadata, "supersedes_id")?;
        let mut evidence = Vec::new();
        for line in &section[evidence_heading + 1..body_heading] {
            let line = line.text(markdown);
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("wikilink:") {
                continue;
            }
            let Some(raw) = trimmed.strip_prefix("- source: ") else {
                return Err(markdown_import_error(
                    "evidence",
                    "evidence must use exported source lines",
                ));
            };
            let mut value = serde_json::from_str::<serde_json::Value>(raw).map_err(|error| {
                markdown_import_error("evidence", format!("source JSON is malformed: {error}"))
            })?;
            let declared_id = value
                .as_object_mut()
                .and_then(|object| object.remove("id"))
                .and_then(|value| value.as_str().map(ToOwned::to_owned));
            let source = serde_json::from_value::<SourceRef>(value).map_err(|error| {
                markdown_import_error("evidence", format!("source is malformed: {error}"))
            })?;
            source.validate()?;
            if declared_id.is_some_and(|id| id != source.id()) {
                return Err(markdown_import_error(
                    "evidence",
                    "source identity does not match its SourceRef fields",
                ));
            }
            if evidence
                .iter()
                .any(|existing: &SourceRef| existing.id() == source.id())
            {
                return Err(markdown_import_error(
                    "evidence",
                    "duplicate source references are not allowed",
                ));
            }
            evidence.push(source);
        }
        if evidence.is_empty() {
            return Err(markdown_import_error(
                "evidence",
                "at least one evidence source is required",
            ));
        }
        let body_start = section
            .iter()
            .position(|line| {
                let line = line.text(markdown);
                line.starts_with('~') && line.ends_with("markdown")
            })
            .ok_or_else(|| markdown_import_error("body", "a markdown body fence is required"))?;
        if body_start <= body_heading {
            return Err(markdown_import_error("body", "body fence is malformed"));
        }
        let opening = section[body_start].text(markdown);
        let fence = opening.strip_suffix("markdown").unwrap_or_default();
        if fence.len() < 3 || !fence.chars().all(|character| character == '~') {
            return Err(markdown_import_error("body", "body fence is malformed"));
        }
        let closing = (body_start + 1..section.len())
            .find(|index| section[*index].text(markdown) == fence)
            .ok_or_else(|| markdown_import_error("body", "body fence is not closed"))?;
        let raw_body = &markdown[section[body_start].next..section[closing].start];
        let body = if let Some(body) = raw_body.strip_suffix("\r\n") {
            body.to_string()
        } else if let Some(body) = raw_body.strip_suffix('\n') {
            body.to_string()
        } else {
            return Err(markdown_import_error(
                "body",
                "body fence must start on a new line",
            ));
        };
        if body.trim().is_empty() {
            return Err(markdown_import_error("body", "must not be empty"));
        }
        if contains_credential_marker(&body) || contains_credential_marker(&reason) {
            return Err(CoreError::SensitiveDataRejected);
        }
        let revision_heading = section[closing + 1..]
            .iter()
            .position(|line| line.text(markdown) == "### Aidebook Revisions")
            .map(|position| closing + 1 + position)
            .unwrap_or(section.len());
        if revision_heading <= body_heading {
            return Err(markdown_import_error(
                "markdown",
                "revision heading must follow the body",
            ));
        }
        for line in &section[closing + 1..revision_heading] {
            if !line.text(markdown).trim().is_empty() {
                return Err(markdown_import_error(
                    "body",
                    "only blank lines may follow the body fence",
                ));
            }
        }
        for line in section.get(revision_heading + 1..).unwrap_or_default() {
            let trimmed = line.text(markdown).trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some(raw) = trimmed.strip_prefix("- revision: ") else {
                return Err(markdown_import_error(
                    "revisions",
                    "revision line is malformed",
                ));
            };
            serde_json::from_str::<serde_json::Value>(raw).map_err(|error| {
                markdown_import_error("revisions", format!("revision JSON is malformed: {error}"))
            })?;
        }
        documents.push(ImportedMemoryDocument {
            id: header_id.to_string(),
            body,
            reason,
            author,
            claim_type,
            evidence,
        });
    }
    Ok(documents)
}

fn parse_required_string(metadata: &HashMap<String, String>, key: &str) -> CoreResult<String> {
    let raw = metadata
        .get(key)
        .ok_or_else(|| markdown_import_error(key, "is required"))?;
    let value = serde_json::from_str::<String>(raw)
        .map_err(|error| markdown_import_error(key, format!("must be a JSON string: {error}")))?;
    if value.trim().is_empty() {
        return Err(markdown_import_error(key, "must not be empty"));
    }
    Ok(value)
}

fn parse_required_i64(metadata: &HashMap<String, String>, key: &str) -> CoreResult<i64> {
    let raw = metadata
        .get(key)
        .ok_or_else(|| markdown_import_error(key, "is required"))?;
    raw.parse::<i64>()
        .map_err(|error| markdown_import_error(key, format!("must be an integer: {error}")))
}

fn parse_optional_string(
    metadata: &HashMap<String, String>,
    key: &str,
) -> CoreResult<Option<String>> {
    let Some(raw) = metadata.get(key) else {
        return Ok(None);
    };
    if raw == "null" {
        return Ok(None);
    }
    let value = serde_json::from_str::<String>(raw).map_err(|error| {
        markdown_import_error(key, format!("must be a JSON string or null: {error}"))
    })?;
    Ok(Some(value))
}

fn filter_graph_response(
    mut response: GraphTraversalResponse,
    provider: Option<&str>,
    kind: Option<&str>,
) -> GraphTraversalResponse {
    if provider.is_none() && kind.is_none() {
        return response;
    }
    let matches_filter = |source: &SourceRef| {
        provider.map_or(true, |value| source.provider == value)
            && kind.map_or(true, |value| source.kind == value)
    };
    let allowed = response
        .nodes
        .iter()
        .filter(|node| matches_filter(&node.source))
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    response.nodes.retain(|node| allowed.contains(&node.id));
    response.edges.retain(|edge| {
        allowed.contains(&edge.from_source_id) && allowed.contains(&edge.to_source_id)
    });
    response
        .unavailable_sources
        .retain(|source| matches_filter(source));
    response
        .stale_sources
        .retain(|source| matches_filter(source));
    response
}

fn graph_link_digest(links: &[SourceLink]) -> String {
    let mut values = links
        .iter()
        .map(|link| format!("{}\0{}", link.kind.trim(), link.target.trim()))
        .collect::<Vec<_>>();
    values.sort();
    sha256_hex(values.join("\n").as_bytes())
}

fn graph_edge_id(
    build_id: &str,
    from_source_id: &str,
    to_source_id: &str,
    relation_type: &str,
    provenance: &str,
) -> String {
    let canonical =
        format!("{build_id}\0{from_source_id}\0{to_source_id}\0{relation_type}\0{provenance}");
    format!("gedge_{}", &sha256_hex(canonical.as_bytes())[..24])
}

fn normalize_path(value: &str) -> String {
    let value = value.trim().replace('\\', "/");
    let value = value.strip_prefix("./").unwrap_or(&value);
    let value = value.strip_prefix('/').unwrap_or(value);
    if value.ends_with(".md") {
        value.to_string()
    } else {
        format!("{value}.md")
    }
}

fn normalize_wikilink_target(value: &str) -> String {
    let mut value = value.trim();
    if let Some((target, _)) = value.split_once('|') {
        value = target.trim();
    }
    if let Some((target, _)) = value.split_once('#') {
        value = target.trim();
    }
    normalize_path(value)
}

fn extract_graph_links(body: &str, provided: &[SourceLink]) -> Vec<SourceLink> {
    let mut links = provided.to_vec();
    let bytes = body.as_bytes();
    let mut index = 0usize;
    while index + 3 < bytes.len() {
        if bytes[index] == b'[' && bytes[index + 1] == b'[' {
            if let Some(end) = body[index + 2..].find("]]") {
                let target = body[index + 2..index + 2 + end]
                    .split_once('|')
                    .map(|(target, _)| target)
                    .unwrap_or(&body[index + 2..index + 2 + end])
                    .split_once('#')
                    .map(|(target, _)| target)
                    .unwrap_or_else(|| {
                        body[index + 2..index + 2 + end]
                            .split_once('|')
                            .map(|(target, _)| target)
                            .unwrap_or(&body[index + 2..index + 2 + end])
                    })
                    .trim();
                if !target.is_empty() {
                    links.push(SourceLink {
                        target: target.to_string(),
                        kind: "wikilink".to_string(),
                    });
                }
                index += end + 4;
                continue;
            }
        }
        index += 1;
    }
    for raw in body.split_whitespace() {
        let target = raw.trim_matches(|character: char| {
            matches!(
                character,
                '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | '.'
            )
        });
        if target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("obsidian://")
        {
            links.push(SourceLink {
                target: target.to_string(),
                kind: "url".to_string(),
            });
        }
    }
    links.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.target.cmp(&right.target))
    });
    links.dedup_by(|left, right| left.kind == right.kind && left.target == right.target);
    links
}

fn load_graph_node(
    transaction: &Transaction<'_>,
    build_id: &str,
    source_id: &str,
) -> CoreResult<GraphNode> {
    transaction
        .query_row(
            "SELECT n.title, n.snapshot_hash, n.link_digest, n.access_status,
                    n.is_deleted, s.provider, s.account_id, s.external_id, s.url, s.kind
             FROM graph_nodes n JOIN sources s ON s.id = n.source_id
             WHERE n.build_id = ?1 AND n.source_id = ?2",
            params![build_id, source_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    SourceRef {
                        provider: row.get(5)?,
                        account_id: row.get(6)?,
                        external_id: row.get(7)?,
                        url: row.get(8)?,
                        kind: row.get(9)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| CoreError::NotFound {
            entity: "graph node".to_string(),
            id: source_id.to_string(),
        })
        .and_then(
            |(title, snapshot_hash, link_digest, access_status, is_deleted, source)| {
                Ok(GraphNode {
                    id: source_id.to_string(),
                    source,
                    title,
                    snapshot_hash,
                    link_digest,
                    build_id: build_id.to_string(),
                    access_status: AccessStatus::from_str(&access_status)?,
                    is_deleted: is_deleted != 0,
                })
            },
        )
}

fn load_graph_edge(
    transaction: &Transaction<'_>,
    build_id: &str,
    row: GraphEdgeRow,
) -> CoreResult<GraphEdge> {
    let from = load_source(transaction, &row.from_source_id)?;
    let to = load_source(transaction, &row.to_source_id)?;
    Ok(GraphEdge {
        id: row.id,
        from_source_id: row.from_source_id,
        to_source_id: row.to_source_id,
        from,
        to,
        relation_type: row.relation_type,
        provenance: GraphProvenance::from_str(&row.provenance)?,
        evidence_location: row.evidence_location,
        confidence: row.confidence,
        source_url: row.source_url,
        target_url: row.target_url,
        source_hash: row.source_hash,
        target_hash: row.target_hash,
        build_id: build_id.to_string(),
        stale: row.stale,
    })
}

fn live_graph_snapshot(
    transaction: &Transaction<'_>,
    source_id: &str,
) -> CoreResult<Option<(String, String, AccessStatus, bool)>> {
    transaction
        .query_row(
            "SELECT content_hash, body, links_json, access_status, is_deleted
             FROM snapshots WHERE source_id = ?1",
            params![source_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)? != 0,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .map(|(hash, body, links_json, access_status, deleted)| {
            let links = serde_json::from_str::<Vec<SourceLink>>(&links_json).map_err(|error| {
                CoreError::Database {
                    message: error.to_string(),
                }
            })?;
            Ok((
                hash,
                graph_link_digest(&extract_graph_links(&body, &links)),
                AccessStatus::from_str(&access_status)?,
                deleted,
            ))
        })
        .transpose()
}

fn find_source_id(transaction: &Transaction<'_>, source: &SourceRef) -> CoreResult<Option<String>> {
    transaction
        .query_row(
            "SELECT id FROM sources WHERE provider = ?1 AND account_id = ?2
             AND external_id = ?3 AND kind = ?4",
            params![
                source.provider,
                source.account_id,
                source.external_id,
                source.kind
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

fn ensure_source(transaction: &Transaction<'_>, source: &SourceRef) -> CoreResult<String> {
    let source_id = find_source_id(transaction, source)?.unwrap_or_else(|| source.id());
    transaction
        .execute(
            "INSERT INTO sources(id, provider, account_id, external_id, url, kind, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(provider, account_id, external_id, kind) DO UPDATE SET url = excluded.url",
            params![
                source_id,
                source.provider,
                source.account_id,
                source.external_id,
                source.url,
                source.kind,
                now_rfc3339(),
            ],
        )
        .map_err(database_error)?;
    Ok(source_id)
}

fn load_source<C>(connection: &C, source_id: &str) -> CoreResult<SourceRef>
where
    C: Deref<Target = Connection>,
{
    connection
        .query_row(
            "SELECT provider, account_id, external_id, url, kind FROM sources WHERE id = ?1",
            params![source_id],
            |row| {
                Ok(SourceRef {
                    provider: row.get(0)?,
                    account_id: row.get(1)?,
                    external_id: row.get(2)?,
                    url: row.get(3)?,
                    kind: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| CoreError::NotFound {
            entity: "source".to_string(),
            id: source_id.to_string(),
        })
}

fn load_source_result(
    transaction: &Transaction<'_>,
    source_id: &str,
    max_age_seconds: Option<i64>,
) -> CoreResult<(Option<SearchResult>, Option<SourceRef>)> {
    let source = load_source(transaction, source_id)?;
    let row = transaction
        .query_row(
            "SELECT title, body, source_updated_at, fetched_at, access_status, is_deleted
             FROM snapshots WHERE source_id = ?1",
            params![source_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some((title, body, source_updated_at, fetched_at, access_status, is_deleted)) = row else {
        return Ok((None, Some(source)));
    };
    let access_status = AccessStatus::from_str(&access_status)?;
    if !access_status.is_searchable() || is_deleted != 0 {
        return Ok((None, Some(source)));
    }
    Ok((
        Some(SearchResult {
            source,
            title,
            snippet: body.chars().take(240).collect(),
            source_updated_at,
            fetched_at: fetched_at.clone(),
            freshness: freshness(&access_status, &fetched_at, max_age_seconds)?,
            access_status,
        }),
        None,
    ))
}

fn load_memory<C>(connection: &C, id: &str) -> CoreResult<Memory>
where
    C: Deref<Target = Connection>,
{
    let row = connection
        .query_row(
            "SELECT body, reason, author, claim_type, version, idempotency_key,
                    created_at, updated_at, retracted_at, supersedes_id
             FROM memories WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| CoreError::NotFound {
            entity: "memory".to_string(),
            id: id.to_string(),
        })?;
    let mut statement = connection
        .prepare(
            "SELECT s.provider, s.account_id, s.external_id, s.url, s.kind
             FROM memory_evidence e JOIN sources s ON s.id = e.source_id
             WHERE e.memory_id = ?1 ORDER BY s.provider, s.external_id",
        )
        .map_err(database_error)?;
    let evidence_rows = statement
        .query_map(params![id], |row| {
            Ok(SourceRef {
                provider: row.get(0)?,
                account_id: row.get(1)?,
                external_id: row.get(2)?,
                url: row.get(3)?,
                kind: row.get(4)?,
            })
        })
        .map_err(database_error)?;
    let mut evidence = Vec::new();
    for row in evidence_rows {
        evidence.push(row.map_err(database_error)?);
    }
    Ok(Memory {
        id: id.to_string(),
        body: row.0,
        reason: row.1,
        author: row.2,
        claim_type: row.3,
        version: row.4,
        idempotency_key: row.5,
        created_at: row.6,
        updated_at: row.7,
        retracted_at: row.8,
        supersedes_id: row.9,
        evidence,
    })
}

fn load_ui_memory<C>(connection: &C, memory_id: &str) -> CoreResult<UiMemory>
where
    C: Deref<Target = Connection>,
{
    let row = connection
        .query_row(
            "SELECT title, work, kind FROM ui_memories WHERE memory_id = ?1",
            params![memory_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| CoreError::NotFound {
            entity: "ui memory".to_string(),
            id: memory_id.to_string(),
        })?;
    Ok(UiMemory {
        id: memory_id.to_string(),
        title: row.0,
        work: row.1,
        kind: row.2,
        memory: load_memory(connection, memory_id)?,
    })
}

fn load_observation<C>(connection: &C, id: &str) -> CoreResult<Observation>
where
    C: Deref<Target = Connection>,
{
    let row = connection
        .query_row(
            "SELECT session_id, body, evidence_json, actor, state, version,
                    idempotency_key, created_at, updated_at
             FROM observations WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| CoreError::NotFound {
            entity: "observation".to_string(),
            id: id.to_string(),
        })?;
    let evidence = serde_json::from_str(&row.2).map_err(|error| CoreError::Database {
        message: error.to_string(),
    })?;
    Ok(Observation {
        id: id.to_string(),
        session_id: row.0,
        body: row.1,
        evidence,
        actor: row.3,
        state: CandidateState::from_str(&row.4)?,
        version: row.5,
        idempotency_key: row.6,
        created_at: row.7,
        updated_at: row.8,
    })
}

fn load_candidate<C>(connection: &C, id: &str) -> CoreResult<MemoryCandidate>
where
    C: Deref<Target = Connection>,
{
    let row = connection
        .query_row(
            "SELECT observation_id, body, reason, evidence_json, author, claim_type,
                    state, version, idempotency_key, created_at, updated_at,
                    accepted_memory_id, rejection_reason
             FROM memory_candidates WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| CoreError::NotFound {
            entity: "memory candidate".to_string(),
            id: id.to_string(),
        })?;
    let evidence = serde_json::from_str(&row.3).map_err(|error| CoreError::Database {
        message: error.to_string(),
    })?;
    Ok(MemoryCandidate {
        id: id.to_string(),
        observation_id: row.0,
        body: row.1,
        reason: row.2,
        evidence,
        author: row.4,
        claim_type: row.5,
        state: CandidateState::from_str(&row.6)?,
        version: row.7,
        idempotency_key: row.8,
        created_at: row.9,
        updated_at: row.10,
        accepted_memory_id: row.11,
        rejection_reason: row.12,
    })
}

fn source_ids_for_memory(
    transaction: &Transaction<'_>,
    evidence: &[SourceRef],
) -> CoreResult<Vec<String>> {
    let mut ids = Vec::with_capacity(evidence.len());
    for source in evidence {
        let id = find_source_id(transaction, source)?.ok_or_else(|| CoreError::NotFound {
            entity: "evidence source".to_string(),
            id: source.id(),
        })?;
        ids.push(id);
    }
    Ok(ids)
}

fn replace_memory_evidence(
    transaction: &Transaction<'_>,
    memory_id: &str,
    source_ids: &[String],
) -> CoreResult<()> {
    transaction
        .execute(
            "DELETE FROM memory_evidence WHERE memory_id = ?1",
            params![memory_id],
        )
        .map_err(database_error)?;
    for source_id in source_ids {
        transaction
            .execute(
                "INSERT INTO memory_evidence(memory_id, source_id) VALUES (?1, ?2)",
                params![memory_id, source_id],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

fn insert_revision(transaction: &Transaction<'_>, memory: &Memory, action: &str) -> CoreResult<()> {
    let evidence_json =
        serde_json::to_string(&memory.evidence).map_err(|error| CoreError::Database {
            message: error.to_string(),
        })?;
    transaction
        .execute(
            "INSERT INTO memory_revisions
                (memory_id, version, body, reason, evidence_json, author, claim_type,
                 supersedes_id, action, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                memory.id,
                memory.version,
                memory.body,
                memory.reason,
                evidence_json,
                memory.author,
                memory.claim_type,
                memory.supersedes_id,
                action,
                memory.updated_at,
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn load_revision(
    transaction: &Transaction<'_>,
    memory_id: &str,
    version: i64,
) -> CoreResult<MemoryRevision> {
    let row = transaction
        .query_row(
            "SELECT body, reason, evidence_json, author, claim_type, supersedes_id,
                    action, created_at
             FROM memory_revisions WHERE memory_id = ?1 AND version = ?2",
            params![memory_id, version],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| CoreError::NotFound {
            entity: "memory revision".to_string(),
            id: format!("{memory_id}@{version}"),
        })?;
    let evidence = serde_json::from_str(&row.2).map_err(|error| CoreError::Database {
        message: error.to_string(),
    })?;
    Ok(MemoryRevision {
        memory_id: memory_id.to_string(),
        version,
        body: row.0,
        reason: row.1,
        evidence,
        author: row.3,
        claim_type: row.4,
        supersedes_id: row.5,
        action: row.6,
        created_at: row.7,
    })
}

fn check_expected_version(memory: &Memory, expected: i64) -> CoreResult<()> {
    if memory.version != expected {
        return Err(CoreError::VersionConflict {
            entity: "memory".to_string(),
            id: memory.id.clone(),
            expected,
            actual: memory.version,
        });
    }
    Ok(())
}

fn validate_mutation_key(id: &str, key: &str) -> CoreResult<()> {
    if id.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "id".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if key.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "idempotency_key".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    Ok(())
}

fn validate_memory_input(input: &MemoryUpsertInput) -> CoreResult<()> {
    if input.body.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "body".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if input.reason.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "reason".to_string(),
            message: "is required to explain why the memory is stored".to_string(),
        });
    }
    if input.evidence.is_empty() {
        return Err(CoreError::InvalidInput {
            field: "evidence".to_string(),
            message: "at least one source reference is required".to_string(),
        });
    }
    if input.author.trim().is_empty() || input.claim_type.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "author/claim_type".to_string(),
            message: "both fields are required".to_string(),
        });
    }
    if input.idempotency_key.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "idempotency_key".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if input.expected_version.is_some_and(|version| version < 0) {
        return Err(CoreError::InvalidInput {
            field: "expected_version".to_string(),
            message: "must not be negative".to_string(),
        });
    }
    if contains_credential_marker(&input.body) || contains_credential_marker(&input.reason) {
        return Err(CoreError::SensitiveDataRejected);
    }
    let mut seen = HashSet::new();
    for source in &input.evidence {
        source.validate()?;
        if !seen.insert(format!(
            "{}\0{}\0{}\0{}",
            source.provider, source.account_id, source.external_id, source.kind
        )) {
            return Err(CoreError::InvalidInput {
                field: "evidence".to_string(),
                message: "duplicate source references are not allowed".to_string(),
            });
        }
    }
    Ok(())
}

fn validate_observation_capture(input: &ObservationCaptureInput) -> CoreResult<()> {
    if input.session_id.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "session_id".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if input.body.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "body".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if input.actor.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "actor".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if input.evidence.is_empty() {
        return Err(CoreError::InvalidInput {
            field: "evidence".to_string(),
            message: "at least one source reference is required".to_string(),
        });
    }
    if input.idempotency_key.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "idempotency_key".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if contains_credential_marker(&input.body) {
        return Err(CoreError::SensitiveDataRejected);
    }
    validate_evidence_references(&input.evidence)
}

fn validate_candidate_distill(input: &CandidateDistillInput) -> CoreResult<()> {
    if input.observation_id.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "observation_id".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if input.body.trim().is_empty() || input.reason.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "body/reason".to_string(),
            message: "both fields are required".to_string(),
        });
    }
    if input.author.trim().is_empty() || input.claim_type.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "author/claim_type".to_string(),
            message: "both fields are required".to_string(),
        });
    }
    if input.idempotency_key.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "idempotency_key".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if input.expected_version.is_some_and(|version| version < 1) {
        return Err(CoreError::InvalidInput {
            field: "expected_version".to_string(),
            message: "must be greater than zero".to_string(),
        });
    }
    if contains_credential_marker(&input.body) || contains_credential_marker(&input.reason) {
        return Err(CoreError::SensitiveDataRejected);
    }
    Ok(())
}

fn validate_evidence_references(evidence: &[SourceRef]) -> CoreResult<()> {
    let mut seen = HashSet::new();
    for source in evidence {
        source.validate()?;
        if !seen.insert(source.id()) {
            return Err(CoreError::InvalidInput {
                field: "evidence".to_string(),
                message: "duplicate source references are not allowed".to_string(),
            });
        }
    }
    Ok(())
}

fn validate_candidate_transition(id: &str, expected_version: i64, key: &str) -> CoreResult<()> {
    if id.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "id".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if expected_version < 1 {
        return Err(CoreError::InvalidInput {
            field: "expected_version".to_string(),
            message: "must be greater than zero".to_string(),
        });
    }
    if key.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "idempotency_key".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    Ok(())
}

fn check_candidate_version(candidate: &MemoryCandidate, expected: i64) -> CoreResult<()> {
    if candidate.version != expected {
        return Err(CoreError::VersionConflict {
            entity: "memory candidate".to_string(),
            id: candidate.id.clone(),
            expected,
            actual: candidate.version,
        });
    }
    Ok(())
}

fn validate_ui_memory_input(input: &UiMemoryUpsertInput) -> CoreResult<()> {
    if input.title.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "title".to_string(),
            message: "must not be empty".to_string(),
        });
    }
    if input.title.chars().count() > 100 {
        return Err(CoreError::InvalidInput {
            field: "title".to_string(),
            message: "must be at most 100 characters".to_string(),
        });
    }
    if input.work < 0 || input.kind.trim().is_empty() {
        return Err(CoreError::InvalidInput {
            field: "work/kind".to_string(),
            message: "work must be non-negative and kind is required".to_string(),
        });
    }
    validate_memory_input(&input.memory)
}

fn request_digest<T: Serialize>(value: &T) -> CoreResult<String> {
    let encoded = serde_json::to_vec(value).map_err(|error| CoreError::Database {
        message: error.to_string(),
    })?;
    Ok(sha256_hex(&encoded))
}

fn check_idempotency<T>(
    transaction: &Transaction<'_>,
    key: &str,
    operation: &str,
    digest: &str,
) -> CoreResult<Option<T>>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let row = transaction
        .query_row(
            "SELECT operation, request_digest, response_json
             FROM idempotency_records WHERE idempotency_key = ?1",
            params![key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some((stored_operation, stored_digest, response_json)) = row else {
        return Ok(None);
    };
    if stored_operation != operation || stored_digest != digest {
        return Err(CoreError::IdempotencyConflict {
            key: key.to_string(),
        });
    }
    let mut response: T =
        serde_json::from_str(&response_json).map_err(|error| CoreError::Database {
            message: error.to_string(),
        })?;
    // All current idempotent response types contain this field.  Keep the
    // serialized response authoritative; callers still receive the replay
    // marker via a small JSON round trip so the contract remains generic.
    if let Ok(mut value) = serde_json::to_value(&response) {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "idempotent_replay".to_string(),
                serde_json::Value::Bool(true),
            );
            response = serde_json::from_value(value).map_err(|error| CoreError::Database {
                message: error.to_string(),
            })?;
        }
    }
    Ok(Some(response))
}

fn store_idempotency<T: Serialize>(
    transaction: &Transaction<'_>,
    key: &str,
    operation: &str,
    digest: &str,
    response: &T,
) -> CoreResult<()> {
    let response_json = serde_json::to_string(response).map_err(|error| CoreError::Database {
        message: error.to_string(),
    })?;
    transaction
        .execute(
            "INSERT INTO idempotency_records
                (idempotency_key, operation, request_digest, response_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![key, operation, digest, response_json, now_rfc3339()],
        )
        .map_err(database_error)?;
    Ok(())
}

fn load_sync_state(connection: &Connection, connection_id: &str) -> CoreResult<SyncState> {
    connection
        .query_row(
            "SELECT connection_id, provider, scope, status, last_success_at, last_attempt_at,
                    failure_at, cursor, last_error_code, last_error, retry_at
             FROM sync_states WHERE connection_id = ?1",
            params![connection_id],
            |row| {
                Ok(SyncState {
                    connection_id: row.get(0)?,
                    provider: row.get(1)?,
                    scope: row.get(2)?,
                    status: row.get(3)?,
                    last_success_at: row.get(4)?,
                    last_attempt_at: row.get(5)?,
                    failure_at: row.get(6)?,
                    cursor: row.get(7)?,
                    last_error_code: row.get(8)?,
                    last_error: row.get(9)?,
                    retry_at: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| CoreError::NotFound {
            entity: "sync state".to_string(),
            id: connection_id.to_string(),
        })
}

fn freshness(
    access_status: &AccessStatus,
    fetched_at: &str,
    max_age_seconds: Option<i64>,
) -> CoreResult<Freshness> {
    if !access_status.is_searchable() {
        return Ok(Freshness::Unavailable);
    }
    let Some(max_age_seconds) = max_age_seconds else {
        return Ok(Freshness::Fresh);
    };
    let fetched = parse_timestamp(fetched_at)?;
    let age = (Utc::now() - fetched).num_seconds();
    if age > max_age_seconds {
        Ok(Freshness::Stale)
    } else {
        Ok(Freshness::Fresh)
    }
}

fn safe_fts_query(query: &str) -> Option<String> {
    let terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" AND "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_second_migration_does_not_record_a_partial_version() {
        let error =
            Database::in_memory_with_migration_failure(2).expect_err("migration should fail");
        assert!(matches!(error, CoreError::Migration { version: 2, .. }));

        let mut connection = Connection::open_in_memory().expect("in-memory DB");
        // A fresh connection is used here because the failed constructor does
        // not expose its intentionally discarded connection.  The migration
        // runner itself is additionally exercised below with an injected
        // failure after version 1 has been applied.
        connection
            .execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);")
            .expect("bootstrap table");
        connection
            .execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (1, 'test')",
                [],
            )
            .expect("version one");
        let error = migrate(&mut connection, Some(2)).expect_err("migration should fail");
        assert!(matches!(error, CoreError::Migration { version: 2, .. }));
        let versions: i64 = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("version count");
        assert_eq!(versions, 1);
        let has_revisions: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_revisions'",
                [],
                |row| row.get(0),
            )
            .expect("schema query");
        assert_eq!(has_revisions, 0);
    }
}
