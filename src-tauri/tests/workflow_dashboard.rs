use aidebook_lib::core::{
    Core, DashboardInput, DashboardPage, DashboardSection as S, TimeBlock, WorkflowFields,
    WorkflowItem, WorkflowKind as K, WorkflowSaveInput,
};
use std::time::Instant;

fn fields(title: &str) -> WorkflowFields {
    WorkflowFields {
        title: title.into(),
        status: "planned".into(),
        purpose: String::new(),
        blocked_reason: None,
        work_id: None,
        target_date: None,
        priority: 0,
        pinned: false,
        time_blocks: vec![],
    }
}
fn save(core: &Core, kind: K, fields: WorkflowFields) -> WorkflowItem {
    core.workflow_save(WorkflowSaveInput {
        kind,
        id: None,
        expected_version: None,
        idempotency_key: format!("create-{}", fields.title),
        fields,
    })
    .unwrap()
    .item
}
fn input(section: S, now: &str, timezone: &str) -> DashboardInput {
    DashboardInput {
        section,
        now: Some(now.into()),
        timezone: timezone.into(),
        limit: 100,
        offset: 0,
        kind: None,
    }
}
fn page(core: &Core, section: S) -> DashboardPage {
    core.dashboard_get(input(section, "2026-09-23T12:00:00Z", "Asia/Seoul"))
        .unwrap()
}
fn block(start: &str, end: &str) -> TimeBlock {
    TimeBlock {
        start: start.into(),
        end: end.into(),
    }
}

#[test]
fn local_midnight_and_dst_days_are_half_open() {
    for (zone, start, end, inside, outside) in [
        (
            "Asia/Seoul",
            "2026-09-22T14:30:00Z",
            "2026-09-22T15:00:00Z",
            "2026-09-22T14:59:00Z",
            "2026-09-22T15:00:00Z",
        ),
        (
            "America/New_York",
            "2026-03-08T00:00:00-05:00",
            "2026-03-09T00:00:00-04:00",
            "2026-03-09T03:59:00Z",
            "2026-03-09T04:00:00Z",
        ),
        (
            "America/New_York",
            "2026-11-01T00:00:00-04:00",
            "2026-11-02T00:00:00-05:00",
            "2026-11-02T04:59:00Z",
            "2026-11-02T05:00:00Z",
        ),
    ] {
        let core = Core::in_memory().unwrap();
        let mut f = fields("timed");
        f.time_blocks = vec![block(start, end)];
        save(&core, K::Task, f);
        assert_eq!(
            core.dashboard_get(input(S::Today, inside, zone))
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            core.dashboard_get(input(S::Today, outside, zone))
                .unwrap()
                .total,
            0
        );
    }
    let core = Core::in_memory().unwrap();
    let mut f = fields("cross-midnight");
    f.time_blocks = vec![block(
        "2026-09-22T23:30:00+09:00",
        "2026-09-23T00:30:00+09:00",
    )];
    save(&core, K::Task, f);
    for now in ["2026-09-22T12:00:00+09:00", "2026-09-23T12:00:00+09:00"] {
        assert_eq!(
            core.dashboard_get(input(S::Today, now, "Asia/Seoul"))
                .unwrap()
                .total,
            1
        );
    }
}

#[test]
fn ordering_reasons_filters_and_pages_are_consistent() {
    let core = Core::in_memory().unwrap();
    for (title, pinned, due, priority) in [
        ("priority", false, None, 3),
        ("soon", false, Some("2026-09-26"), 0),
        ("overdue", false, Some("2026-09-20"), 0),
        ("pinned", true, None, 0),
        ("no-date", false, None, 0),
        ("today", false, Some("2026-09-23"), 0),
    ] {
        let mut f = fields(title);
        f.pinned = pinned;
        f.target_date = due.map(str::to_owned);
        f.priority = priority;
        save(&core, K::Task, f);
    }
    let all = page(&core, S::Next);
    assert_eq!(
        all.entries
            .iter()
            .map(|e| e.item.fields.title.as_str())
            .collect::<Vec<_>>(),
        vec!["pinned", "overdue", "today", "soon", "priority", "no-date"]
    );
    assert_eq!(all.entries[0].reasons, vec!["pinned"]);
    assert_eq!(all.entries[1].reasons, vec!["overdue"]);
    assert_eq!(page(&core, S::Today).total, 1);
    assert_eq!(page(&core, S::Attention).total, 1);
    assert!(!all.calendar_connected);
    let mut collected = vec![];
    for offset in [0, 2, 4, 6] {
        let mut request = input(S::Next, "2026-09-23T12:00:00Z", "Asia/Seoul");
        request.limit = 2;
        request.offset = offset;
        let p = core.dashboard_get(request).unwrap();
        assert_eq!(p.total, 6);
        assert_eq!(p.has_more, offset < 4);
        collected.extend(p.entries.into_iter().map(|e| e.entry_id));
    }
    assert_eq!(
        collected,
        all.entries
            .iter()
            .map(|e| e.entry_id.clone())
            .collect::<Vec<_>>()
    );
    let mut request = input(S::Next, "2026-09-23T12:00:00Z", "Asia/Seoul");
    request.kind = Some(K::Work);
    assert_eq!(core.dashboard_get(request).unwrap().total, 0);
}

