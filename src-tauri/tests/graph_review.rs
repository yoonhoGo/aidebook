use aidebook_lib::core::{
    AccessStatus, Core, GraphRebuildRequest, GraphTraversalRequest, RelationInput, Snapshot,
    SourceLink, SourceRef,
};

fn sample(id: &str) -> Snapshot {
    Snapshot::new(
        SourceRef::new(
            "obsidian",
            "review",
            format!("{id}.md"),
            format!("https://example.test/{id}"),
            "note",
        ),
        id,
        format!("body of {id}"),
        None,
        "2026-09-21T00:00:00Z",
    )
}
fn traverse(core: &Core, source: &SourceRef) -> aidebook_lib::core::GraphTraversalResponse {
    core.graph_traverse(GraphTraversalRequest {
        source: Some(source.clone()),
        max_depth: Some(3),
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn url_changes_invalidate_edges_without_changing_body() {
    let core = Core::in_memory().unwrap();
    let mut a = sample("a");
    let mut b = sample("b");
    a.links.push(SourceLink {
        target: b.source.url.clone(),
        kind: "url".into(),
    });
    core.ingest_snapshot(a.clone()).unwrap();
    core.ingest_snapshot(b.clone()).unwrap();
    core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    assert_eq!(traverse(&core, &a.source).edges.len(), 1);
    b.source.url = "https://example.test/new-b".into();
    core.ingest_snapshot(b).unwrap();
    assert!(
        traverse(&core, &a.source).edges.is_empty(),
        "old URL edge is stale even if body hash matches"
    );
    core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    assert!(traverse(&core, &a.source).edges.is_empty());
}

#[test]
fn revoked_target_does_not_leave_dangling_edges_or_cached_titles() {
    let core = Core::in_memory().unwrap();
    let mut a = sample("public");
    let b = sample("private-marker");
    a.links.push(SourceLink {
        target: b.source.url.clone(),
        kind: "url".into(),
    });
    core.ingest_snapshot(a.clone()).unwrap();
    core.ingest_snapshot(b.clone()).unwrap();
    core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    core.record_source_failure(
        b.source.clone(),
        AccessStatus::PermissionDenied,
        "revoked",
        None,
    )
    .unwrap();
    let result = traverse(&core, &a.source);
    assert!(result.edges.is_empty());
    assert!(result
        .nodes
        .iter()
        .all(|node| node.source.id() != b.source.id()));
    core.clear_cache().unwrap();
    match core.graph_traverse(GraphTraversalRequest {
        source: Some(a.source),
        ..Default::default()
    }) {
        Ok(result) => {
            assert!(result.nodes.is_empty());
            assert!(result.edges.is_empty());
        }
        Err(_) => {}
    }
}

#[test]
fn removed_explicit_relation_cannot_survive_a_rebuild_digest_replay() {
    let core = Core::in_memory().unwrap();
    let a = sample("from");
    let b = sample("to");
    core.ingest_snapshot(a.clone()).unwrap();
    core.ingest_snapshot(b.clone()).unwrap();
    let relation = RelationInput {
        from: a.source.clone(),
        to: b.source.clone(),
        relation_type: "reference".into(),
        reason: "review".into(),
    };
    core.add_relation(relation.clone()).unwrap();
    let first = core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    assert_eq!(traverse(&core, &a.source).edges.len(), 1);
    core.remove_relation(relation).unwrap();
    let next = core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    assert_ne!(first.build.build_id, next.build.build_id);
    assert!(traverse(&core, &a.source).edges.is_empty());
}

#[test]
fn traversal_handles_inbound_edges_and_returns_only_complete_edges_at_bounds() {
    let core = Core::in_memory().unwrap();
    let mut a = sample("hub");
    let b = sample("leaf1");
    let c = sample("leaf2");
    for target in [&b, &c] {
        a.links.push(SourceLink {
            target: target.source.url.clone(),
            kind: "url".into(),
        });
    }
    for item in [&a, &b, &c] {
        core.ingest_snapshot(item.clone()).unwrap();
    }
    core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    let inbound = traverse(&core, &b.source);
    assert_eq!(
        inbound.nodes.len(),
        3,
        "inbound traversal reaches hub and other leaf"
    );
    assert_eq!(
        inbound.edges.len(),
        2,
        "inbound traversal retains directed edges without duplicates"
    );
    let bounded = core
        .graph_traverse(GraphTraversalRequest {
            source: Some(a.source),
            max_nodes: Some(2),
            max_depth: Some(3),
            ..Default::default()
        })
        .unwrap();
    let ids: std::collections::HashSet<_> = bounded.nodes.iter().map(|n| n.source.id()).collect();
    assert!(bounded.truncated);
    assert!(
        bounded
            .edges
            .iter()
            .all(|e| ids.contains(&e.from_source_id) && ids.contains(&e.to_source_id)),
        "every returned edge must have both endpoint nodes inside bounds"
    );
}

#[test]
fn link_only_changes_are_stale_before_rebuild_and_revocation_is_unavailable() {
    let core = Core::in_memory().unwrap();
    let mut a = sample("link-source");
    let b = sample("link-target");
    a.links.push(SourceLink {
        target: b.source.url.clone(),
        kind: "url".into(),
    });
    core.ingest_snapshot(a.clone()).unwrap();
    core.ingest_snapshot(b.clone()).unwrap();
    core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    a.links.clear();
    core.ingest_snapshot(a.clone()).unwrap();
    let stale = traverse(&core, &a.source);
    assert!(
        stale.edges.is_empty(),
        "link-only edits must invalidate old edges before rebuild"
    );
    a.links.push(SourceLink {
        target: b.source.url.clone(),
        kind: "url".into(),
    });
    core.ingest_snapshot(a.clone()).unwrap();
    core.rebuild_graph(GraphRebuildRequest::default()).unwrap();
    core.record_source_failure(
        b.source.clone(),
        AccessStatus::PermissionDenied,
        "revoked",
        None,
    )
    .unwrap();
    let unavailable = traverse(&core, &a.source);
    assert!(
        unavailable
            .unavailable_sources
            .iter()
            .any(|s| s.id() == b.source.id()),
        "revocation is unavailable, not merely stale"
    );
}
