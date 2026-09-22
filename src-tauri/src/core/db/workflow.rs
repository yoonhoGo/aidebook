//! Local workflow persistence. External source fields are deliberately absent.
use super::*;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE work_items (
    id TEXT PRIMARY KEY NOT NULL,
    data_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE tasks (
    id TEXT PRIMARY KEY NOT NULL,
    work_id TEXT REFERENCES work_items(id) ON DELETE RESTRICT,
    data_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_work_items_updated ON work_items(updated_at DESC, id);
CREATE INDEX idx_tasks_work_updated ON tasks(work_id, updated_at DESC, id);
CREATE INDEX idx_tasks_updated ON tasks(updated_at DESC, id);
CREATE TABLE activity_events (
    id TEXT PRIMARY KEY NOT NULL,
    target_id TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    previous_state TEXT,
    next_state TEXT NOT NULL,
    version INTEGER NOT NULL,
    occurred_at TEXT NOT NULL,
    UNIQUE(target_kind, target_id, version)
);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowKind {
    Work,
    Task,
}
impl WorkflowKind {
    fn table(self) -> &'static str {
        match self {
            Self::Work => "work_items",
            Self::Task => "tasks",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeBlock {
    pub start: String,
    pub end: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowFields {
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub work_id: Option<String>,
    /// Local target date, never a remote ticket deadline.
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub time_blocks: Vec<TimeBlock>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSaveInput {
    pub kind: WorkflowKind,
    pub id: Option<String>,
    pub expected_version: Option<i64>,
    pub idempotency_key: String,
    pub fields: WorkflowFields,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowItem {
    pub id: String,
    pub kind: WorkflowKind,
    pub version: i64,
    pub fields: WorkflowFields,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMutation {
    pub item: WorkflowItem,
    pub idempotent_replay: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowGetInput {
    pub kind: WorkflowKind,
    pub id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowListInput {
    pub kind: WorkflowKind,
    #[serde(default)]
    pub work_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}
fn default_limit() -> usize {
    50
}
fn invalid(field: &str, message: &str) -> CoreError {
    CoreError::InvalidInput {
        field: field.into(),
        message: message.into(),
    }
}
fn json_error(error: serde_json::Error) -> CoreError {
    CoreError::Database {
        message: error.to_string(),
    }
}
fn read_item(connection: &Connection, kind: WorkflowKind, id: &str) -> CoreResult<WorkflowItem> {
    let raw: Option<String> = connection
        .query_row(
            &format!("SELECT data_json FROM {} WHERE id=?1", kind.table()),
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;
    serde_json::from_str(&raw.ok_or_else(|| CoreError::NotFound {
        entity: kind.table().into(),
        id: id.into(),
    })?)
    .map_err(json_error)
}
fn validate(input: &WorkflowSaveInput) -> CoreResult<()> {
    let f = &input.fields;
    if f.title.trim().is_empty() || f.title.len() > 1000 {
        return Err(invalid("title", "requires 1–1000 bytes"));
    }
    if input.idempotency_key.trim().is_empty() || input.idempotency_key.len() > 200 {
        return Err(invalid("idempotency_key", "requires 1–200 bytes"));
    }
    if input.id.is_some() != input.expected_version.is_some()
        || input.expected_version.is_some_and(|v| v < 1)
    {
        return Err(invalid(
            "expected_version",
            "updates require an ID and a positive current version; creates require neither",
        ));
    }
    let states: &[&str] = match input.kind {
        WorkflowKind::Work => &[
            "planned",
            "in_progress",
            "review",
            "done",
            "on_hold",
            "cancelled",
        ],
        WorkflowKind::Task => &["planned", "in_progress", "done", "cancelled"],
    };
    if !states.contains(&f.status.as_str()) {
        return Err(invalid("status", "unsupported state for this kind"));
    }
    if input.kind == WorkflowKind::Work && (f.work_id.is_some() || !f.time_blocks.is_empty()) {
        return Err(invalid(
            "fields",
            "only tasks may have a parent work or time blocks",
        ));
    }
    if f.priority > 3 {
        return Err(invalid("priority", "must be 0–3; 3 is highest"));
    }
    if f.purpose.len() > 20000
        || f.blocked_reason
            .as_ref()
            .is_some_and(|v| v.len() > 2000 || v.trim().is_empty())
    {
        return Err(invalid("fields", "purpose is limited to 20000 bytes; blocked reason must be nonempty and at most 2000 bytes"));
    }
    if let Some(date) = &f.target_date {
        let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| invalid("target_date", "requires YYYY-MM-DD"))?;
        if parsed.format("%Y-%m-%d").to_string() != *date {
            return Err(invalid("target_date", "requires YYYY-MM-DD"));
        }
    }
    if f.time_blocks.len() > 100 {
        return Err(invalid("time_blocks", "maximum 100 blocks"));
    }
    for block in &f.time_blocks {
        let start = chrono::DateTime::parse_from_rfc3339(&block.start)
            .map_err(|_| invalid("time_blocks.start", "requires RFC3339 with offset"))?;
        let end = chrono::DateTime::parse_from_rfc3339(&block.end)
            .map_err(|_| invalid("time_blocks.end", "requires RFC3339 with offset"))?;
        if start >= end {
            return Err(invalid("time_blocks", "end must be after start"));
        }
    }
    Ok(())
}
impl Database {
    pub fn workflow_get(&self, input: WorkflowGetInput) -> CoreResult<WorkflowItem> {
        read_item(&*self.lock()?, input.kind, &input.id)
    }
    pub fn workflow_list(&self, input: WorkflowListInput) -> CoreResult<Vec<WorkflowItem>> {
        if input.limit == 0 || input.limit > 100 || input.offset > i64::MAX as usize {
            return Err(invalid(
                "pagination",
                "limit must be 1–100 and offset must fit i64",
            ));
        }
        if input.kind == WorkflowKind::Work && input.work_id.is_some() {
            return Err(invalid("work_id", "parent filter is only for tasks"));
        }
        let connection = self.lock()?;
        let sql = if input.kind == WorkflowKind::Task {
            "SELECT data_json FROM tasks WHERE (?3 IS NULL OR work_id=?3) ORDER BY updated_at DESC, id LIMIT ?1 OFFSET ?2".to_string()
        } else {
            "SELECT data_json FROM work_items ORDER BY updated_at DESC, id LIMIT ?1 OFFSET ?2"
                .to_string()
        };
        let mut stmt = connection.prepare(&sql).map_err(database_error)?;
        let mut bindings = vec![
            Value::Integer(input.limit as i64),
            Value::Integer(input.offset as i64),
        ];
        if input.kind == WorkflowKind::Task {
            bindings.push(input.work_id.map(Value::Text).unwrap_or(Value::Null));
        }
        let rows = stmt
            .query_map(params_from_iter(bindings), |r| r.get::<_, String>(0))
            .map_err(database_error)?;
        rows.map(|r| serde_json::from_str(&r.map_err(database_error)?).map_err(json_error))
            .collect()
    }
    pub fn workflow_save(
        &self,
        input: WorkflowSaveInput,
        trusted_review: bool,
    ) -> CoreResult<WorkflowMutation> {
        validate(&input)?;
        // This gate runs before replay lookup: an automated transport cannot replay a trusted completion.
        if input.kind == WorkflowKind::Work && input.fields.status == "done" && !trusted_review {
            return Err(invalid(
                "status",
                "work completion requires the trusted desktop review action",
            ));
        }
        let digest = request_digest(&input)?;
        let mut connection = self.lock()?;
        let tx = connection.transaction().map_err(database_error)?;
        if let Some(replay) =
            check_idempotency(&tx, &input.idempotency_key, "workflow.save", &digest)?
        {
            return Ok(replay);
        }
        let old = input
            .id
            .as_ref()
            .map(|id| read_item(&tx, input.kind, id))
            .transpose()?;
        if let Some(old) = &old {
            if input.expected_version != Some(old.version) {
                return Err(CoreError::VersionConflict {
                    entity: input.kind.table().into(),
                    id: old.id.clone(),
                    expected: input.expected_version.unwrap_or(0),
                    actual: old.version,
                });
            }
            if matches!(old.fields.status.as_str(), "done" | "cancelled")
                && old.fields.status != input.fields.status
                && !matches!(input.fields.status.as_str(), "planned" | "in_progress")
            {
                return Err(invalid(
                    "status",
                    "reopen to planned or in_progress before another transition",
                ));
            }
        }
        if input.kind == WorkflowKind::Work
            && input.fields.status == "done"
            && !old
                .as_ref()
                .is_some_and(|v| matches!(v.fields.status.as_str(), "review" | "done"))
        {
            return Err(invalid(
                "status",
                "move work to review before confirming completion",
            ));
        }
        if let Some(work_id) = &input.fields.work_id {
            read_item(&tx, WorkflowKind::Work, work_id)?;
        }
        let now = now_rfc3339();
        let item = WorkflowItem {
            id: old
                .as_ref()
                .map(|v| v.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            kind: input.kind,
            version: old.as_ref().map_or(1, |v| v.version + 1),
            fields: input.fields.clone(),
            created_at: old
                .as_ref()
                .map(|v| v.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now.clone(),
        };
        let raw = serde_json::to_string(&item).map_err(json_error)?;
        match input.kind {
            WorkflowKind::Work => tx.execute("INSERT INTO work_items(id,data_json,updated_at) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data_json=excluded.data_json,updated_at=excluded.updated_at", params![item.id,raw,now]),
            WorkflowKind::Task => tx.execute("INSERT INTO tasks(id,work_id,data_json,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET work_id=excluded.work_id,data_json=excluded.data_json,updated_at=excluded.updated_at", params![item.id,item.fields.work_id,raw,now]),
        }.map_err(database_error)?;
        tx.execute("INSERT INTO activity_events(id,target_id,target_kind,previous_state,next_state,version,occurred_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![Uuid::new_v4().to_string(),item.id,input.kind.table(),old.as_ref().map(|v| &v.fields.status),item.fields.status,item.version,now]).map_err(database_error)?;
        let result = WorkflowMutation {
            item,
            idempotent_replay: false,
        };
        store_idempotency(
            &tx,
            &input.idempotency_key,
            "workflow.save",
            &digest,
            &result,
        )?;
        tx.commit().map_err(database_error)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(kind: WorkflowKind, key: &str) -> WorkflowSaveInput {
        WorkflowSaveInput {
            kind,
            id: None,
            expected_version: None,
            idempotency_key: key.into(),
            fields: WorkflowFields {
                title: "동일한 제목".into(),
                status: "planned".into(),
                purpose: "결정과 행동 보존".into(),
                blocked_reason: None,
                work_id: None,
                target_date: Some("2026-09-23".into()),
                priority: 2,
                time_blocks: vec![],
            },
        }
    }
    fn update(item: &WorkflowItem, key: &str, status: &str) -> WorkflowSaveInput {
        let mut request = input(item.kind, key);
        request.id = Some(item.id.clone());
        request.expected_version = Some(item.version);
        request.fields = item.fields.clone();
        request.fields.status = status.into();
        request
    }
    fn list(kind: WorkflowKind) -> WorkflowListInput {
        WorkflowListInput {
            kind,
            work_id: None,
            limit: 100,
            offset: 0,
        }
    }
    #[test]
    fn replay_conflict_and_same_title_are_independent() {
        let db = Database::in_memory().unwrap();
        let request = input(WorkflowKind::Work, "create");
        let first = db.workflow_save(request.clone(), false).unwrap();
        let replay = db.workflow_save(request.clone(), false).unwrap();
        assert_eq!(first.item, replay.item);
        assert!(replay.idempotent_replay);
        let mut changed = request;
        changed.fields.title = "different".into();
        assert!(matches!(
            db.workflow_save(changed, false),
            Err(CoreError::IdempotencyConflict { .. })
        ));
        let second = db
            .workflow_save(input(WorkflowKind::Work, "other"), false)
            .unwrap();
        assert_ne!(first.item.id, second.item.id);
        assert_eq!(db.workflow_list(list(WorkflowKind::Work)).unwrap().len(), 2);
        let events: i64 = db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM activity_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 2);
    }
    #[test]
    fn stale_edit_and_completion_review_are_guarded() {
        let db = Database::in_memory().unwrap();
        let work = db
            .workflow_save(input(WorkflowKind::Work, "create"), false)
            .unwrap()
            .item;
        assert!(db
            .workflow_save(update(&work, "early", "done"), true)
            .is_err());
        let review = db
            .workflow_save(update(&work, "review", "review"), false)
            .unwrap()
            .item;
        assert!(matches!(
            db.workflow_save(update(&work, "stale", "in_progress"), false),
            Err(CoreError::VersionConflict { .. })
        ));
        let finish = update(&review, "finish", "done");
        assert!(db.workflow_save(finish.clone(), false).is_err());
        let done = db.workflow_save(finish.clone(), true).unwrap().item;
        assert!(db.workflow_save(finish, false).is_err());
        assert!(db
            .workflow_save(update(&done, "invalid-reopen", "review"), true)
            .is_err());
        let reopened = db
            .workflow_save(update(&done, "reopen", "in_progress"), false)
            .unwrap()
            .item;
        assert_eq!(reopened.version, 4);
    }
    #[test]
    fn competing_writers_commit_exactly_one_version() {
        let core = crate::core::Core::in_memory().unwrap();
        let original = core
            .workflow_save(input(WorkflowKind::Task, "create"))
            .unwrap()
            .item;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|index| {
                let core = core.clone();
                let barrier = barrier.clone();
                let request = update(&original, &format!("writer-{index}"), "in_progress");
                std::thread::spawn(move || {
                    barrier.wait();
                    core.workflow_save(request)
                })
            })
            .collect();
        barrier.wait();
        let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(CoreError::VersionConflict { .. })))
                .count(),
            1
        );
        assert_eq!(
            core.workflow_get(WorkflowGetInput {
                kind: WorkflowKind::Task,
                id: original.id
            })
            .unwrap()
            .version,
            2
        );
    }

    #[test]
    fn parent_and_dates_validated_without_partial_writes() {
        let db = Database::in_memory().unwrap();
        let mut request = input(WorkflowKind::Task, "task");
        request.fields.work_id = Some("missing".into());
        assert!(matches!(
            db.workflow_save(request.clone(), false),
            Err(CoreError::NotFound { .. })
        ));
        request.fields.work_id = None;
        request.fields.target_date = Some("2026-02-30".into());
        assert!(db.workflow_save(request.clone(), false).is_err());
        request.fields.target_date = None;
        request.fields.time_blocks = vec![TimeBlock {
            start: "2026-09-23T10:00:00+09:00".into(),
            end: "2026-09-23T01:00:00Z".into(),
        }];
        assert!(db.workflow_save(request.clone(), false).is_err());
        request.fields.time_blocks[0].end = "2026-09-23T02:00:00Z".into();
        assert!(db.workflow_save(request, false).is_ok()); // failed attempts did not consume key
    }
    #[test]
    fn event_failure_rolls_back_item_and_idempotency() {
        let db = Database::in_memory().unwrap();
        db.lock().unwrap().execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON activity_events BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        let request = input(WorkflowKind::Work, "atomic");
        assert!(db.workflow_save(request.clone(), false).is_err());
        assert!(db
            .workflow_list(list(WorkflowKind::Work))
            .unwrap()
            .is_empty());
        db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_event;")
            .unwrap();
        assert!(!db.workflow_save(request, false).unwrap().idempotent_replay);
    }
    #[test]
    fn migration_eight_failure_rolls_back_and_can_retry() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(migrate(&mut conn, Some(8)).is_err());
        let version: i64 = conn
            .query_row("SELECT max(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, 7);
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='work_items'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
        migrate(&mut conn, None).unwrap();
        conn.execute("INSERT INTO work_items VALUES ('probe','{}','now')", [])
            .unwrap();
    }
    #[test]
    fn reopen_database_preserves_tasks_blocks_and_backup() {
        let dir = std::env::temp_dir().join(format!("aidebook-workflow-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("core.sqlite");
        let backup = dir.join("backup.sqlite");
        let (work, task) = {
            let db = Database::open(&path).unwrap();
            let work = db
                .workflow_save(input(WorkflowKind::Work, "work"), false)
                .unwrap()
                .item;
            let mut request = input(WorkflowKind::Task, "task");
            request.fields.work_id = Some(work.id.clone());
            request.fields.time_blocks = vec![TimeBlock {
                start: "2026-09-23T10:00:00+09:00".into(),
                end: "2026-09-23T11:00:00+09:00".into(),
            }];
            let task = db.workflow_save(request, false).unwrap().item;
            db.backup_to(&backup).unwrap();
            (work, task)
        };
        let db = Database::open(&path).unwrap();
        assert_eq!(
            db.workflow_get(WorkflowGetInput {
                kind: WorkflowKind::Task,
                id: task.id.clone()
            })
            .unwrap(),
            task
        );
        let mut query = list(WorkflowKind::Task);
        query.work_id = Some(work.id.clone());
        assert_eq!(db.workflow_list(query).unwrap(), vec![task.clone()]);
        db.workflow_save(update(&task, "complete-task", "done"), false)
            .unwrap();
        assert_eq!(
            db.workflow_get(WorkflowGetInput {
                kind: WorkflowKind::Work,
                id: work.id
            })
            .unwrap()
            .fields
            .status,
            "planned"
        );
        db.restore_from(&backup).unwrap();
        assert_eq!(
            db.workflow_get(WorkflowGetInput {
                kind: WorkflowKind::Task,
                id: task.id
            })
            .unwrap()
            .fields
            .status,
            "planned"
        );
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn pagination_has_no_duplicate_ids_and_rejects_bad_limit() {
        let db = Database::in_memory().unwrap();
        for index in 0..5 {
            db.workflow_save(input(WorkflowKind::Task, &format!("{index}")), false)
                .unwrap();
        }
        let mut query = list(WorkflowKind::Task);
        query.limit = 2;
        let first = db.workflow_list(query.clone()).unwrap();
        query.offset = 2;
        let second = db.workflow_list(query.clone()).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
        assert!(first.iter().all(|a| second.iter().all(|b| a.id != b.id)));
        query.limit = 0;
        assert!(db.workflow_list(query).is_err());
    }
    #[test]
    fn ipc_and_core_share_records_without_trusted_completion() {
        let core = crate::core::Core::in_memory().unwrap();
        let created = crate::core::ipc_dispatch(
            &core,
            "workflow.save",
            serde_json::to_value(input(WorkflowKind::Work, "ipc")).unwrap(),
        )
        .unwrap();
        let item: WorkflowMutation = serde_json::from_value(created).unwrap();
        let listed =
            crate::core::ipc_dispatch(&core, "workflow.list", serde_json::json!({"kind":"work"}))
                .unwrap();
        assert_eq!(listed[0]["id"], item.item.id);
        assert!(crate::core::ipc_dispatch(
            &core,
            "workflow.save",
            serde_json::to_value(update(&item.item, "complete", "done")).unwrap()
        )
        .is_err());
    }
}