#[test]
fn inactive_parents_block_actions_and_report_attention() {
    let dir = TempDir::new();
    let db = dir.0.join("core.sqlite");
    let core = Core::open(&db).unwrap();
    for status in ["on_hold", "cancelled", "done"] {
        let mut f = fields(status);
        f.status = if status == "done" { "review" } else { status }.into();
        let mut parent = save(&core, K::Work, f);
        if status == "done" {
            // Fixture the trusted UI result; public API intentionally cannot confirm work completion.
            parent.fields.status = "done".into();
            rusqlite::Connection::open(&db)
                .unwrap()
                .execute(
                    "UPDATE work_items SET data_json=?1 WHERE id=?2",
                    rusqlite::params![serde_json::to_string(&parent).unwrap(), parent.id],
                )
                .unwrap();
        }
        let mut child = fields(&format!("child-{status}"));
        child.work_id = Some(parent.id);
        child.target_date = Some("2026-09-23".into());
        save(&core, K::Task, child);
    }
    assert_eq!(page(&core, S::Next).total, 0);
    assert_eq!(page(&core, S::Today).total, 0);
    let attention = page(&core, S::Attention);
    assert_eq!(
        attention
            .entries
            .iter()
            .filter(|e| e.reasons.contains(&"parent_inactive".into()))
            .count(),
        3
    );
    for status in ["review", "in_progress"] {
        let mut f = fields(&format!("work-{status}"));
        f.status = status.into();
        save(&core, K::Work, f);
    }
    let mut blocked = fields("blocked");
    blocked.blocked_reason = Some("waiting".into());
    save(&core, K::Task, blocked);
    assert_eq!(page(&core, S::InProgress).total, 1);
    assert_eq!(page(&core, S::Next).total, 1);
    assert!(page(&core, S::Attention)
        .entries
        .iter()
        .any(|e| e.reasons.contains(&"review".into())));
}

#[test]
fn completion_uses_transition_time_not_later_edit() {
    let dir = TempDir::new();
    let db = dir.0.join("core.sqlite");
    let core = Core::open(&db).unwrap();
    let mut f = fields("finished");
    f.status = "done".into();
    let task = save(&core, K::Task, f);
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE activity_events SET occurred_at='2026-09-01T00:00:00Z' WHERE target_id=?1",
        [&task.id],
    )
    .unwrap();
    let mut edited = task.fields.clone();
    edited.title = "edited after finishing".into();
    let edited = core
        .workflow_save(WorkflowSaveInput {
            kind: K::Task,
            id: Some(task.id.clone()),
            expected_version: Some(task.version),
            idempotency_key: "edit-finished".into(),
            fields: edited,
        })
        .unwrap()
        .item;
    conn.execute("UPDATE activity_events SET occurred_at='2026-09-23T00:00:00Z' WHERE target_id=?1 AND previous_state='done'",[&task.id]).unwrap();
    assert_eq!(
        page(&core, S::Completed).total,
        0,
        "editing must not make old completion recent"
    );
    conn.execute("UPDATE activity_events SET occurred_at='2026-09-22T00:00:00Z' WHERE target_id=?1 AND previous_state IS NULL",[&task.id]).unwrap();
    assert_eq!(
        page(&core, S::Completed).entries[0].completed_at.as_deref(),
        Some("2026-09-22T00:00:00+00:00")
    );
    let mut reopened = edited.fields.clone();
    reopened.status = "in_progress".into();
    core.workflow_save(WorkflowSaveInput {
        kind: K::Task,
        id: Some(task.id),
        expected_version: Some(edited.version),
        idempotency_key: "reopen-finished".into(),
        fields: reopened,
    })
    .unwrap();
    assert_eq!(page(&core, S::Completed).total, 0);
}

#[test]
fn invalid_query_inputs_are_rejected() {
    let core = Core::in_memory().unwrap();
    for request in [
        input(S::Today, "bad", "UTC"),
        input(S::Today, "2026-09-23T00:00:00", "UTC"),
        input(S::Today, "2026-09-23T00:00:00Z", "not/a-zone"),
    ] {
        assert!(core.dashboard_get(request).is_err());
    }
    for (limit, offset) in [(0, 0), (101, 0), (20, usize::MAX)] {
        let mut r = input(S::Next, "2026-09-23T00:00:00Z", "UTC");
        r.limit = limit;
        r.offset = offset;
        assert!(core.dashboard_get(r).is_err());
    }
}

#[test]
fn dashboard_10k_query_p95_is_recorded() {
    let core = Core::in_memory().unwrap();
    let parent = save(&core, K::Work, fields("benchmark-parent"));
    for i in 0..9999 {
        let mut f = fields(&format!("benchmark-{i:05}"));
        f.work_id = Some(parent.id.clone());
        f.priority = (i % 4) as u8;
        f.pinned = i % 37 == 0;
        f.status = if i % 11 == 0 { "done" } else { "in_progress" }.into();
        f.target_date = Some(format!("2026-09-{:02}", 20 + i % 8));
        if i % 3 == 0 {
            f.time_blocks = vec![block("2026-09-23T01:00:00Z", "2026-09-23T02:00:00Z")];
        }
        save(&core, K::Task, f);
    }
    let mut results = vec![];
    for section in [S::Today, S::Next, S::Attention, S::InProgress, S::Completed] {
        let request = input(section, "2026-09-23T23:59:00Z", "UTC");
        for _ in 0..3 {
            core.dashboard_get(request.clone()).unwrap();
        }
        let mut durations = vec![];
        for _ in 0..30 {
            let start = Instant::now();
            core.dashboard_get(request.clone()).unwrap();
            durations.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        durations.sort_by(f64::total_cmp);
        let p95 = durations[28];
        results.push(serde_json::json!({"section":section,"p95_ms":p95,"target_ms":300,"target_met":p95<=300.0,"runs":30}));
    }
    println!(
        "WORKFLOW_DASHBOARD_BENCHMARK={}",
        serde_json::json!({"fixture_count":10000,"storage":"in_memory_sqlite","profile":"test_debug","os":std::env::consts::OS,"arch":std::env::consts::ARCH,"results":results})
    );
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("workflow-dashboard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
