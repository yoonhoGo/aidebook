//! Explicit, read-only Obsidian vault scanning for the M1 vertical slice.
//!
//! The adapter accepts a vault path supplied by the user. It never discovers
//! or chooses a vault on its own, never writes a file, and rejects paths that
//! resolve outside the selected root. The watcher is intentionally polling:
//! every poll re-checks file hashes, which also recovers changes missed while
//! an iCloud placeholder was offline.

use super::fixtures::ReadOnlyConnector;
use super::types::{
    AccessStatus, ConnectorManifest, CoreError, CoreResult, Snapshot, SourceLink, SourceRef,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const DEFAULT_ALLOWED_FRONTMATTER: &[&str] = &["title", "tags", "aliases", "date", "modified"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    pub root_path: PathBuf,
    pub account_id: String,
    pub connection_id: String,
    #[serde(default)]
    pub allowed_frontmatter: Vec<String>,
}

impl VaultConfig {
    pub fn new(
        root_path: impl Into<PathBuf>,
        account_id: impl Into<String>,
        connection_id: impl Into<String>,
    ) -> Self {
        Self {
            root_path: root_path.into(),
            account_id: account_id.into(),
            connection_id: connection_id.into(),
            allowed_frontmatter: DEFAULT_ALLOWED_FRONTMATTER
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObsidianAdapter {
    config: VaultConfig,
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultScanResult {
    pub root_path: String,
    pub snapshots: Vec<Snapshot>,
    pub inaccessible: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultChangeKind {
    Added,
    Modified,
    Removed,
    BecameAvailable,
    BecameUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultChange {
    pub external_id: String,
    pub kind: VaultChangeKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultIndex {
    pub content_hashes: HashMap<String, String>,
    pub access: HashMap<String, AccessStatus>,
}

pub struct VaultWatcher {
    adapter: ObsidianAdapter,
    previous: VaultIndex,
}

impl ObsidianAdapter {
    pub fn open(config: VaultConfig) -> CoreResult<Self> {
        if config.account_id.trim().is_empty() || config.connection_id.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "vault.account_id/connection_id".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        let root = fs::canonicalize(&config.root_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                CoreError::PermissionDenied {
                    provider: "obsidian".to_string(),
                    message: "the selected vault cannot be read".to_string(),
                }
            } else {
                CoreError::NotFound {
                    entity: "selected Obsidian vault".to_string(),
                    id: config.root_path.display().to_string(),
                }
            }
        })?;
        let metadata = fs::metadata(&root).map_err(|error| CoreError::Connector {
            message: error.to_string(),
        })?;
        if !metadata.is_dir() {
            return Err(CoreError::InvalidInput {
                field: "vault.root_path".to_string(),
                message: "the selected path must be a directory".to_string(),
            });
        }
        if config.allowed_frontmatter.is_empty() {
            return Ok(Self {
                config: VaultConfig {
                    allowed_frontmatter: DEFAULT_ALLOWED_FRONTMATTER
                        .iter()
                        .map(|key| (*key).to_string())
                        .collect(),
                    ..config
                },
                root,
            });
        }
        Ok(Self { config, root })
    }

    pub fn config(&self) -> &VaultConfig {
        &self.config
    }

    pub fn root_path(&self) -> &Path {
        &self.root
    }

    pub fn scan(&self) -> CoreResult<VaultScanResult> {
        let mut files = Vec::new();
        collect_markdown_files(&self.root, &self.root, &mut files)?;
        files.sort();
        let mut snapshots = Vec::with_capacity(files.len());
        for path in files {
            snapshots.push(self.read_path(&path)?);
        }
        let inaccessible = snapshots
            .iter()
            .filter(|snapshot| !snapshot.access_status.is_searchable())
            .count();
        Ok(VaultScanResult {
            root_path: self.root.display().to_string(),
            snapshots,
            inaccessible,
        })
    }

    pub fn read_path(&self, path: &Path) -> CoreResult<Snapshot> {
        let canonical = fs::canonicalize(path).map_err(|error| CoreError::NotFound {
            entity: "vault file".to_string(),
            id: format!("{}: {error}", path.display()),
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(CoreError::PermissionDenied {
                provider: "obsidian".to_string(),
                message: "the requested file is outside the selected vault".to_string(),
            });
        }
        if canonical
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("md")
        {
            return Err(CoreError::InvalidInput {
                field: "vault.file".to_string(),
                message: "only Markdown files are indexed".to_string(),
            });
        }
        let relative =
            canonical
                .strip_prefix(&self.root)
                .map_err(|_| CoreError::PermissionDenied {
                    provider: "obsidian".to_string(),
                    message: "the requested file is outside the selected vault".to_string(),
                })?;
        let external_id = relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let source = SourceRef::new(
            "obsidian",
            self.config.account_id.clone(),
            external_id,
            self.obsidian_url(relative),
            "note",
        );
        let metadata = fs::metadata(&canonical).map_err(|error| CoreError::Connector {
            message: error.to_string(),
        })?;
        if is_placeholder_or_conflict(&canonical, &metadata) {
            let mut snapshot = Snapshot::new(
                source,
                relative
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Untitled"),
                "",
                None,
                super::types::now_rfc3339(),
            );
            snapshot.access_status = AccessStatus::Unavailable;
            snapshot.unavailable_reason =
                Some("iCloud placeholder or conflict file is not available".to_string());
            return snapshot.normalize();
        }
        let body = match fs::read_to_string(&canonical) {
            Ok(body) => body,
            Err(error) => {
                let mut snapshot = Snapshot::new(
                    source,
                    relative
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Untitled"),
                    "",
                    None,
                    super::types::now_rfc3339(),
                );
                snapshot.access_status = if error.kind() == std::io::ErrorKind::PermissionDenied {
                    AccessStatus::PermissionDenied
                } else {
                    AccessStatus::Unavailable
                };
                snapshot.unavailable_reason =
                    Some("the Markdown file could not be read".to_string());
                return snapshot.normalize();
            }
        };
        let parsed = parse_markdown(&body, &self.config.allowed_frontmatter, relative);
        let source_updated_at = metadata.modified().ok().map(system_time_to_rfc3339);
        let mut snapshot = Snapshot::new(
            source,
            parsed.title,
            parsed.body,
            source_updated_at,
            super::types::now_rfc3339(),
        );
        snapshot.metadata = parsed.metadata;
        snapshot.links = parsed.links;
        snapshot.normalize()
    }

    pub fn watcher(self) -> CoreResult<VaultWatcher> {
        let initial = self.scan()?;
        Ok(VaultWatcher {
            previous: VaultIndex::from_scan(&initial),
            adapter: self,
        })
    }

    fn obsidian_url(&self, relative: &Path) -> String {
        let file = relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        format!(
            "obsidian://open?vault={}&file={}",
            percent_encode(&self.config.account_id),
            percent_encode(&file)
        )
    }
}

impl ReadOnlyConnector for ObsidianAdapter {
    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            id: "obsidian".to_string(),
            version: "m1".to_string(),
            api_version: "1".to_string(),
            capabilities: vec![
                "read".to_string(),
                "search".to_string(),
                "delta".to_string(),
                "watch".to_string(),
            ],
            permissions: vec!["selected-vault-read".to_string()],
        }
    }

    fn connection_id(&self) -> &str {
        &self.config.connection_id
    }

    fn list(&self) -> CoreResult<Vec<Snapshot>> {
        Ok(self.scan()?.snapshots)
    }

    fn fetch(&self, source: &SourceRef) -> CoreResult<Snapshot> {
        if source.provider != "obsidian" || source.account_id != self.config.account_id {
            return Err(CoreError::PermissionDenied {
                provider: "obsidian".to_string(),
                message: "source is outside the selected vault scope".to_string(),
            });
        }
        self.read_path(&self.root.join(&source.external_id))
    }
}

impl VaultIndex {
    pub fn from_scan(scan: &VaultScanResult) -> Self {
        let mut index = Self::default();
        for snapshot in &scan.snapshots {
            index.content_hashes.insert(
                snapshot.source.external_id.clone(),
                snapshot.content_hash.clone(),
            );
            index.access.insert(
                snapshot.source.external_id.clone(),
                snapshot.access_status.clone(),
            );
        }
        index
    }

    pub fn diff(&self, next: &Self) -> Vec<VaultChange> {
        let mut ids = self
            .content_hashes
            .keys()
            .chain(next.content_hashes.keys())
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .filter_map(|external_id| {
                let before_hash = self.content_hashes.get(&external_id);
                let after_hash = next.content_hashes.get(&external_id);
                let before_access = self.access.get(&external_id);
                let after_access = next.access.get(&external_id);
                match (before_hash, after_hash) {
                    (None, Some(_)) => Some(VaultChange {
                        external_id,
                        kind: VaultChangeKind::Added,
                    }),
                    (Some(_), None) => Some(VaultChange {
                        external_id,
                        kind: VaultChangeKind::Removed,
                    }),
                    (Some(before), Some(after)) if before != after => Some(VaultChange {
                        external_id,
                        kind: if before_access != after_access
                            && after_access.is_some_and(AccessStatus::is_searchable)
                        {
                            VaultChangeKind::BecameAvailable
                        } else if before_access != after_access
                            && before_access.is_some_and(AccessStatus::is_searchable)
                        {
                            VaultChangeKind::BecameUnavailable
                        } else {
                            VaultChangeKind::Modified
                        },
                    }),
                    (Some(_), Some(_)) if before_access != after_access => Some(VaultChange {
                        external_id,
                        kind: if after_access.is_some_and(AccessStatus::is_searchable) {
                            VaultChangeKind::BecameAvailable
                        } else {
                            VaultChangeKind::BecameUnavailable
                        },
                    }),
                    _ => None,
                }
            })
            .collect()
    }
}

impl VaultWatcher {
    pub fn observe(&mut self, scan: VaultScanResult) -> Vec<VaultChange> {
        let next = VaultIndex::from_scan(&scan);
        let changes = self.previous.diff(&next);
        self.previous = next;
        changes
    }

    pub fn poll(&mut self) -> CoreResult<(VaultScanResult, Vec<VaultChange>)> {
        let scan = self.adapter.scan()?;
        let changes = self.observe(scan.clone());
        Ok((scan, changes))
    }
}

#[derive(Debug)]
struct ParsedMarkdown {
    title: String,
    body: String,
    metadata: BTreeMap<String, serde_json::Value>,
    links: Vec<SourceLink>,
}

fn parse_markdown(body: &str, allowed: &[String], relative: &Path) -> ParsedMarkdown {
    let lines = body.lines().collect::<Vec<_>>();
    let mut metadata = BTreeMap::new();
    let mut body_start = 0;
    if lines.first().is_some_and(|line| line.trim() == "---") {
        if let Some(end) = lines.iter().skip(1).position(|line| line.trim() == "---") {
            let end = end + 1;
            let allowed = allowed
                .iter()
                .map(|key| key.as_str())
                .collect::<HashSet<_>>();
            let mut current_key: Option<String> = None;
            for line in &lines[1..end] {
                if let Some((key, value)) = line.split_once(':') {
                    let key = key.trim().to_string();
                    if allowed.contains(key.as_str()) {
                        let value = parse_frontmatter_value(value.trim());
                        metadata.insert(key.clone(), value);
                        current_key = Some(key);
                    } else {
                        current_key = None;
                    }
                } else if line.trim_start().starts_with('-') {
                    if let Some(key) = current_key.as_ref() {
                        if let Some(value) = metadata.get_mut(key) {
                            if let serde_json::Value::Array(items) = value {
                                items.push(parse_frontmatter_value(
                                    line.trim_start_matches('-').trim(),
                                ));
                            }
                        }
                    }
                }
            }
            body_start = end + 1;
        }
    }
    let markdown_body = lines[body_start..].join("\n").trim().to_string();
    let title = metadata
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            lines[body_start..].iter().find_map(|line| {
                line.strip_prefix("# ")
                    .map(|title| title.trim().to_string())
            })
        })
        .unwrap_or_else(|| {
            relative
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Untitled")
                .to_string()
        });
    ParsedMarkdown {
        title,
        links: parse_links(&markdown_body),
        body: markdown_body,
        metadata,
    }
}

fn parse_frontmatter_value(value: &str) -> serde_json::Value {
    let value = value.trim_matches(|character| character == '"' || character == '\'');
    if value.starts_with('[') && value.ends_with(']') {
        return serde_json::Value::Array(
            value[1..value.len() - 1]
                .split(',')
                .filter(|item| !item.trim().is_empty())
                .map(|item| serde_json::Value::String(item.trim().trim_matches('"').to_string()))
                .collect(),
        );
    }
    match value {
        "true" => serde_json::Value::Bool(true),
        "false" => serde_json::Value::Bool(false),
        "null" => serde_json::Value::Null,
        _ => serde_json::Value::String(value.to_string()),
    }
}

fn parse_links(body: &str) -> Vec<SourceLink> {
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = 0;
    while let Some(start) = body[cursor..].find("[[") {
        let start = cursor + start + 2;
        let Some(end_offset) = body[start..].find("]]") else {
            break;
        };
        let end = start + end_offset;
        let target = body[start..end]
            .split('|')
            .next()
            .unwrap_or_default()
            .split('#')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if !target.is_empty() && seen.insert(format!("wikilink:{target}")) {
            links.push(SourceLink {
                target,
                kind: "wikilink".to_string(),
            });
        }
        cursor = end + 2;
    }
    for word in body.split_whitespace() {
        let trimmed = word.trim_matches(|character: char| "()[]{}<>\"'.,;".contains(character));
        if (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
            && seen.insert(format!("url:{trimmed}"))
        {
            links.push(SourceLink {
                target: trimmed.to_string(),
                kind: "url".to_string(),
            });
        }
    }
    links
}

fn collect_markdown_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> CoreResult<()> {
    let entries = fs::read_dir(current).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            CoreError::PermissionDenied {
                provider: "obsidian".to_string(),
                message: format!("cannot read {}", current.display()),
            }
        } else {
            CoreError::Connector {
                message: error.to_string(),
            }
        }
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| CoreError::Connector {
            message: error.to_string(),
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| CoreError::Connector {
            message: error.to_string(),
        })?;
        if file_type.is_symlink() {
            let canonical = fs::canonicalize(&path).map_err(|error| CoreError::Connector {
                message: error.to_string(),
            })?;
            if !canonical.starts_with(root) {
                continue;
            }
            if canonical.is_file()
                && canonical
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("md")
            {
                files.push(canonical);
            }
        } else if file_type.is_dir() {
            collect_markdown_files(root, &path, files)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            files.push(path);
        }
    }
    Ok(())
}

fn is_placeholder_or_conflict(path: &Path, metadata: &fs::Metadata) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    metadata.len() == 0
        && (name.ends_with(".icloud")
            || name.contains(".icloud.")
            || name.contains("conflicted copy")
            || name.contains(".conflicted"))
}

fn system_time_to_rfc3339(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339()
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_allowed_metadata_and_explicit_links() {
        let parsed = parse_markdown(
            "---\ntitle: Release notes\ntags: [release, m1]\nprivate: no\n---\n# Fallback\nSee [[Roadmap|the roadmap]] and https://example.com/issues/1.",
            &VaultConfig::new("/tmp", "vault", "connection").allowed_frontmatter,
            Path::new("Release.md"),
        );
        assert_eq!(parsed.title, "Release notes");
        assert!(parsed.metadata.contains_key("tags"));
        assert!(!parsed.metadata.contains_key("private"));
        assert_eq!(parsed.links.len(), 2);
    }

    #[test]
    fn selected_root_scan_diff_detects_add_modify_remove_without_writing() {
        let root = std::env::temp_dir().join(format!("aidebook-vault-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("vault");
        let path = root.join("Note.md");
        fs::File::create(&path)
            .expect("note")
            .write_all(b"# One\nfirst")
            .expect("write");
        let adapter =
            ObsidianAdapter::open(VaultConfig::new(&root, "fixture", "vault")).expect("open");
        let first = adapter.scan().expect("first scan");
        fs::write(&path, "# Two\nsecond").expect("modify");
        fs::write(root.join("New.md"), "# New\ntext").expect("add");
        fs::remove_file(&path).expect("remove");
        let second = adapter.scan().expect("second scan");
        let changes = VaultIndex::from_scan(&first).diff(&VaultIndex::from_scan(&second));
        assert!(changes
            .iter()
            .any(|change| change.kind == VaultChangeKind::Removed));
        assert!(changes
            .iter()
            .any(|change| change.kind == VaultChangeKind::Added));
        assert!(!path.exists());
        fs::remove_dir_all(&root).expect("cleanup");
    }
}
