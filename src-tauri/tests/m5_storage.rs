use aidebook_lib::core::{
    AccessStatus, Core, CoreError, MemoryUpsertInput, SearchRequest, Snapshot, SourceRef,
};
use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

fn source(id: &str) -> (SourceRef, Snapshot) {
    let source = SourceRef::new(
        "m5",
        "storage",
        id,
        format!("https://example.test/m5/{id}"),
        "note",
    );
    let snapshot = Snapshot::new(
        source.clone(),
        format!("Title {id}"),
        format!("body {id} benchmark"),
        Some("2026-09-19T00:00:00Z".to_string()),
        "2026-09-19T00:01:00Z",
    );
    (source, snapshot)
}

fn search(core: &Core, query: &str) -> usize {
    core.search(SearchRequest {
        query: query.to_string(),
        provider: Some("m5".to_string()),
        kind: Some("note".to_string()),
        source_updated_after: None,
        source_updated_before: None,
        max_age_seconds: None,
        limit: Some(20),
    })
    .expect("search")
    .results
    .len()
}

#[test]
fn backup_restore_is_integrity_checked_and_cache_clear_preserves_memories() {
    let directory = PathBuf::from(format!("/tmp/aidebook-m5-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).expect("directory");
    let database = directory.join("aidebook.sqlite");
    let backup = directory.join("aidebook.backup.sqlite");
    let core = Core::open(&database).expect("database");
    let (first, first_snapshot) = source("first");
    let (_, second_snapshot) = source("second");
    core.ingest_snapshot(first_snapshot)
        .expect("first snapshot");
    core.upsert_memory(MemoryUpsertInput {
        id: None,
        body: "preserve this user decision".to_string(),
        reason: "explicit local memory".to_string(),
        evidence: vec![first.clone()],
        author: "user".to_string(),
        claim_type: "decision".to_string(),
        idempotency_key: "m5-memory".to_string(),
        expected_version: None,
        supersedes_id: None,
    })
    .expect("memory");
    let backup_result = core.backup_to(&backup).expect("backup");
    assert_eq!(backup_result.schema_version, 8);
    core.ingest_snapshot(second_snapshot)
        .expect("second snapshot");
    assert_eq!(search(&core, "second"), 1);
    core.restore_from(&backup).expect("restore");
    assert_eq!(search(&core, "second"), 0);
    assert_eq!(search(&core, "first"), 1);

    let cache = core.clear_cache().expect("clear cache");
    assert_eq!(cache.snapshots_removed, 1);
    assert!(cache.sources_preserved && cache.memories_preserved);
    assert_eq!(search(&core, "first"), 0);
    let context = core
        .context_get(aidebook_lib::core::ContextRequest {
            source: Some(first.clone()),
            source_id: None,
            max_age_seconds: None,
        })
        .expect("context after cache clear");
    assert_eq!(context.unavailable_sources.len(), 1);
    assert_eq!(context.memories.len(), 1);

    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn corrupt_restore_does_not_replace_active_database() {
    let directory = PathBuf::from(format!("/tmp/aidebook-m5-corrupt-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).expect("directory");
    let database = directory.join("aidebook.sqlite");
    let corrupt = directory.join("corrupt.sqlite");
    let core = Core::open(&database).expect("database");
    let (_, snapshot) = source("safe");
    core.ingest_snapshot(snapshot).expect("snapshot");
    fs::write(&corrupt, b"not sqlite").expect("corrupt fixture");
    let error = core.restore_from(&corrupt).expect_err("corrupt restore");
    assert!(matches!(error, CoreError::Database { .. }));
    assert_eq!(search(&core, "safe"), 1);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn restoring_a_v4_backup_migrates_only_the_staged_copy() {
    let directory = PathBuf::from(format!("/tmp/aidebook-m5-v4-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).expect("directory");
    let database = directory.join("aidebook.sqlite");
    let backup = directory.join("aidebook-v6.sqlite");
    let legacy = directory.join("aidebook-v4.sqlite");
    let core = Core::open(&database).expect("database");
    let (source, snapshot) = source("legacy");
    core.ingest_snapshot(snapshot).expect("snapshot");
    core.upsert_memory(MemoryUpsertInput {
        id: None,
        body: "legacy memory".to_string(),
        reason: "migration test".to_string(),
        evidence: vec![source],
        author: "user".to_string(),
        claim_type: "decision".to_string(),
        idempotency_key: "legacy-memory".to_string(),
        expected_version: None,
        supersedes_id: None,
    })
    .expect("memory");
    core.backup_to(&backup).expect("backup");
    fs::copy(&backup, &legacy).expect("legacy copy");
    {
        let connection = Connection::open(&legacy).expect("legacy sqlite");
        connection
            .execute_batch(
                "DROP TABLE tasks;
                 DROP TABLE work_items;
                 DROP TABLE activity_events;
                 DROP TABLE memory_candidates;
                 DROP TABLE observations;
                 DROP TABLE graph_edges;
                 DROP TABLE graph_nodes;
                 DROP TABLE graph_builds;
                 DELETE FROM schema_migrations WHERE version > 4;",
            )
            .expect("strip newer graph migrations");
    }
    let before = fs::read(&legacy).expect("legacy bytes");
    let result = core.restore_from(&legacy).expect("restore legacy backup");
    assert_eq!(result.schema_version, 8);
    assert_eq!(
        fs::read(&legacy).expect("legacy bytes after restore"),
        before
    );
    assert_eq!(search(&core, "legacy"), 1);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn failure_state_is_not_treated_as_cache_success() {
    let core = Core::in_memory().expect("core");
    let (source, snapshot) = source("failure");
    core.ingest_snapshot(snapshot).expect("snapshot");
    core.record_source_failure(
        source,
        AccessStatus::Unavailable,
        "offline",
        Some("2026-09-19T00:02:00Z".to_string()),
    )
    .expect("failure");
    assert_eq!(search(&core, "failure"), 0);
}
