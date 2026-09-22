//! Explicit references and non-destructive legacy grouping import.
use super::workflow::{WorkflowFields, WorkflowItem, WorkflowKind};
use super::*;
pub(super) const SCHEMA: &str = r#"
CREATE TABLE work_links(id TEXT PRIMARY KEY,work_id TEXT NOT NULL REFERENCES work_items(id),target_kind TEXT NOT NULL,target_id TEXT NOT NULL,relation_type TEXT NOT NULL,data_json TEXT NOT NULL,UNIQUE(work_id,target_kind,target_id,relation_type));
CREATE INDEX idx_work_links_work ON work_links(work_id,id);
CREATE TABLE legacy_work_mappings(namespace TEXT NOT NULL,legacy_work_index INTEGER NOT NULL,work_id TEXT NOT NULL REFERENCES work_items(id),PRIMARY KEY(namespace,legacy_work_index));
"#;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTargetKind {
    Source,
    Memory,
}
impl WorkflowTargetKind {
    fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Memory => "memory",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLinkAddInput {
    pub work_id: String,
    pub target_kind: WorkflowTargetKind,
    #[serde(default)]
    pub target_id: String,
    #[serde(default)]
    pub target_source: Option<SourceRef>,
    pub relation_type: String,
    pub reason: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub expected_version: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLinkRemoveInput {
    pub id: String,
    pub expected_version: i64,
    pub idempotency_key: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLinkListInput {
    pub work_id: String,
    #[serde(default)]
    pub include_removed: bool,
    #[serde(default = "limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}
fn limit() -> usize {
    100
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowLink {
    pub id: String,
    pub work_id: String,
    pub target_kind: WorkflowTargetKind,
    pub target_id: String,
    pub relation_type: String,
    pub reason: String,
    pub author: String,
    pub confirmed: bool,
    pub version: i64,
    pub created_at: String,
    pub removed_at: Option<String>,
    pub access_status: String,
    #[serde(default)]
    pub target_title: Option<String>,
    #[serde(default)]
    pub target_source: Option<SourceRef>,
    #[serde(default)]
    pub fetched_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowLinkMutation {
    pub link: WorkflowLink,
    pub idempotent_replay: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyWorkGroup {
    pub legacy_work_index: i64,
    pub title: String,
    #[serde(default)]
    pub native_memory_ids: Vec<String>,
    #[serde(default)]
    pub unmigrated_note_count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowImportInput {
    pub namespace: String,
    pub groups: Vec<LegacyWorkGroup>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowImportApplyInput {
    pub namespace: String,
    pub groups: Vec<LegacyWorkGroup>,
    pub idempotency_key: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowImportSkipped {
    pub memory_id: Option<String>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowImportGroupPreview {
    pub legacy_work_index: i64,
    pub title: String,
    pub existing_work_id: Option<String>,
    pub linkable_memory_ids: Vec<String>,
    pub skipped: Vec<WorkflowImportSkipped>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowImportPreview {
    pub groups: Vec<WorkflowImportGroupPreview>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowImportResult {
    pub works: Vec<WorkflowItem>,
    pub linked_count: usize,
    pub skipped: Vec<WorkflowImportSkipped>,
    pub idempotent_replay: bool,
}
fn invalid(field: &str, message: &str) -> CoreError {
    CoreError::InvalidInput {
        field: field.into(),
        message: message.into(),
    }
}
fn json_error(e: serde_json::Error) -> CoreError {
    CoreError::Database {
        message: e.to_string(),
    }
}
fn key(k: &str) -> CoreResult<()> {
    if k.trim().is_empty() || k.len() > 200 {
        Err(invalid("idempotency_key", "requires 1–200 bytes"))
    } else {
        Ok(())
    }
}
fn work(c: &Connection, id: &str) -> CoreResult<WorkflowItem> {
    let raw: Option<String> = c
        .query_row("SELECT data_json FROM work_items WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(database_error)?;
    serde_json::from_str(&raw.ok_or_else(|| CoreError::NotFound {
        entity: "work".into(),
        id: id.into(),
    })?)
    .map_err(json_error)
}
// Current evidence controls visibility; stored link JSON never contains copied source text.
fn accessible(c: &Connection, kind: WorkflowTargetKind, id: &str) -> CoreResult<bool> {
    let sql=match kind {WorkflowTargetKind::Source=>"SELECT EXISTS(SELECT 1 FROM snapshots WHERE source_id=?1 AND access_status='accessible' AND is_deleted=0)",WorkflowTargetKind::Memory=>"SELECT EXISTS(SELECT 1 FROM memories m WHERE m.id=?1 AND m.retracted_at IS NULL AND NOT EXISTS(SELECT 1 FROM memory_evidence e LEFT JOIN snapshots s ON s.source_id=e.source_id WHERE e.memory_id=m.id AND (s.source_id IS NULL OR s.access_status!='accessible' OR s.is_deleted!=0)))"};
    c.query_row(sql, [id], |r| r.get(0)).map_err(database_error)
}
fn strip_presentation(l: &mut WorkflowLink) {
    l.target_title = None;
    l.target_source = None;
    l.fetched_at = None;
}
fn refresh(c: &Connection, mut l: WorkflowLink) -> CoreResult<WorkflowLink> {
    strip_presentation(&mut l);
    l.access_status = if accessible(c, l.target_kind, &l.target_id)? {
        "accessible"
    } else {
        "unavailable"
    }
    .into();
    if l.access_status == "accessible" {
        match l.target_kind {
            WorkflowTargetKind::Source => {
                let (title, fetched, source) = c.query_row("SELECT sn.title,sn.fetched_at,s.provider,s.account_id,s.external_id,s.url,s.kind FROM sources s JOIN snapshots sn ON sn.source_id=s.id WHERE s.id=?1", [&l.target_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,SourceRef{provider:r.get(2)?,account_id:r.get(3)?,external_id:r.get(4)?,url:r.get(5)?,kind:r.get(6)?}))).map_err(database_error)?;
                l.target_title = Some(title);
                l.fetched_at = Some(fetched);
                l.target_source = Some(source);
            }
            WorkflowTargetKind::Memory => {
                l.target_title = Some(
                    c.query_row(
                        "SELECT title FROM ui_memories WHERE memory_id=?1",
                        [&l.target_id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(database_error)?
                    .unwrap_or_else(|| "Memory".into()),
                );
            }
        }
    }
    Ok(l)
}
fn link_by_id(c: &Connection, id: &str) -> CoreResult<WorkflowLink> {
    let raw: Option<String> = c
        .query_row("SELECT data_json FROM work_links WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(database_error)?;
    refresh(
        c,
        serde_json::from_str(&raw.ok_or_else(|| CoreError::NotFound {
            entity: "work_link".into(),
            id: id.into(),
        })?)
        .map_err(json_error)?,
    )
}
fn write_link(c: &Connection, l: &WorkflowLink) -> CoreResult<()> {
    let mut stored = l.clone();
    strip_presentation(&mut stored);
    c.execute("INSERT INTO work_links(id,work_id,target_kind,target_id,relation_type,data_json) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET data_json=excluded.data_json",params![l.id,l.work_id,l.target_kind.name(),l.target_id,l.relation_type,serde_json::to_string(&stored).map_err(json_error)?]).map_err(database_error)?;
    Ok(())
}
fn add(c: &Connection, i: &WorkflowLinkAddInput) -> CoreResult<WorkflowLink> {
    work(c, &i.work_id)?;
    if !accessible(c, i.target_kind, &i.target_id)? {
        return Err(invalid(
            "target_id",
            "target is missing, retracted, or unavailable",
        ));
    }
    let prior:Option<String>=c.query_row("SELECT id FROM work_links WHERE work_id=?1 AND target_kind=?2 AND target_id=?3 AND relation_type=?4",params![i.work_id,i.target_kind.name(),i.target_id,i.relation_type],|r|r.get(0)).optional().map_err(database_error)?;
    let mut l = if let Some(id) = prior {
        let old = link_by_id(c, &id)?;
        if i.expected_version.is_some() && i.expected_version != Some(old.version)
            || old.removed_at.is_some() && i.expected_version != Some(old.version)
        {
            return Err(CoreError::VersionConflict {
                entity: "work_link".into(),
                id,
                expected: i.expected_version.unwrap_or(0),
                actual: old.version,
            });
        }
        if old.removed_at.is_none() {
            return Ok(old);
        }
        WorkflowLink {
            version: old.version + 1,
            removed_at: None,
            reason: i.reason.clone(),
            ..old
        }
    } else {
        if i.expected_version.is_some() {
            return Err(invalid("expected_version", "new links must omit version"));
        }
        WorkflowLink {
            id: Uuid::new_v4().to_string(),
            work_id: i.work_id.clone(),
            target_kind: i.target_kind,
            target_id: i.target_id.clone(),
            relation_type: i.relation_type.clone(),
            reason: i.reason.clone(),
            author: "explicit_request".into(),
            confirmed: true,
            version: 1,
            created_at: now_rfc3339(),
            removed_at: None,
            access_status: "accessible".into(),
            target_title: None,
            target_source: None,
            fetched_at: None,
        }
    };
    l.access_status = "accessible".into();
    write_link(c, &l)?;
    refresh(c, l)
}
fn validate_import(i: &WorkflowImportInput) -> CoreResult<()> {
    if i.namespace.trim().is_empty()
        || i.namespace.len() > 200
        || i.groups.is_empty()
        || i.groups.len() > 100
    {
        return Err(invalid(
            "import",
            "namespace requires 1–200 bytes and groups 1–100",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for g in &i.groups {
        if g.legacy_work_index < 0
            || !seen.insert(g.legacy_work_index)
            || g.title.trim().is_empty()
            || g.title.len() > 1000
            || g.native_memory_ids.len() > 1000
            || g.native_memory_ids.iter().any(|s| s.len() > 200)
        {
            return Err(invalid("groups","unique nonnegative indices, title 1–1000 bytes and at most 1000 bounded memory IDs required"));
        }
    }
    Ok(())
}
fn preview(c: &Connection, i: &WorkflowImportInput) -> CoreResult<WorkflowImportPreview> {
    validate_import(i)?;
    let mut groups = vec![];
    for g in &i.groups {
        let existing_work_id=c.query_row("SELECT work_id FROM legacy_work_mappings WHERE namespace=?1 AND legacy_work_index=?2",params![i.namespace,g.legacy_work_index],|r|r.get(0)).optional().map_err(database_error)?;
        let mut p = WorkflowImportGroupPreview {
            legacy_work_index: g.legacy_work_index,
            title: g.title.clone(),
            existing_work_id,
            linkable_memory_ids: vec![],
            skipped: vec![],
        };
        for id in &g.native_memory_ids {
            if accessible(c, WorkflowTargetKind::Memory, id)? {
                if !p.linkable_memory_ids.contains(id) {
                    p.linkable_memory_ids.push(id.clone());
                }
            } else {
                p.skipped.push(WorkflowImportSkipped {
                    memory_id: Some(id.clone()),
                    reason: "missing, retracted, or inaccessible canonical memory".into(),
                });
            }
        }
        if g.unmigrated_note_count > 0 {
            p.skipped.push(WorkflowImportSkipped {
                memory_id: None,
                reason: format!(
                    "{} browser notes require separate explicit memory import",
                    g.unmigrated_note_count
                ),
            });
        }
        groups.push(p);
    }
    Ok(WorkflowImportPreview { groups })
}
impl Database {
    pub fn workflow_link_add(
        &self,
        mut i: WorkflowLinkAddInput,
    ) -> CoreResult<WorkflowLinkMutation> {
        key(&i.idempotency_key)?;
        if ![
            "context",
            "meeting_minutes",
            "specification",
            "implements",
            "verification",
            "completion_record",
        ]
        .contains(&i.relation_type.as_str())
            || i.reason.trim().is_empty()
            || i.reason.len() > 2000
        {
            return Err(invalid(
                "link",
                "supported relation type and reason 1–2000 bytes required",
            ));
        }
        let digest = request_digest(&i)?;
        let mut c = self.lock()?;
        let tx = c.transaction().map_err(database_error)?;
        if let Some(mut r) = check_idempotency::<WorkflowLinkMutation>(
            &tx,
            &i.idempotency_key,
            "work_link.add",
            &digest,
        )? {
            r.link = refresh(&tx, r.link)?;
            return Ok(r);
        }
        if let Some(source) = &i.target_source {
            if i.target_kind != WorkflowTargetKind::Source || !i.target_id.is_empty() {
                return Err(invalid(
                    "target_source",
                    "use exactly one source reference or target ID",
                ));
            }
            i.target_id = find_source_id(&tx, source)?
                .ok_or_else(|| invalid("target_source", "source has not been indexed"))?;
        } else if i.target_id.trim().is_empty() {
            return Err(invalid("target_id", "target required"));
        }
        let r = WorkflowLinkMutation {
            link: add(&tx, &i)?,
            idempotent_replay: false,
        };
        let mut stored = r.clone();
        strip_presentation(&mut stored.link);
        store_idempotency(&tx, &i.idempotency_key, "work_link.add", &digest, &stored)?;
        tx.commit().map_err(database_error)?;
        Ok(r)
    }
    pub fn workflow_link_remove(
        &self,
        i: WorkflowLinkRemoveInput,
    ) -> CoreResult<WorkflowLinkMutation> {
        key(&i.idempotency_key)?;
        let digest = request_digest(&i)?;
        let mut c = self.lock()?;
        let tx = c.transaction().map_err(database_error)?;
        if let Some(mut r) = check_idempotency::<WorkflowLinkMutation>(
            &tx,
            &i.idempotency_key,
            "work_link.remove",
            &digest,
        )? {
            r.link = refresh(&tx, r.link)?;
            return Ok(r);
        }
        let mut l = link_by_id(&tx, &i.id)?;
        if l.version != i.expected_version {
            return Err(CoreError::VersionConflict {
                entity: "work_link".into(),
                id: i.id,
                expected: i.expected_version,
                actual: l.version,
            });
        }
        if l.removed_at.is_none() {
            l.removed_at = Some(now_rfc3339());
            l.version += 1;
            write_link(&tx, &l)?;
        }
        let r = WorkflowLinkMutation {
            link: l,
            idempotent_replay: false,
        };
        let mut stored = r.clone();
        strip_presentation(&mut stored.link);
        store_idempotency(
            &tx,
            &i.idempotency_key,
            "work_link.remove",
            &digest,
            &stored,
        )?;
        tx.commit().map_err(database_error)?;
        Ok(r)
    }
    pub fn workflow_link_list(&self, i: WorkflowLinkListInput) -> CoreResult<Vec<WorkflowLink>> {
        if i.limit == 0 || i.limit > 100 || i.offset > i64::MAX as usize {
            return Err(invalid(
                "pagination",
                "limit 1–100 and offset fitting i64 required",
            ));
        }
        let c = self.lock()?;
        work(&c, &i.work_id)?;
        let mut s=c.prepare("SELECT data_json FROM work_links WHERE work_id=?1 AND (?2 OR json_extract(data_json,'$.removed_at') IS NULL) ORDER BY id LIMIT ?3 OFFSET ?4").map_err(database_error)?;
        let rows = s
            .query_map(
                params![
                    i.work_id,
                    i.include_removed,
                    i.limit as i64,
                    i.offset as i64
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(database_error)?;
        rows.map(|r| {
            refresh(
                &c,
                serde_json::from_str(&r.map_err(database_error)?).map_err(json_error)?,
            )
        })
        .collect()
    }
    pub fn workflow_import_preview(
        &self,
        i: WorkflowImportInput,
    ) -> CoreResult<WorkflowImportPreview> {
        preview(&*self.lock()?, &i)
    }
    pub fn workflow_import_apply(
        &self,
        i: WorkflowImportApplyInput,
    ) -> CoreResult<WorkflowImportResult> {
        key(&i.idempotency_key)?;
        let input = WorkflowImportInput {
            namespace: i.namespace.clone(),
            groups: i.groups.clone(),
        };
        validate_import(&input)?;
        let digest = request_digest(&i)?;
        let mut c = self.lock()?;
        let tx = c.transaction().map_err(database_error)?;
        if let Some(r) = check_idempotency(&tx, &i.idempotency_key, "workflow.import", &digest)? {
            return Ok(r);
        }
        let p = preview(&tx, &input)?;
        let mut r = WorkflowImportResult {
            works: vec![],
            linked_count: 0,
            skipped: vec![],
            idempotent_replay: false,
        };
        for g in p.groups {
            let w = if let Some(id) = g.existing_work_id {
                work(&tx, &id)?
            } else {
                let now = now_rfc3339();
                let w = WorkflowItem {
                    id: Uuid::new_v4().to_string(),
                    kind: WorkflowKind::Work,
                    version: 1,
                    fields: WorkflowFields {
                        title: g.title,
                        status: "planned".into(),
                        purpose: String::new(),
                        blocked_reason: None,
                        work_id: None,
                        target_date: None,
                        priority: 0,
                        pinned: false,
                        time_blocks: vec![],
                    },
                    created_at: now.clone(),
                    updated_at: now.clone(),
                };
                tx.execute(
                    "INSERT INTO work_items VALUES(?1,?2,?3)",
                    params![w.id, serde_json::to_string(&w).map_err(json_error)?, now],
                )
                .map_err(database_error)?;
                tx.execute(
                    "INSERT INTO legacy_work_mappings VALUES(?1,?2,?3)",
                    params![i.namespace, g.legacy_work_index, w.id],
                )
                .map_err(database_error)?;
                tx.execute(
                    "INSERT INTO activity_events VALUES(?1,?2,'work_items',NULL,'planned',1,?3)",
                    params![Uuid::new_v4().to_string(), w.id, now],
                )
                .map_err(database_error)?;
                w
            };
            for id in g.linkable_memory_ids {
                let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM work_links WHERE work_id=?1 AND target_kind='memory' AND target_id=?2 AND relation_type='context')",params![w.id,id],|r|r.get(0)).map_err(database_error)?;
                if !exists {
                    add(
                        &tx,
                        &WorkflowLinkAddInput {
                            work_id: w.id.clone(),
                            target_kind: WorkflowTargetKind::Memory,
                            target_id: id,
                            target_source: None,
                            relation_type: "context".into(),
                            reason: "Explicit legacy work group import".into(),
                            idempotency_key: i.idempotency_key.clone(),
                            expected_version: None,
                        },
                    )?;
                    r.linked_count += 1;
                }
            }
            r.skipped.extend(g.skipped);
            r.works.push(w);
        }
        store_idempotency(&tx, &i.idempotency_key, "workflow.import", &digest, &r)?;
        tx.commit().map_err(database_error)?;
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn group() -> LegacyWorkGroup {
        LegacyWorkGroup {
            legacy_work_index: 0,
            title: "same title".into(),
            native_memory_ids: vec!["m".into(), "missing".into()],
            unmigrated_note_count: 2,
        }
    }
    fn request(key: &str) -> WorkflowImportApplyInput {
        WorkflowImportApplyInput {
            namespace: "browser-v1".into(),
            groups: vec![group()],
            idempotency_key: key.into(),
        }
    }
    fn seed(db: &Database) {
        db.lock().unwrap().execute_batch("INSERT INTO sources VALUES('s','obsidian','vault','note','obsidian://note','note','now'); INSERT INTO snapshots(source_id,title,body,fetched_at,content_hash) VALUES('s','private title','private body','now','hash'); INSERT INTO memories(id,body,reason,author,claim_type,version,idempotency_key,idempotency_digest,created_at,updated_at) VALUES('m','memory','reason','user','fact',1,'seed','seed','now','now'); INSERT INTO memory_evidence VALUES('m','s');").unwrap();
    }
    fn list(db: &Database, id: &str) -> Vec<WorkflowLink> {
        db.workflow_link_list(WorkflowLinkListInput {
            work_id: id.into(),
            include_removed: true,
            limit: 100,
            offset: 0,
        })
        .unwrap()
    }
    #[test]
    fn preview_import_mapping_replay_preserves_edits_and_originals() {
        let db = Database::in_memory().unwrap();
        seed(&db);
        let p = db
            .workflow_import_preview(WorkflowImportInput {
                namespace: "browser-v1".into(),
                groups: vec![group()],
            })
            .unwrap();
        assert_eq!(p.groups[0].linkable_memory_ids, vec!["m"]);
        assert_eq!(p.groups[0].skipped.len(), 2);
        let r = db.workflow_import_apply(request("first")).unwrap();
        assert_eq!(r.linked_count, 1);
        assert!(
            db.workflow_import_apply(request("first"))
                .unwrap()
                .idempotent_replay
        );
        let mut fields = r.works[0].fields.clone();
        fields.title = "user edited".into();
        db.workflow_save(
            super::super::workflow::WorkflowSaveInput {
                kind: WorkflowKind::Work,
                id: Some(r.works[0].id.clone()),
                expected_version: Some(1),
                idempotency_key: "edit".into(),
                fields,
            },
            false,
        )
        .unwrap();
        let again = db.workflow_import_apply(request("new-key")).unwrap();
        assert_eq!(again.works[0].fields.title, "user edited");
        assert_eq!(again.works[0].version, 2);
        assert_eq!(again.linked_count, 0);
        let mut changed = request("first");
        changed.groups[0].title = "changed".into();
        assert!(matches!(
            db.workflow_import_apply(changed),
            Err(CoreError::IdempotencyConflict { .. })
        ));
        let mut other = request("other");
        other.namespace = "different browser".into();
        assert_ne!(
            db.workflow_import_apply(other).unwrap().works[0].id,
            r.works[0].id
        );
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row("SELECT body FROM memories WHERE id='m'", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "memory"
        );
    }
    #[test]
    fn revoked_evidence_rechecked_and_remove_version_guarded() {
        let db = Database::in_memory().unwrap();
        seed(&db);
        let r = db.workflow_import_apply(request("first")).unwrap();
        let l = list(&db, &r.works[0].id)[0].clone();
        let add = WorkflowLinkAddInput {
            work_id: l.work_id.clone(),
            target_kind: WorkflowTargetKind::Memory,
            target_id: "m".into(),
            target_source: None,
            relation_type: "context".into(),
            reason: "explicit".into(),
            idempotency_key: "add".into(),
            expected_version: None,
        };
        assert_eq!(db.workflow_link_add(add.clone()).unwrap().link.id, l.id);
        let remove = WorkflowLinkRemoveInput {
            id: l.id.clone(),
            expected_version: 1,
            idempotency_key: "remove".into(),
        };
        let removed = db.workflow_link_remove(remove.clone()).unwrap();
        assert_eq!(removed.link.version, 2);
        assert!(db.workflow_link_remove(remove).unwrap().idempotent_replay);
        assert!(matches!(
            db.workflow_link_remove(WorkflowLinkRemoveInput {
                id: l.id,
                expected_version: 1,
                idempotency_key: "stale".into()
            }),
            Err(CoreError::VersionConflict { .. })
        ));
        let mut restore = add.clone();
        restore.idempotency_key = "restore".into();
        assert!(db.workflow_link_add(restore.clone()).is_err());
        restore.expected_version = Some(2);
        assert_eq!(db.workflow_link_add(restore).unwrap().link.version, 3);
        db.lock()
            .unwrap()
            .execute(
                "UPDATE snapshots SET access_status='permission_denied' WHERE source_id='s'",
                [],
            )
            .unwrap();
        assert_eq!(list(&db, &r.works[0].id)[0].access_status, "unavailable");
        assert_eq!(
            db.workflow_link_add(add).unwrap().link.access_status,
            "unavailable"
        );
        let mut invalid = WorkflowLinkAddInput {
            work_id: r.works[0].id.clone(),
            target_kind: WorkflowTargetKind::Memory,
            target_id: "missing".into(),
            target_source: None,
            relation_type: "verification".into(),
            reason: "explicit".into(),
            idempotency_key: "invalid".into(),
            expected_version: None,
        };
        assert!(db.workflow_link_add(invalid.clone()).is_err());
        invalid.target_id = "m".into();
        assert!(db.workflow_link_add(invalid).is_err());
    }
    #[test]
    fn import_failure_rolls_back_mapping_work_link_and_idempotency() {
        let db = Database::in_memory().unwrap();
        seed(&db);
        db.lock().unwrap().execute_batch("CREATE TRIGGER fail_link BEFORE INSERT ON work_links BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(db.workflow_import_apply(request("atomic")).is_err());
        for table in [
            "work_items",
            "legacy_work_mappings",
            "work_links",
            "idempotency_records",
            "activity_events",
        ] {
            let n: i64 = db
                .lock()
                .unwrap()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{table}");
        }
        db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_link")
            .unwrap();
        assert!(
            !db.workflow_import_apply(request("atomic"))
                .unwrap()
                .idempotent_replay
        );
    }
    #[test]
    fn migration_nine_rolls_back_and_retries() {
        let mut c = Connection::open_in_memory().unwrap();
        assert!(migrate(&mut c, Some(9)).is_err());
        assert_eq!(
            c.query_row("SELECT max(version) FROM schema_migrations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            8
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='work_links'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        migrate(&mut c, None).unwrap();
    }
    #[test]
    fn competing_removals_commit_one_version() {
        let db = std::sync::Arc::new(Database::in_memory().unwrap());
        seed(&db);
        let imported = db.workflow_import_apply(request("first")).unwrap();
        let id = list(&db, &imported.works[0].id)[0].id.clone();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|n| {
                let db = db.clone();
                let id = id.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    db.workflow_link_remove(WorkflowLinkRemoveInput {
                        id,
                        expected_version: 1,
                        idempotency_key: format!("remove-{n}"),
                    })
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
    }
    #[test]
    fn backup_restores_mapping_links_and_replay() {
        let dir = std::env::temp_dir().join(format!("workflow-links-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("live.sqlite")).unwrap();
        seed(&db);
        let imported = db.workflow_import_apply(request("first")).unwrap();
        let link = list(&db, &imported.works[0].id)[0].clone();
        let backup = dir.join("backup.sqlite");
        db.backup_to(&backup).unwrap();
        db.workflow_link_remove(WorkflowLinkRemoveInput {
            id: link.id,
            expected_version: 1,
            idempotency_key: "remove".into(),
        })
        .unwrap();
        db.restore_from(&backup).unwrap();
        assert!(list(&db, &imported.works[0].id)[0].removed_at.is_none());
        assert!(
            db.workflow_import_apply(request("first"))
                .unwrap()
                .idempotent_replay
        );
        assert_eq!(
            db.workflow_import_apply(request("mapped")).unwrap().works[0].id,
            imported.works[0].id
        );
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn source_reference_resolves_existing_identity_only() {
        let db = Database::in_memory().unwrap();
        seed(&db);
        let imported = db.workflow_import_apply(request("first")).unwrap();
        let mut input = WorkflowLinkAddInput {
            work_id: imported.works[0].id.clone(),
            target_kind: WorkflowTargetKind::Source,
            target_id: String::new(),
            target_source: Some(SourceRef::new(
                "obsidian",
                "vault",
                "note",
                "obsidian://note",
                "note",
            )),
            relation_type: "specification".into(),
            reason: "explicit source".into(),
            idempotency_key: "source".into(),
            expected_version: None,
        };
        assert_eq!(
            db.workflow_link_add(input.clone()).unwrap().link.target_id,
            "s"
        );
        input.idempotency_key = "unknown".into();
        input.target_source.as_mut().unwrap().external_id = "unindexed".into();
        assert!(db.workflow_link_add(input).is_err());
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row("SELECT count(*) FROM sources", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn presentation_is_live_redacted_and_never_persisted() {
        let db = Database::in_memory().unwrap();
        seed(&db);
        let imported = db.workflow_import_apply(request("first")).unwrap();
        let input = WorkflowLinkAddInput {
            work_id: imported.works[0].id.clone(),
            target_kind: WorkflowTargetKind::Source,
            target_id: "s".into(),
            target_source: None,
            relation_type: "context".into(),
            reason: "explicit".into(),
            idempotency_key: "presentation".into(),
            expected_version: None,
        };
        let first = db.workflow_link_add(input.clone()).unwrap();
        assert_eq!(first.link.target_title.as_deref(), Some("private title"));
        assert_eq!(
            first.link.target_source.as_ref().unwrap().external_id,
            "note"
        );
        assert_eq!(first.link.fetched_at.as_deref(), Some("now"));
        for sql in [
            "SELECT data_json FROM work_links WHERE target_kind='source'",
            "SELECT response_json FROM idempotency_records WHERE idempotency_key='presentation'",
        ] {
            let stored: String = db.lock().unwrap().query_row(sql, [], |r| r.get(0)).unwrap();
            assert!(!stored.contains("private title"));
            assert!(!stored.contains("obsidian://note"));
        }
        db.lock()
            .unwrap()
            .execute(
                "UPDATE snapshots SET title='current title' WHERE source_id='s'",
                [],
            )
            .unwrap();
        assert_eq!(
            db.workflow_link_add(input.clone())
                .unwrap()
                .link
                .target_title
                .as_deref(),
            Some("current title")
        );
        db.lock()
            .unwrap()
            .execute(
                "UPDATE snapshots SET access_status='permission_denied' WHERE source_id='s'",
                [],
            )
            .unwrap();
        let replay = db.workflow_link_add(input).unwrap();
        assert!(replay.idempotent_replay);
        assert!(replay.link.target_title.is_none());
        assert!(replay.link.target_source.is_none());
        assert!(replay.link.fetched_at.is_none());
        assert!(list(&db, &imported.works[0].id)
            .iter()
            .all(|l| l.target_title.is_none()
                && l.target_source.is_none()
                && l.fetched_at.is_none()));
    }
}
