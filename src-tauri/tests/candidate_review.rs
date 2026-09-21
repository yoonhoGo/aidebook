use aidebook_lib::core::*;
use rusqlite::Connection;

fn proposed(core: &Core, key: &str) -> MemoryCandidate {
    let source = SourceRef::new(
        "review",
        "candidate",
        key,
        format!("https://example.test/{key}"),
        "note",
    );
    core.ingest_snapshot(Snapshot::new(
        source.clone(),
        "Decision",
        "Keep local data",
        None,
        "2026-09-21T00:00:00Z",
    ))
    .unwrap();
    let observation = core
        .observation_capture(ObservationCaptureInput {
            id: None,
            session_id: "review-session".into(),
            body: "Keep local data".into(),
            evidence: vec![source],
            actor: "agent".into(),
            idempotency_key: format!("{key}:capture"),
        })
        .unwrap();
    let distilled = core
        .candidate_distill(CandidateDistillInput {
            observation_id: observation.observation.id,
            body: "Keep local data".into(),
            reason: "Explicit local decision".into(),
            author: "agent".into(),
            claim_type: "decision".into(),
            idempotency_key: format!("{key}:distill"),
            expected_version: Some(1),
        })
        .unwrap();
    core.candidate_propose(CandidateProposeInput {
        id: distilled.candidate.id,
        expected_version: distilled.candidate.version,
        idempotency_key: format!("{key}:propose"),
    })
    .unwrap()
    .candidate
}

#[test]
fn promotion_rolls_back_every_write_when_candidate_update_fails() {
    let dir = std::env::temp_dir().join(format!(
        "aidebook-candidate-review-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("core.sqlite");
    let core = Core::open(&db).unwrap();
    let candidate = proposed(&core, "atomic");
    let connection = Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_review BEFORE UPDATE OF state ON memory_candidates WHEN NEW.state = 'accepted' BEGIN SELECT RAISE(ABORT, 'review failure fixture'); END;").unwrap();
    let input = CandidateAcceptInput {
        id: candidate.id.clone(),
        expected_version: candidate.version,
        idempotency_key: "atomic:accept".into(),
        memory_id: Some("review-atomic-memory".into()),
        expected_memory_version: Some(0),
    };
    assert!(core.candidate_accept(input.clone()).is_err());
    let after = core.candidate(&candidate.id).unwrap();
    assert_eq!(after.state, CandidateState::Proposed);
    assert_eq!(after.version, candidate.version);
    for table in ["memories", "memory_revisions", "memory_evidence"] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table} must be rolled back");
    }
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM idempotency_records WHERE idempotency_key LIKE '%accept%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "both outer and nested idempotency writes roll back"
    );
    connection
        .execute_batch("DROP TRIGGER fail_review;")
        .unwrap();
    let accepted = core.candidate_accept(input.clone()).unwrap();
    assert_eq!(accepted.candidate.state, CandidateState::Accepted);
    let replay = core.candidate_accept(input).unwrap();
    assert!(replay.idempotent_replay);
    assert_eq!(accepted.memory.id, replay.memory.id);
    assert_eq!(core.memory_history(&accepted.memory.id).unwrap().len(), 1);
    drop(connection);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn competing_review_cannot_accept_a_rejected_candidate() {
    let core = Core::in_memory().unwrap();
    let candidate = proposed(&core, "competing");
    core.candidate_reject(CandidateRejectInput {
        id: candidate.id.clone(),
        expected_version: candidate.version,
        idempotency_key: "competing:reject".into(),
        reason: Some("Not supported".into()),
    })
    .unwrap();
    let result = core.candidate_accept(CandidateAcceptInput {
        id: candidate.id.clone(),
        expected_version: candidate.version,
        idempotency_key: "competing:accept".into(),
        memory_id: None,
        expected_memory_version: None,
    });
    assert!(matches!(result, Err(CoreError::VersionConflict { .. })));
    assert_eq!(
        core.candidate(&candidate.id).unwrap().state,
        CandidateState::Rejected
    );
}
