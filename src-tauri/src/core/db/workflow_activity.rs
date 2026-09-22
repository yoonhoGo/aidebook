//! Read-only status/version history from the existing workflow activity log.
use super::workflow::WorkflowKind;
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowActivityInput {
    pub kind: WorkflowKind,
    pub id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}
fn default_limit() -> usize {
    20
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowActivityEvent {
    pub previous_state: Option<String>,
    pub next_state: String,
    pub version: i64,
    pub occurred_at: String,
}

impl Database {
    pub fn workflow_activity_list(
        &self,
        input: WorkflowActivityInput,
    ) -> CoreResult<Vec<WorkflowActivityEvent>> {
        if input.limit == 0 || input.limit > 100 || input.offset > i64::MAX as usize {
            return Err(CoreError::InvalidInput {
                field: "pagination".into(),
                message: "limit must be 1–100; offset must fit i64".into(),
            });
        }
        if input.id.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "id".into(),
                message: "target id must be nonempty".into(),
            });
        }
        let table = match input.kind {
            WorkflowKind::Work => "work_items",
            WorkflowKind::Task => "tasks",
        };
        let connection = self.lock()?;
        let exists: bool = connection
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1)"),
                [&input.id],
                |r| r.get(0),
            )
            .map_err(database_error)?;
        if !exists {
            return Err(CoreError::NotFound {
                entity: table.into(),
                id: input.id,
            });
        }
        let mut statement = connection.prepare("SELECT previous_state,next_state,version,occurred_at FROM activity_events WHERE target_kind=?1 AND target_id=?2 ORDER BY version DESC LIMIT ?3 OFFSET ?4").map_err(database_error)?;
        let rows = statement
            .query_map(
                params![table, input.id, input.limit as i64, input.offset as i64],
                |row| {
                    Ok(WorkflowActivityEvent {
                        previous_state: row.get(0)?,
                        next_state: row.get(1)?,
                        version: row.get(2)?,
                        occurred_at: row.get(3)?,
                    })
                },
            )
            .map_err(database_error)?;
        rows.map(|row| row.map_err(database_error)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::workflow::{WorkflowFields, WorkflowSaveInput};
    use super::*;
    fn request() -> WorkflowSaveInput {
        WorkflowSaveInput {
            kind: WorkflowKind::Task,
            id: None,
            expected_version: None,
            idempotency_key: "create".into(),
            fields: WorkflowFields {
                title: "history".into(),
                status: "planned".into(),
                purpose: String::new(),
                blocked_reason: None,
                work_id: None,
                target_date: None,
                priority: 0,
                pinned: false,
                time_blocks: vec![],
            },
        }
    }
    fn query(id: &str) -> WorkflowActivityInput {
        WorkflowActivityInput {
            kind: WorkflowKind::Task,
            id: id.into(),
            limit: 20,
            offset: 0,
        }
    }
    #[test]
    fn history_keeps_edits_completion_reopen_and_pagination() {
        let db = Database::in_memory().unwrap();
        let mut req = request();
        let mut item = db.workflow_save(req.clone(), false).unwrap().item;
        for (key, state) in [
            ("edit", "planned"),
            ("done", "done"),
            ("reopen", "in_progress"),
        ] {
            req.id = Some(item.id.clone());
            req.expected_version = Some(item.version);
            req.idempotency_key = key.into();
            req.fields.status = state.into();
            item = db.workflow_save(req.clone(), false).unwrap().item;
        }
        // An idempotent retry must not add another activity.
        db.workflow_save(req, false).unwrap();
        let events = db.workflow_activity_list(query(&item.id)).unwrap();
        assert_eq!(
            events.iter().map(|e| e.version).collect::<Vec<_>>(),
            vec![4, 3, 2, 1]
        );
        assert_eq!(events[0].previous_state.as_deref(), Some("done"));
        assert_eq!(events[0].next_state, "in_progress");
        assert_eq!(events[1].next_state, "done");
        assert_eq!(events[2].previous_state.as_deref(), Some("planned"));
        assert_eq!(events[2].next_state, "planned");
        assert_eq!(events[3].previous_state, None);
        assert!(events
            .iter()
            .all(|e| chrono::DateTime::parse_from_rfc3339(&e.occurred_at).is_ok()));
        let mut page = query(&item.id);
        page.limit = 2;
        page.offset = 2;
        assert_eq!(db.workflow_activity_list(page).unwrap(), events[2..]);
        let mut page = query(&item.id);
        page.offset = 4;
        assert!(db.workflow_activity_list(page).unwrap().is_empty());
        let mut wrong_kind = query(&item.id);
        wrong_kind.kind = WorkflowKind::Work;
        assert!(matches!(
            db.workflow_activity_list(wrong_kind),
            Err(CoreError::NotFound { .. })
        ));
    }
    #[test]
    fn history_rejects_invalid_targets_and_pagination() {
        let db = Database::in_memory().unwrap();
        assert!(matches!(
            db.workflow_activity_list(query("missing")),
            Err(CoreError::NotFound { .. })
        ));
        assert!(matches!(
            db.workflow_activity_list(query(" ")),
            Err(CoreError::InvalidInput { .. })
        ));
        for (limit, offset) in [(0, 0), (101, 0), (20, usize::MAX)] {
            let mut q = query("missing");
            q.limit = limit;
            q.offset = offset;
            assert!(matches!(
                db.workflow_activity_list(q),
                Err(CoreError::InvalidInput { .. })
            ));
        }
        let q: WorkflowActivityInput = serde_json::from_str(r#"{"kind":"work","id":"x"}"#).unwrap();
        assert_eq!(q.limit, 20);
        assert_eq!(q.offset, 0);
    }
}
