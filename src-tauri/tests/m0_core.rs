use aidebook_lib::core::FixtureAdapter;
use aidebook_lib::core::{
    AccessStatus, ContextRequest, Core, CoreError, Freshness, MemoryRestoreInput,
    MemoryRetractInput, MemoryUpsertInput, ReadOnlyConnector, SearchRequest, SourceRef,
};
use std::fs;
use uuid::Uuid;

fn fixture_core() -> Core {
    let core = Core::in_memory().expect("in-memory core");
    let obsidian = FixtureAdapter::from_json(include_str!("../fixtures/obsidian.json"))
        .expect("Obsidian fixture");
    let github =
        FixtureAdapter::from_json(include_str!("../fixtures/github.json")).expect("GitHub fixture");
    let obsidian_results = core.ingest_connector(&obsidian).expect("ingest Obsidian");
    let github_results = core.ingest_connector(&github).expect("ingest GitHub");
    assert_eq!(obsidian_results.len(), 4);
    assert_eq!(github_results.len(), 4);
    assert!(obsidian_results[1].deduplicated);
    assert!(github_results[1].deduplicated);
    core
}

fn release_source() -> SourceRef {
    SourceRef::new(
        "obsidian",
        "vault-fixture",
        "Projects/Aidebook/Release.md",
        "obsidian://open?vault=fixture&file=Projects%2FAidebook%2FRelease.md",
        "note",
    )
}

#[test]
fn both_read_only_fixtures_dedupe_and_exclude_denied_sources() {
    let core = fixture_core();
    let results = core
        .search(SearchRequest {
            query: String::new(),
            provider: None,
            kind: None,
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: None,
            limit: None,
        })
        .expect("search fixture cache");
    assert_eq!(results.results.len(), 2);
    assert!(results
        .results
        .iter()
        .all(|result| result.access_status == AccessStatus::Accessible));

    let release_results = core
        .search(SearchRequest {
            query: "release".to_string(),
            provider: None,
            kind: None,
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: None,
            limit: Some(50),
        })
        .expect("FTS search");
    assert_eq!(release_results.results.len(), 2);
    assert!(release_results
        .results
        .iter()
        .any(|result| result.source.provider == "obsidian"));
    assert!(release_results
        .results
        .iter()
        .any(|result| result.source.provider == "github"));

    let denied = core
        .context(ContextRequest {
            source_id: None,
            source: Some(SourceRef::new(
                "github",
                "yoonhoGo",
                "yoonhoGo/aidebook#99",
                "https://github.com/yoonhoGo/aidebook/issues/99",
                "issue",
            )),
            max_age_seconds: None,
        })
        .expect("denied context");
    assert!(denied.sources.is_empty());
    assert_eq!(denied.unavailable_sources.len(), 1);
    assert_eq!(denied.missing_providers, vec!["github".to_string()]);
}

#[test]
fn failed_refresh_keeps_last_good_snapshot_and_survives_restart() {
    let path = std::env::temp_dir().join(format!("aidebook-m0-{}.sqlite", Uuid::new_v4()));
    let core = Core::open(&path).expect("file-backed core");
    let source = release_source();
    let original = core
        .ingest_snapshot(aidebook_lib::core::Snapshot::new(
            source.clone(),
            "Release",
            "last good body",
            Some("2026-09-18T08:00:00Z".to_string()),
            "2026-09-18T09:00:00Z",
        ))
        .expect("good snapshot");
    assert!(!original.deduplicated);
    core.record_source_failure(
        source.clone(),
        AccessStatus::Unavailable,
        "provider timeout",
        Some("2026-09-18T10:00:00Z".to_string()),
    )
    .expect("failed refresh");
    let cached = core.snapshot(&source).expect("cached failed source");
    assert_eq!(cached.body, "last good body");
    assert_eq!(cached.fetched_at, "2026-09-18T09:00:00Z");
    assert_eq!(cached.access_status, AccessStatus::Unavailable);
    drop(core);

    let reopened = Core::open(&path).expect("reopen core");
    let cached_after_restart = reopened.snapshot(&source).expect("persisted cache");
    assert_eq!(cached_after_restart.body, "last good body");
    assert_eq!(
        cached_after_restart.access_status,
        AccessStatus::Unavailable
    );
    let results = reopened
        .search(SearchRequest {
            query: "last".to_string(),
            provider: None,
            kind: None,
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: None,
            limit: None,
        })
        .expect("search unavailable cache");
    assert!(results.results.is_empty());
    fs::remove_file(&path).expect("remove test database");
}

