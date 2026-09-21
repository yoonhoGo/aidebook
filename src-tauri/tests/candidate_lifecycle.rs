use aidebook_lib::core::*;

fn source() -> (SourceRef, Snapshot) {
    let source = SourceRef::new(
        "candidate-test",
        "lifecycle",
        "decision.md",
        "obsidian://open?vault=lifecycle&file=decision.md",
        "note",
    );
    let snapshot = Snapshot::new(
        source.clone(),
        "Decision",
        "Keep reviewable local memories",
        None,
        "2026-09-21T00:00:00Z",
    );
    (source, snapshot)
}

fn proposed(core: &Core) -> MemoryCandidate {
    let (source, snapshot) = source();
    core.ingest_snapshot(snapshot).expect("source");
    let capture = core
        .observation_capture(ObservationCaptureInput {
            id: None,
            session_id: "lifecycle-session".into(),
            body: "Keep reviewable local memories".into(),
            evidence: vec![source],
            actor: "agent".into(),
            idempotency_key: "lifecycle:capture".into(),
        })
        .expect("capture");
    let distilled = core
        .candidate_distill(CandidateDistillInput {
            observation_id: capture.observation.id,
            body: "Keep reviewable local memories".into(),
            reason: "The observation is grounded in the selected note".into(),
            author: "agent".into(),
            claim_type: "decision".into(),
            idempotency_key: "lifecycle:distill".into(),
            expected_version: Some(1),
        })
        .expect("distill");
    core.candidate_propose(CandidateProposeInput {
        id: distilled.candidate.id,
        expected_version: distilled.candidate.version,
        idempotency_key: "lifecycle:propose".into(),
    })
    .expect("propose")
    .candidate
}

#[test]
fn lifecycle_is_persistent_and_idempotent_until_human_review() {
    let core = Core::in_memory().expect("core");
    let candidate = proposed(&core);
    let replay = core
        .observation_capture(ObservationCaptureInput {
            id: None,
            session_id: "lifecycle-session".into(),
            body: "Keep reviewable local memories".into(),
            evidence: vec![source().0],
            actor: "agent".into(),
            idempotency_key: "lifecycle:capture".into(),
        })
        .expect("capture replay");
    assert!(replay.idempotent_replay);
    assert_eq!(replay.observation.state, CandidateState::Captured);
    assert_eq!(
        core.observation(&replay.observation.id).unwrap().state,
        CandidateState::Distilled
    );
    assert_eq!(candidate.state, CandidateState::Proposed);
    assert_eq!(
        core.candidates(Some(CandidateState::Proposed))
            .unwrap()
            .len(),
        1
    );

    let conflict = core.candidate_propose(CandidateProposeInput {
        id: candidate.id.clone(),
        expected_version: candidate.version - 1,
        idempotency_key: "lifecycle:stale-propose".into(),
    });
    assert!(matches!(conflict, Err(CoreError::VersionConflict { .. })));
    assert_eq!(
        core.candidates(Some(CandidateState::Accepted))
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn reject_is_terminal_and_never_creates_a_canonical_memory() {
    let core = Core::in_memory().expect("core");
    let candidate = proposed(&core);
    let rejected = core
        .candidate_reject(CandidateRejectInput {
            id: candidate.id.clone(),
            expected_version: candidate.version,
            idempotency_key: "lifecycle:reject".into(),
            reason: Some("Review evidence was insufficient".into()),
        })
        .expect("reject");
    assert_eq!(rejected.candidate.state, CandidateState::Rejected);
    assert_eq!(
        core.candidates(Some(CandidateState::Rejected))
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        core.candidate_accept(CandidateAcceptInput {
            id: candidate.id,
            expected_version: rejected.candidate.version - 1,
            idempotency_key: "lifecycle:late-accept".into(),
            memory_id: None,
            expected_memory_version: None,
        }),
        Err(CoreError::VersionConflict { .. })
    ));
}
