//! Derived dashboard, never a second copy of workflow state.
use super::workflow::{WorkflowItem, WorkflowKind};
use super::*;
use chrono::{DateTime, Duration};
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DashboardSection {
    Today,
    Next,
    Attention,
    InProgress,
    Completed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardInput {
    pub timezone: String,
    /// Optional deterministic reference instant. Omit to use the Core clock.
    pub now: Option<String>,
    pub section: DashboardSection,
    #[serde(default = "page_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    pub kind: Option<WorkflowKind>,
}
fn page_limit() -> usize {
    20
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardEntry {
    pub entry_id: String,
    pub item: WorkflowItem,
    pub reasons: Vec<String>,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub completed_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardPage {
    pub date: String,
    pub timezone: String,
    pub generated_at: String,
    pub section: DashboardSection,
    pub entries: Vec<DashboardEntry>,
    pub total: usize,
    pub has_more: bool,
    pub calendar_connected: bool,
}
fn invalid(field: &str, message: &str) -> CoreError {
    CoreError::InvalidInput {
        field: field.into(),
        message: message.into(),
    }
}
fn parse_time(raw: &str) -> CoreResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| invalid("timestamp", "expected RFC3339 timestamp with offset"))
}
fn decode(raw: String) -> CoreResult<WorkflowItem> {
    serde_json::from_str(&raw).map_err(|e| CoreError::Database {
        message: e.to_string(),
    })
}
fn active(item: &WorkflowItem) -> bool {
    !matches!(
        item.fields.status.as_str(),
        "done" | "cancelled" | "on_hold"
    )
}
fn action_rank(
    item: &WorkflowItem,
    today: &str,
    soon: &str,
) -> (u8, u8, std::cmp::Reverse<u8>, String, String) {
    let due = item.fields.target_date.as_deref();
    let urgency = match due {
        Some(d) if d < today => 0,
        Some(d) if d <= soon => 1,
        _ => 2,
    };
    (
        u8::from(!item.fields.pinned),
        urgency,
        std::cmp::Reverse(item.fields.priority),
        due.unwrap_or("9999-12-31").to_owned(),
        item.id.clone(),
    )
}
fn base_entry(item: &WorkflowItem, reasons: Vec<String>) -> DashboardEntry {
    DashboardEntry {
        entry_id: format!(
            "{}:{}",
            if item.kind == WorkflowKind::Work {
                "work"
            } else {
                "task"
            },
            item.id
        ),
        item: item.clone(),
        reasons,
        starts_at: None,
        ends_at: None,
        completed_at: None,
    }
}
impl Database {
    pub fn dashboard_get(&self, input: DashboardInput) -> CoreResult<DashboardPage> {
        if input.limit == 0 || input.limit > 100 || input.offset > i64::MAX as usize {
            return Err(invalid(
                "pagination",
                "limit must be 1–100; offset must fit i64",
            ));
        }
        let tz: Tz = input
            .timezone
            .parse()
            .map_err(|_| invalid("timezone", "expected a supported IANA timezone"))?;
        let now = input
            .now
            .as_deref()
            .map(parse_time)
            .transpose()?
            .unwrap_or_else(Utc::now);
        let today = now.with_timezone(&tz).date_naive();
        let date = today.to_string();
        let soon = today
            .checked_add_signed(Duration::days(3))
            .ok_or_else(|| invalid("now", "date outside supported range"))?
            .to_string();
        let connection = self.lock()?;
        // One connection lock gives a consistent workflow snapshot across both tables.
        let mut statement = connection
            .prepare("SELECT data_json FROM work_items UNION ALL SELECT data_json FROM tasks")
            .map_err(database_error)?;
        let items: Vec<WorkflowItem> = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(database_error)?
            .map(|r| decode(r.map_err(database_error)?))
            .collect::<CoreResult<_>>()?;
        let parents: HashMap<&str, &WorkflowItem> = items
            .iter()
            .filter(|i| i.kind == WorkflowKind::Work)
            .map(|i| (i.id.as_str(), i))
            .collect();
        let mut completion = HashMap::new();
        if input.section == DashboardSection::Completed {
            let mut statement = connection.prepare("SELECT target_id, target_kind, MAX(occurred_at) FROM activity_events WHERE next_state='done' AND (previous_state IS NULL OR previous_state<>'done') GROUP BY target_id,target_kind").map_err(database_error)?;
            for row in statement
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(database_error)?
            {
                let (id, kind, time) = row.map_err(database_error)?;
                completion.insert((id, kind), time);
            }
        }
        let mut entries = Vec::new();
        for item in &items {
            if input.kind.is_some_and(|k| item.kind != k) {
                continue;
            }
            if item.fields.status == "cancelled" {
                continue;
            }
            let parent_inactive = item
                .fields
                .work_id
                .as_deref()
                .and_then(|id| parents.get(id))
                .is_some_and(|p| !active(p));
            let runnable = active(item)
                && !parent_inactive
                && item.fields.blocked_reason.is_none()
                && item.fields.status != "review";
            let overdue = item
                .fields
                .target_date
                .as_deref()
                .is_some_and(|d| d < date.as_str());
            match input.section {
                DashboardSection::Today if active(item) && !parent_inactive => {
                    for (index, block) in item.fields.time_blocks.iter().enumerate() {
                        let start = parse_time(&block.start)?;
                        let end = parse_time(&block.end)?;
                        // Compare local calendar dates, not a fixed 24h duration (DST safe).
                        let first_day = start.with_timezone(&tz).date_naive();
                        let last_day = (end - Duration::nanoseconds(1))
                            .with_timezone(&tz)
                            .date_naive();
                        if first_day <= today && last_day >= today {
                            let mut entry = base_entry(item, vec!["scheduled".into()]);
                            entry.entry_id.push_str(&format!(":block:{index}"));
                            entry.starts_at = Some(start.to_rfc3339());
                            entry.ends_at = Some(end.to_rfc3339());
                            entries.push(entry);
                        }
                    }
                    if item.fields.target_date.as_deref() == Some(date.as_str()) {
                        let mut entry = base_entry(item, vec!["due_today".into()]);
                        entry.entry_id.push_str(":due");
                        entries.push(entry);
                    }
                }
                DashboardSection::Next if runnable => {
                    let mut reasons = Vec::new();
                    if item.fields.pinned {
                        reasons.push("pinned".into());
                    }
                    if overdue {
                        reasons.push("overdue".into());
                    } else if item
                        .fields
                        .target_date
                        .as_deref()
                        .is_some_and(|d| d <= soon.as_str())
                    {
                        reasons.push("due_soon".into());
                    }
                    if item.fields.priority > 0 {
                        reasons.push(format!("priority_{}", item.fields.priority));
                    }
                    if reasons.is_empty() {
                        reasons.push("ready".into());
                    }
                    entries.push(base_entry(item, reasons));
                }
                DashboardSection::Attention if item.fields.status != "done" => {
                    let mut reasons = Vec::new();
                    if item.fields.status == "on_hold" {
                        reasons.push("on_hold".into());
                    }
                    if parent_inactive {
                        reasons.push("parent_inactive".into());
                    }
                    if item.fields.blocked_reason.is_some() {
                        reasons.push("blocked".into());
                    }
                    if item.fields.status == "review" {
                        reasons.push("review".into());
                    }
                    if overdue && active(item) && !parent_inactive {
                        reasons.push("overdue".into());
                    }
                    if !reasons.is_empty() {
                        entries.push(base_entry(item, reasons));
                    }
                }
                DashboardSection::InProgress
                    if item.kind == WorkflowKind::Work && item.fields.status == "in_progress" =>
                {
                    entries.push(base_entry(item, vec!["in_progress".into()]));
                }
                DashboardSection::Completed if item.fields.status == "done" => {
                    let key = (
                        item.id.clone(),
                        if item.kind == WorkflowKind::Work {
                            "work_items"
                        } else {
                            "tasks"
                        }
                        .to_owned(),
                    );
                    if let Some(raw) = completion.get(&key) {
                        let completed = parse_time(raw)?;
                        let days = today
                            .signed_duration_since(completed.with_timezone(&tz).date_naive())
                            .num_days();
                        if (0..7).contains(&days) && completed <= now {
                            let mut entry = base_entry(item, vec!["completed".into()]);
                            entry.completed_at = Some(completed.to_rfc3339());
                            entries.push(entry);
                        }
                    }
                }
                _ => (),
            }
        }
        match input.section {
            DashboardSection::Today => entries.sort_by(|a, b| {
                (a.starts_at.is_none(), &a.starts_at, &a.entry_id).cmp(&(
                    b.starts_at.is_none(),
                    &b.starts_at,
                    &b.entry_id,
                ))
            }),
            DashboardSection::Completed => entries.sort_by(|a, b| {
                b.completed_at
                    .cmp(&a.completed_at)
                    .then_with(|| a.entry_id.cmp(&b.entry_id))
            }),
            _ => entries.sort_by_key(|e| action_rank(&e.item, &date, &soon)),
        }
        let total = entries.len();
        let page_entries = entries
            .into_iter()
            .skip(input.offset)
            .take(input.limit)
            .collect();
        Ok(DashboardPage {
            date,
            timezone: input.timezone,
            generated_at: now.to_rfc3339(),
            section: input.section,
            entries: page_entries,
            total,
            has_more: input.offset.saturating_add(input.limit) < total,
            calendar_connected: false,
        })
    }
}