#[test]
fn memory_mutations_are_idempotent_versioned_and_reversible() {
    let core = fixture_core();
    let evidence = vec![release_source()];
    let initial = core
        .upsert_memory(MemoryUpsertInput {
            id: None,
            body: "Wait for native smoke verification before release.".to_string(),
            reason: "This release constraint must survive the next session.".to_string(),
            evidence: evidence.clone(),
            author: "agent".to_string(),
            claim_type: "constraint".to_string(),
            idempotency_key: "memory-create-1".to_string(),
            expected_version: None,
            supersedes_id: None,
        })
        .expect("create memory");
    assert!(initial.created);
    assert_eq!(initial.memory.version, 1);
    assert_eq!(initial.memory.evidence, evidence);

    let replay = core
        .upsert_memory(MemoryUpsertInput {
            id: None,
            body: "Wait for native smoke verification before release.".to_string(),
            reason: "This release constraint must survive the next session.".to_string(),
            evidence: vec![release_source()],
            author: "agent".to_string(),
            claim_type: "constraint".to_string(),
            idempotency_key: "memory-create-1".to_string(),
            expected_version: None,
            supersedes_id: None,
        })
        .expect("idempotent replay");
    assert!(replay.idempotent_replay);
    assert_eq!(replay.memory.id, initial.memory.id);
    assert_eq!(replay.memory.version, 1);

    let conflict = core.upsert_memory(MemoryUpsertInput {
        id: None,
        body: "different body".to_string(),
        reason: "different request".to_string(),
        evidence: vec![release_source()],
        author: "agent".to_string(),
        claim_type: "constraint".to_string(),
        idempotency_key: "memory-create-1".to_string(),
        expected_version: None,
        supersedes_id: None,
    });
    assert!(matches!(
        conflict,
        Err(CoreError::IdempotencyConflict { .. })
    ));

    let updated = core
        .upsert_memory(MemoryUpsertInput {
            id: Some(initial.memory.id.clone()),
            body: "Wait for native smoke verification and record the result.".to_string(),
            reason: "The constraint was refined by the user.".to_string(),
            evidence: vec![release_source()],
            author: "user".to_string(),
            claim_type: "constraint".to_string(),
            idempotency_key: "memory-update-2".to_string(),
            expected_version: Some(1),
            supersedes_id: None,
        })
        .expect("update memory");
    assert_eq!(updated.memory.version, 2);

    let stale_update = core.upsert_memory(MemoryUpsertInput {
        id: Some(initial.memory.id.clone()),
        body: "stale update".to_string(),
        reason: "should conflict".to_string(),
        evidence: vec![release_source()],
        author: "agent".to_string(),
        claim_type: "constraint".to_string(),
        idempotency_key: "memory-stale".to_string(),
        expected_version: Some(1),
        supersedes_id: None,
    });
    assert!(matches!(
        stale_update,
        Err(CoreError::VersionConflict {
            expected: 1,
            actual: 2,
            ..
        })
    ));

    let retracted = core
        .retract_memory(MemoryRetractInput {
            id: initial.memory.id.clone(),
            expected_version: 2,
            idempotency_key: "memory-retract-3".to_string(),
        })
        .expect("retract memory");
    assert_eq!(retracted.memory.version, 3);
    assert!(retracted.memory.retracted_at.is_some());

    let restored = core
        .restore_memory(MemoryRestoreInput {
            id: initial.memory.id.clone(),
            expected_version: 3,
            revision_version: 1,
            idempotency_key: "memory-restore-4".to_string(),
        })
        .expect("restore memory");
    assert_eq!(restored.memory.version, 4);
    assert_eq!(restored.memory.body, initial.memory.body);
    assert!(restored.memory.retracted_at.is_none());
    let history = core
        .memory_history(&initial.memory.id)
        .expect("memory history");
    assert_eq!(history.len(), 4);
    assert_eq!(history[2].action, "retracted");
    assert_eq!(history[3].action, "restored_from:1");
}

