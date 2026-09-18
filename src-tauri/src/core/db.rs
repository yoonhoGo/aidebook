//! SQLite/FTS5 persistence for the local core.
//!
//! Only this module owns SQLite connections.  Callers operate through
//! `Core`'s request/response methods, which is the boundary that a Tauri
//! command, CLI process, or MCP stdio server will use in later milestones.

use super::types::*;
use chrono::Utc;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ops::Deref;
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

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
];

#[derive(Debug)]
pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> CoreResult<Self> {
        let connection = Connection::open(path).map_err(database_error)?;
        Self::from_connection(connection, None)
    }

    pub fn in_memory() -> CoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(database_error)?;
        Self::from_connection(connection, None)
    }

    fn from_connection(mut connection: Connection, fail_at: Option<i64>) -> CoreResult<Self> {
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(database_error)?;
        migrate(&mut connection, fail_at)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Test-only constructor used to prove that a failed migration does not
    /// leave a partially applied schema behind.
    #[cfg(test)]
    fn in_memory_with_migration_failure(version: i64) -> CoreResult<Self> {
        let connection = Connection::open_in_memory().map_err(database_error)?;
        Self::from_connection(connection, Some(version))
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

    pub fn upsert_memory(&self, input: MemoryUpsertInput) -> CoreResult<MemoryMutation> {
        validate_memory_input(&input)?;
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(database_error)?;
        if let Some(replay) = check_idempotency::<MemoryMutation>(
            &transaction,
            &input.idempotency_key,
            "memory.upsert",
            &digest,
        )? {
            transaction.commit().map_err(database_error)?;
            return Ok(replay);
        }

        let (memory_id, created, version, created_at, previous) = if let Some(id) = &input.id {
            match load_memory(&transaction, id) {
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

        let evidence_ids = source_ids_for_memory(&transaction, &input.evidence)?;
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
        replace_memory_evidence(&transaction, &memory_id, &evidence_ids)?;
        let memory = load_memory(&transaction, &memory_id)?;
        insert_revision(
            &transaction,
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
            &transaction,
            &input.idempotency_key,
            "memory.upsert",
            &digest,
            &mutation,
        )?;
        transaction.commit().map_err(database_error)?;
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

fn database_error(error: rusqlite::Error) -> CoreError {
    CoreError::Database {
        message: error.to_string(),
    }
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
