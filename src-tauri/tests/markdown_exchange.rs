use aidebook_lib::core::*;

fn source(id: &str) -> SourceRef {
    SourceRef::new(
        "markdown-review",
        "account",
        id,
        format!("https://example.test/{id}"),
        "note",
    )
}

fn seed_memory(core: &Core, id: &str, body: &str) -> SourceRef {
    let source = source(id);
    core.ingest_snapshot(Snapshot::new(
        source.clone(),
        format!("Title {id}"),
        format!("Body with [[{id}]]"),
        None,
        "2026-09-21T00:00:00Z",
    ))
    .unwrap();
    core.memory_upsert(MemoryUpsertInput {
        id: Some(format!("memory-{id}")),
        body: body.to_string(),
        reason: "Keep the reviewed decision available.".to_string(),
        evidence: vec![source.clone()],
        author: "user".to_string(),
        claim_type: "decision".to_string(),
        idempotency_key: format!("seed-{id}"),
        expected_version: Some(0),
        supersedes_id: None,
    })
    .unwrap();
    source
}

#[test]
fn export_is_deterministic_and_contains_evidence_revisions_and_wikilinks() {
    let core = Core::in_memory().unwrap();
    let source = seed_memory(&core, "one", "Original decision body.");
    core.memory_upsert(MemoryUpsertInput {
        id: Some("memory-one".to_string()),
        body: "Updated decision body.".to_string(),
        reason: "The decision was clarified.".to_string(),
        evidence: vec![source],
        author: "user".to_string(),
        claim_type: "decision".to_string(),
        idempotency_key: "seed-one-update".to_string(),
        expected_version: Some(1),
        supersedes_id: None,
    })
    .unwrap();

    let first = core.memory_export_markdown().unwrap();
    let second = core.memory_export_markdown().unwrap();
    assert_eq!(first, second);
    assert!(first.contains("<!-- aidebook-memory-exchange:v1 -->"));
    assert!(first.contains("### Aidebook Revisions"));
    assert!(first.contains("### Aidebook Evidence"));
    assert!(first.contains("[[markdown-review/account/one]]"));
    assert!(first.contains("Updated decision body."));
    assert!(first.contains("Original decision body."));
}

#[test]
fn import_uses_edited_body_and_is_idempotent_without_auto_acceptance() {
    let exporter = Core::in_memory().unwrap();
    seed_memory(&exporter, "one", "Original decision body.");
    let markdown = exporter
        .memory_export_markdown()
        .unwrap()
        .replace("Original decision body.", "Edited imported body.");

    let importer = Core::in_memory().unwrap();
    importer
        .ingest_snapshot(Snapshot::new(
            source("one"),
            "Imported source",
            "Evidence source",
            None,
            "2026-09-21T00:00:00Z",
        ))
        .unwrap();
    let first = importer.memory_import_markdown(markdown.clone()).unwrap();
    assert_eq!(first.imported, 1);
    assert_eq!(first.idempotent, 0);
    assert_eq!(first.candidates.len(), 1);
    assert_eq!(first.candidates[0].state, CandidateState::Proposed);
    assert_eq!(first.candidates[0].body, "Edited imported body.");
    assert_eq!(first.candidates[0].evidence, vec![source("one")]);
    assert!(importer.ui_memories().unwrap().is_empty());

    let second = importer.memory_import_markdown(markdown).unwrap();
    assert_eq!(second.imported, 0);
    assert_eq!(second.idempotent, 1);
    assert_eq!(second.candidates[0].id, first.candidates[0].id);
}

#[test]
fn malformed_later_section_rolls_back_every_candidate() {
    let exporter = Core::in_memory().unwrap();
    seed_memory(&exporter, "one", "First body.");
    seed_memory(&exporter, "two", "Second body.");
    let markdown = exporter.memory_export_markdown().unwrap();
    let marker = "### Aidebook Evidence";
    let position = markdown.rfind(marker).unwrap();
    let malformed = format!(
        "{}### Broken Evidence{}",
        &markdown[..position],
        &markdown[position + marker.len()..]
    );
    let importer = Core::in_memory().unwrap();
    importer
        .ingest_snapshot(Snapshot::new(
            source("one"),
            "One",
            "Evidence",
            None,
            "2026-09-21T00:00:00Z",
        ))
        .unwrap();
    importer
        .ingest_snapshot(Snapshot::new(
            source("two"),
            "Two",
            "Evidence",
            None,
            "2026-09-21T00:00:00Z",
        ))
        .unwrap();
    assert!(importer.memory_import_markdown(malformed).is_err());
    assert!(importer.candidates(None).unwrap().is_empty());
}