#[test]
fn sync_failure_does_not_erase_last_success_and_search_limits_are_guarded() {
    let core = fixture_core();
    let success = core
        .record_sync_success(
            "fixture-github-repository",
            "github",
            "yoonhoGo/aidebook",
            Some("cursor-1".to_string()),
            Some("2026-09-18T09:00:00Z".to_string()),
        )
        .expect("sync success");
    assert_eq!(
        success.last_success_at.as_deref(),
        Some("2026-09-18T09:00:00Z")
    );
    let failed = core
        .record_sync_failure(
            "fixture-github-repository",
            "github",
            "yoonhoGo/aidebook",
            "permission_denied",
            "fixture permission denied",
            Some("2026-09-18T10:00:00Z".to_string()),
            Some("2026-09-18T10:05:00Z".to_string()),
        )
        .expect("sync failure");
    assert_eq!(failed.status, "failed");
    assert_eq!(
        failed.last_success_at.as_deref(),
        Some("2026-09-18T09:00:00Z")
    );
    assert_eq!(failed.cursor.as_deref(), Some("cursor-1"));

    let too_many = core.search(SearchRequest {
        query: String::new(),
        provider: None,
        kind: None,
        source_updated_after: None,
        source_updated_before: None,
        max_age_seconds: None,
        limit: Some(51),
    });
    assert!(matches!(too_many, Err(CoreError::InvalidInput { field, .. }) if field == "limit"));

    let old_source = SourceRef::new(
        "obsidian",
        "vault-fixture",
        "Projects/Aidebook/Old.md",
        "obsidian://open?vault=fixture&file=Projects%2FAidebook%2FOld.md",
        "note",
    );
    core.ingest_snapshot(aidebook_lib::core::Snapshot::new(
        old_source,
        "Old cache",
        "old text",
        Some("2020-01-01T00:00:00Z".to_string()),
        "2020-01-01T00:00:00Z",
    ))
    .expect("old snapshot");
    let stale = core
        .search(SearchRequest {
            query: "old".to_string(),
            provider: None,
            kind: None,
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: Some(1),
            limit: Some(20),
        })
        .expect("stale search");
    assert_eq!(stale.results.len(), 1);
    assert_eq!(stale.results[0].freshness, Freshness::Stale);
}

#[test]
fn fixture_contract_is_read_only_and_has_explicit_provider_scope() {
    let adapter =
        FixtureAdapter::from_json(include_str!("../fixtures/github.json")).expect("GitHub fixture");
    assert_eq!(
        adapter.manifest().permissions,
        vec!["read-only".to_string()]
    );
    assert!(adapter
        .manifest()
        .capabilities
        .contains(&"read".to_string()));
    assert_eq!(adapter.connection_id(), "fixture-github-repository");
    let core = Core::in_memory().expect("in-memory core");
    let refresh = core.sources_refresh(&adapter).expect("sources.refresh");
    assert_eq!(refresh.attempted, 4);
    assert_eq!(refresh.indexed, 2);
    assert_eq!(refresh.deduplicated, 1);
    assert_eq!(refresh.inaccessible, 2);
    assert_eq!(refresh.sync_state.status, "succeeded");
    let unknown = SourceRef::new(
        "github",
        "other-account",
        "other/repo#1",
        "https://github.com/other/repo/issues/1",
        "issue",
    );
    assert!(matches!(
        adapter.fetch(&unknown),
        Err(CoreError::NotFound { .. })
    ));
}

#[test]
fn credential_like_memory_content_is_rejected_without_blocking_normal_text() {
    let core = fixture_core();
    let normal = core
        .upsert_memory(MemoryUpsertInput {
            id: None,
            body: "The task-1 checklist remains pending.".to_string(),
            reason: "A normal task reference is safe to retain.".to_string(),
            evidence: vec![release_source()],
            author: "agent".to_string(),
            claim_type: "next_action".to_string(),
            idempotency_key: "safe-task".to_string(),
            expected_version: None,
            supersedes_id: None,
        })
        .expect("normal text");
    assert_eq!(normal.memory.version, 1);
    let rejected = core.upsert_memory(MemoryUpsertInput {
        id: None,
        body: "Do not save token=secret-value".to_string(),
        reason: "credential exclusion test".to_string(),
        evidence: vec![release_source()],
        author: "agent".to_string(),
        claim_type: "inferred".to_string(),
        idempotency_key: "secret-memory".to_string(),
        expected_version: None,
        supersedes_id: None,
    });
    assert!(matches!(rejected, Err(CoreError::SensitiveDataRejected)));
}
