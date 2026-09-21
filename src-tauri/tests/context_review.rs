use aidebook_lib::core::*;
fn seed(core: &Core, id: &str) -> SourceRef {
    let source = SourceRef::new(
        "context-review",
        "scope",
        id,
        format!("https://example.test/{id}"),
        "note",
    );
    core.ingest_snapshot(Snapshot::new(
        source.clone(),
        format!("shared {id}"),
        "shared evidence",
        None,
        "2026-09-21T00:00:00Z",
    ))
    .unwrap();
    core.memory_upsert(MemoryUpsertInput {
        id: None,
        body: format!("shared decision {id}"),
        reason: "shared review".into(),
        evidence: vec![source.clone()],
        author: "user".into(),
        claim_type: "decision".into(),
        idempotency_key: format!("review:{id}"),
        expected_version: None,
        supersedes_id: None,
    })
    .unwrap();
    source
}
#[test]
fn unrooted_query_does_not_limit_memories_to_first_lexical_source() {
    let core = Core::in_memory().unwrap();
    seed(&core, "one");
    seed(&core, "two");
    let packet = core
        .context_query(ContextQueryRequest {
            query: "shared".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(packet.sources.len(), 2);
    assert_eq!(packet.memories.len(), 2);
}
#[test]
fn cache_clear_keeps_memory_but_marks_its_missing_evidence_unavailable() {
    let core = Core::in_memory().unwrap();
    let source = seed(&core, "clear");
    core.clear_cache().unwrap();
    let packet = core
        .context_query(ContextQueryRequest {
            query: "shared".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(packet.memories.len(), 1);
    assert!(packet.sources.is_empty());
    assert!(packet
        .unavailable_sources
        .iter()
        .any(|s| s.id() == source.id()));
}
#[test]
fn new_source_after_graph_build_does_not_break_context_packet() {
    let core = Core::in_memory().unwrap();
    seed(&core, "old");
    core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    let newer = seed(&core, "newer");
    let packet = core
        .context_query(ContextQueryRequest {
            query: "newer".into(),
            source: Some(newer.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(packet.sources.iter().any(|s| s.source.id() == newer.id()));
    assert_eq!(packet.memories.len(), 1);
}
#[test]
fn explicit_revoked_root_is_reported_without_a_graph_build() {
    let core = Core::in_memory().unwrap();
    let source = seed(&core, "denied");
    core.record_source_failure(
        source.clone(),
        AccessStatus::PermissionDenied,
        "revoked",
        None,
    )
    .unwrap();
    let packet = core
        .context_query(ContextQueryRequest {
            query: "shared".into(),
            source: Some(source.clone()),
            ..Default::default()
        })
        .unwrap();
    assert!(packet.sources.is_empty());
    assert_eq!(packet.memories.len(), 1);
    assert!(packet
        .unavailable_sources
        .iter()
        .any(|s| s.id() == source.id()));
}
#[test]
fn source_and_memory_limits_report_truncation() {
    let core = Core::in_memory().unwrap();
    seed(&core, "limit-one");
    seed(&core, "limit-two");
    let packet = core
        .context_query(ContextQueryRequest {
            query: "shared".into(),
            max_sources: Some(1),
            max_memories: Some(1),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(packet.sources.len(), 1);
    assert_eq!(packet.memories.len(), 1);
    assert!(packet.bounds.truncated);
}

#[test]
fn memory_only_match_reports_stale_evidence_under_requested_age() {
    let core = Core::in_memory().unwrap();
    let source = seed(&core, "memory-age");
    let packet = core
        .context_query(ContextQueryRequest {
            query: "decision".into(),
            max_age_seconds: Some(0),
            ..Default::default()
        })
        .unwrap();
    assert!(packet.sources.is_empty());
    assert_eq!(packet.memories.len(), 1);
    assert!(
        packet.stale_sources.iter().any(|s| s.id() == source.id()),
        "memory evidence freshness applies even without lexical source matches"
    );
}
