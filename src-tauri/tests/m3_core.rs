use aidebook_lib::core::{
    AccessStatus, Core, CoreError, MemoryUpsertInput, RelationInput, SearchRequest, Snapshot,
    SourceRef, UiMemoryUpsertInput,
};

fn source(external_id: &str, title: &str) -> (SourceRef, Snapshot) {
    let source = SourceRef::new(
        "fixture",
        "m3-account",
        external_id,
        format!("https://example.test/{external_id}"),
        "note",
    );
    let snapshot = Snapshot::new(
        source.clone(),
        title,
        format!("body for {external_id}"),
        Some("2026-09-19T00:00:00Z".to_string()),
        "2026-09-19T00:01:00Z",
    );
    (source, snapshot)
}

fn ui_input(
    title: &str,
    body: &str,
    source: &SourceRef,
    id: Option<String>,
    expected_version: Option<i64>,
    idempotency_key: &str,
) -> UiMemoryUpsertInput {
    UiMemoryUpsertInput {
        title: title.to_string(),
        work: 0,
        kind: "decision".to_string(),
        memory: MemoryUpsertInput {
            id,
            body: body.to_string(),
            reason: "user explicitly recorded this decision".to_string(),
            evidence: vec![source.clone()],
            author: "user".to_string(),
            claim_type: "explicit".to_string(),
            idempotency_key: idempotency_key.to_string(),
            expected_version,
            supersedes_id: None,
        },
    }
}

#[test]
fn same_title_does_not_merge_and_explicit_relation_can_be_removed() {
    let core = Core::in_memory().expect("core");
    let (first, first_snapshot) = source("one", "Same title");
    let (second, second_snapshot) = source("two", "Same title");
    core.ingest_snapshot(first_snapshot).expect("first source");
    core.ingest_snapshot(second_snapshot)
        .expect("second source");
    let first_id = first.id();
    let second_id = second.id();
    assert_ne!(first_id, second_id);
    core.add_relation(RelationInput {
        from: first.clone(),
        to: second.clone(),
        relation_type: "explicit_related".to_string(),
        reason: "user linked these two records".to_string(),
    })
    .expect("relation");
    let context = core
        .context_get(aidebook_lib::core::ContextRequest {
            source_id: Some(first_id),
            source: None,
            max_age_seconds: None,
        })
        .expect("context");
    assert_eq!(context.sources.len(), 2);
    assert!(core
        .remove_relation(RelationInput {
            from: first.clone(),
            to: second.clone(),
            relation_type: "explicit_related".to_string(),
            reason: "disconnect".to_string(),
        })
        .expect("remove relation"));
    let disconnected = core
        .context_get(aidebook_lib::core::ContextRequest {
            source_id: Some(second.id()),
            source: None,
            max_age_seconds: None,
        })
        .expect("context after disconnect");
    assert_eq!(disconnected.sources.len(), 1);
}

#[test]
fn ui_memory_save_is_versioned_idempotent_and_keeps_user_memory_when_evidence_is_unavailable() {
    let core = Core::in_memory().expect("core");
    let (source, snapshot) = source("evidence", "Evidence");
    core.ingest_snapshot(snapshot).expect("evidence source");
    let first = core
        .ui_memory_upsert(ui_input(
            "User decision",
            "Keep local-only writes",
            &source,
            None,
            None,
            "m3-create",
        ))
        .expect("save");
    let replay = core
        .ui_memory_upsert(ui_input(
            "User decision",
            "Keep local-only writes",
            &source,
            None,
            None,
            "m3-create",
        ))
        .expect("idempotent replay");
    assert!(replay.idempotent_replay);
    assert_eq!(replay.memory.title, "User decision");
    let conflict = core.ui_memory_upsert(ui_input(
        "User decision",
        "Changed concurrently",
        &source,
        Some(first.memory.id.clone()),
        Some(0),
        "m3-conflict",
    ));
    assert!(matches!(conflict, Err(CoreError::VersionConflict { .. })));

    core.record_source_failure(
        source.clone(),
        AccessStatus::PermissionDenied,
        "selected provider access was revoked",
        Some("2026-09-19T00:02:00Z".to_string()),
    )
    .expect("mark evidence unavailable");
    let context = core
        .context_get(aidebook_lib::core::ContextRequest {
            source: Some(source),
            source_id: None,
            max_age_seconds: None,
        })
        .expect("context with unavailable evidence");
    assert!(context.sources.is_empty());
    assert_eq!(context.unavailable_sources.len(), 1);
    assert_eq!(context.memories.len(), 1);
    assert_eq!(context.memories[0].body, "Keep local-only writes");
}

#[test]
fn ten_explicit_ui_examples_remain_separate_records() {
    let core = Core::in_memory().expect("core");
    let (source, snapshot) = source("ten", "Repeated title");
    core.ingest_snapshot(snapshot).expect("source");
    for index in 0..10 {
        core.ui_memory_upsert(ui_input(
            "Repeated title",
            &format!("example {index}"),
            &source,
            None,
            None,
            &format!("m3-example-{index}"),
        ))
        .expect("save example");
    }
    let memories = core.ui_memories().expect("list UI memories");
    assert_eq!(memories.len(), 10);
    let search = core
        .search(SearchRequest {
            query: "body".to_string(),
            provider: Some("fixture".to_string()),
            kind: Some("note".to_string()),
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: None,
            limit: Some(20),
        })
        .expect("search");
    assert_eq!(search.results.len(), 1);
}
