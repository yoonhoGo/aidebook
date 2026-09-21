//! Read-only GitHub repository connector.
//!
//! The adapter keeps provider selection, pagination, credential lookup, and
//! provider error mapping at the connector boundary.  It never places a token
//! in a snapshot, database row, log message, or returned response.  Tests use
//! `FixtureGitHubApi`; the optional curl transport is only used after the user
//! has explicitly selected a repository and stored a token in Keychain.

use super::fixtures::{ChangeBatch, ConnectorConnection, ReadOnlyConnector};
use super::types::{
    now_rfc3339, ConnectorManifest, CoreError, CoreResult, Snapshot, SourceLink, SourceRef,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const DEFAULT_API_BASE_URL: &str = "https://api.github.com";
const KEYCHAIN_SERVICE_PREFIX: &str = "com.yoonho.go.aidebook.credentials.";

/// Credential storage is intentionally narrower than the connector API.  The
/// production implementation is Keychain-backed; tests use an in-memory
/// implementation and never touch a user's account.
pub trait CredentialStore: Clone + Send + Sync + 'static {
    fn get(&self, connection_id: &str) -> CoreResult<Option<String>>;
    fn set(&self, connection_id: &str, token: &str) -> CoreResult<()>;
    fn delete(&self, connection_id: &str) -> CoreResult<()>;
}

#[derive(Clone, Default)]
pub struct MemoryCredentialStore {
    values: Arc<Mutex<HashMap<String, String>>>,
}

impl MemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for MemoryCredentialStore {
    fn get(&self, connection_id: &str) -> CoreResult<Option<String>> {
        let values = self.values.lock().map_err(|_| CoreError::Database {
            message: "credential store mutex was poisoned".to_string(),
        })?;
        Ok(values.get(connection_id).cloned())
    }

    fn set(&self, connection_id: &str, token: &str) -> CoreResult<()> {
        if connection_id.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "connection_id".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        if token.trim().is_empty() {
            return Err(CoreError::InvalidInput {
                field: "token".to_string(),
                message: "must not be empty".to_string(),
            });
        }
        let mut values = self.values.lock().map_err(|_| CoreError::Database {
            message: "credential store mutex was poisoned".to_string(),
        })?;
        values.insert(connection_id.to_string(), token.to_string());
        Ok(())
    }

    fn delete(&self, connection_id: &str) -> CoreResult<()> {
        let mut values = self.values.lock().map_err(|_| CoreError::Database {
            message: "credential store mutex was poisoned".to_string(),
        })?;
        values.remove(connection_id);
        Ok(())
    }
}

/// Native macOS Keychain access: secrets never enter process arguments or logs.
#[derive(Debug, Clone, Default)]
pub struct KeychainCredentialStore;
impl KeychainCredentialStore {
    fn service(connection_id: &str) -> String {
        format!("{KEYCHAIN_SERVICE_PREFIX}{connection_id}")
    }
    fn unavailable(message: impl Into<String>) -> CoreError {
        CoreError::Provider {
            provider: "keychain".into(),
            code: "unavailable".into(),
            message: message.into(),
            retry_at: None,
        }
    }
}
impl CredentialStore for KeychainCredentialStore {
    fn get(&self, connection_id: &str) -> CoreResult<Option<String>> {
        validate_connection_id(connection_id)?;
        #[cfg(target_os = "macos")]
        {
            match security_framework::passwords::get_generic_password(
                &Self::service(connection_id),
                "aidebook",
            ) {
                Ok(bytes) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| Self::unavailable("Keychain credential is not text")),
                Err(error) if error.code() == -25300 => Ok(None),
                Err(_) => Err(Self::unavailable("Keychain lookup was rejected")),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(Self::unavailable(
                "macOS Keychain is only available on macOS",
            ))
        }
    }
    fn set(&self, connection_id: &str, token: &str) -> CoreResult<()> {
        validate_connection_id(connection_id)?;
        if token.trim().is_empty() || token.chars().any(char::is_control) {
            return Err(CoreError::InvalidInput {
                field: "token".into(),
                message: "must be nonempty and contain no control characters".into(),
            });
        }
        #[cfg(target_os = "macos")]
        {
            security_framework::passwords::set_generic_password(
                &Self::service(connection_id),
                "aidebook",
                token.as_bytes(),
            )
            .map_err(|_| Self::unavailable("Keychain write was rejected"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(Self::unavailable(
                "macOS Keychain is only available on macOS",
            ))
        }
    }
    fn delete(&self, connection_id: &str) -> CoreResult<()> {
        validate_connection_id(connection_id)?;
        #[cfg(target_os = "macos")]
        {
            match security_framework::passwords::delete_generic_password(
                &Self::service(connection_id),
                "aidebook",
            ) {
                Ok(()) => Ok(()),
                Err(error) if error.code() == -25300 => Ok(()),
                Err(_) => Err(Self::unavailable("Keychain deletion was rejected")),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(Self::unavailable(
                "macOS Keychain is only available on macOS",
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubConfig {
    pub account_id: String,
    pub connection_id: String,
    pub owner: String,
    pub repository: String,
    #[serde(default = "default_api_base_url")]
    pub api_base_url: String,
    #[serde(default = "default_page_size")]
    pub per_page: u32,
}

impl GitHubConfig {
    pub fn new(
        account_id: impl Into<String>,
        connection_id: impl Into<String>,
        owner: impl Into<String>,
        repository: impl Into<String>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            connection_id: connection_id.into(),
            owner: owner.into(),
            repository: repository.into(),
            api_base_url: DEFAULT_API_BASE_URL.to_string(),
            per_page: default_page_size(),
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        for (field, value) in [
            ("account_id", self.account_id.as_str()),
            ("connection_id", self.connection_id.as_str()),
            ("owner", self.owner.as_str()),
            ("repository", self.repository.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(CoreError::InvalidInput {
                    field: field.to_string(),
                    message: "must not be empty".to_string(),
                });
            }
        }
        validate_connection_id(&self.connection_id)?;
        validate_segment("owner", &self.owner)?;
        validate_segment("repository", &self.repository)?;
        if !(1..=100).contains(&self.per_page) {
            return Err(CoreError::InvalidInput {
                field: "per_page".to_string(),
                message: "must be between 1 and 100".to_string(),
            });
        }
        if !self.api_base_url.starts_with("https://")
            || self.api_base_url.contains('?')
            || self.api_base_url.contains('#')
        {
            return Err(CoreError::InvalidInput {
                field: "api_base_url".to_string(),
                message: "must be an https URL without query or fragment".to_string(),
            });
        }
        Ok(())
    }

    pub fn scope(&self) -> String {
        format!("{}/{}", self.owner, self.repository)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubComment {
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    /// `issue` or `pull_request`; the connector never infers this from title.
    pub kind: String,
    pub state: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub html_url: String,
    #[serde(default)]
    pub comments: Vec<GitHubComment>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubPage {
    pub items: Vec<GitHubIssue>,
    pub next_page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", content = "details", rename_all = "snake_case")]
pub enum GitHubApiError {
    #[error("GitHub authentication was rejected")]
    Unauthorized { message: String },
    #[error("GitHub repository permission was denied")]
    Forbidden { message: String },
    #[error("GitHub resource was not found")]
    NotFound { message: String },
    #[error("GitHub rate limit exceeded")]
    RateLimited {
        message: String,
        retry_at: Option<String>,
    },
    #[error("GitHub server error ({status})")]
    Server { status: u16, message: String },
    #[error("GitHub network error")]
    Network { message: String },
    #[error("GitHub response was malformed")]
    Malformed { message: String },
}

pub trait GitHubApi: Clone + Send + Sync + 'static {
    fn fetch_page(
        &self,
        config: &GitHubConfig,
        page: u32,
        token: &str,
    ) -> Result<GitHubPage, GitHubApiError>;
}

/// Deterministic page/error source for tests and offline development.
#[derive(Debug, Clone, Default)]
pub struct FixtureGitHubApi {
    pages: Arc<Mutex<HashMap<u32, Result<GitHubPage, GitHubApiError>>>>,
}

impl FixtureGitHubApi {
    pub fn new(pages: impl IntoIterator<Item = GitHubPage>) -> Self {
        let pages = pages
            .into_iter()
            .enumerate()
            .map(|(index, page)| ((index + 1) as u32, Ok(page)))
            .collect();
        Self {
            pages: Arc::new(Mutex::new(pages)),
        }
    }

    pub fn with_error(self, page: u32, error: GitHubApiError) -> Self {
        if let Ok(mut pages) = self.pages.lock() {
            pages.insert(page, Err(error));
        }
        self
    }
}

impl GitHubApi for FixtureGitHubApi {
    fn fetch_page(
        &self,
        _config: &GitHubConfig,
        page: u32,
        _token: &str,
    ) -> Result<GitHubPage, GitHubApiError> {
        self.pages
            .lock()
            .map_err(|_| GitHubApiError::Network {
                message: "fixture mutex was poisoned".to_string(),
            })?
            .get(&page)
            .cloned()
            .unwrap_or_else(|| {
                Ok(GitHubPage {
                    items: Vec::new(),
                    next_page: None,
                })
            })
    }
}

/// Optional live transport.  It invokes the system curl executable only on a
/// manually selected refresh; the token is supplied through curl's stdin
/// config rather than a command-line argument.
#[derive(Debug, Clone, Default)]
pub struct HttpGitHubApi;

impl GitHubApi for HttpGitHubApi {
    fn fetch_page(
        &self,
        config: &GitHubConfig,
        page: u32,
        token: &str,
    ) -> Result<GitHubPage, GitHubApiError> {
        let base = config.api_base_url.trim_end_matches('/');
        let url = format!(
            "{base}/repos/{}/{}/issues?state=all&per_page={}&page={}",
            config.owner, config.repository, config.per_page, page
        );
        let payload = curl_json(&url, token)?;
        let array = payload
            .as_array()
            .ok_or_else(|| GitHubApiError::Malformed {
                message: "issues response was not an array".to_string(),
            })?;
        let mut items = Vec::with_capacity(array.len());
        for item in array {
            let number = item.get("number").and_then(Value::as_u64).ok_or_else(|| {
                GitHubApiError::Malformed {
                    message: "issue number is missing".to_string(),
                }
            })?;
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| GitHubApiError::Malformed {
                    message: "issue title is missing".to_string(),
                })?
                .to_string();
            let html_url = item
                .get("html_url")
                .and_then(Value::as_str)
                .ok_or_else(|| GitHubApiError::Malformed {
                    message: "issue html_url is missing".to_string(),
                })?
                .to_string();
            let kind = if item.get("pull_request").is_some() {
                "pull_request"
            } else {
                "issue"
            };
            let comments = if item.get("comments_url").and_then(Value::as_str).is_some() {
                if item.get("comments").and_then(Value::as_u64).unwrap_or(0) > 0 {
                    parse_comments(&curl_json(
                        &format!(
                            "{base}/repos/{}/{}/issues/{number}/comments",
                            config.owner, config.repository
                        ),
                        token,
                    )?)?
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            let labels = item
                .get("labels")
                .and_then(Value::as_array)
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(|label| label.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            items.push(GitHubIssue {
                number,
                title,
                body: item.get("body").and_then(Value::as_str).map(str::to_string),
                kind: kind.to_string(),
                state: item
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                author: item
                    .get("user")
                    .and_then(|user| user.get("login"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                updated_at: item
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                html_url,
                comments,
                labels,
            });
        }
        Ok(GitHubPage {
            next_page: if items.len() as u32 >= config.per_page {
                Some(page + 1)
            } else {
                None
            },
            items,
        })
    }
}

#[derive(Clone)]
pub struct GitHubAdapter<A, C>
where
    A: GitHubApi,
    C: CredentialStore,
{
    config: GitHubConfig,
    api: A,
    credentials: C,
}

impl<A, C> GitHubAdapter<A, C>
where
    A: GitHubApi,
    C: CredentialStore,
{
    pub fn new(config: GitHubConfig, api: A, credentials: C) -> CoreResult<Self> {
        config.validate()?;
        Ok(Self {
            config,
            api,
            credentials,
        })
    }

    pub fn config(&self) -> &GitHubConfig {
        &self.config
    }

    pub fn credentials(&self) -> &C {
        &self.credentials
    }

    pub fn list_pages(&self) -> CoreResult<Vec<GitHubIssue>> {
        let token = self.token()?;
        let mut page = 1;
        let mut visited = HashSet::new();
        let mut issues = Vec::new();
        loop {
            if !visited.insert(page) || visited.len() > 1_000 {
                return Err(CoreError::Provider {
                    provider: "github".to_string(),
                    code: "pagination_loop".to_string(),
                    message: "the provider returned a repeated or excessive page cursor"
                        .to_string(),
                    retry_at: None,
                });
            }
            let result = self
                .api
                .fetch_page(&self.config, page, &token)
                .map_err(map_github_error)?;
            let next = result.next_page;
            issues.extend(result.items);
            match next {
                Some(next_page) => page = next_page,
                None => break,
            }
        }
        Ok(issues)
    }

    fn token(&self) -> CoreResult<String> {
        self.credentials
            .get(&self.config.connection_id)?
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| CoreError::Provider {
                provider: "github".to_string(),
                code: "credential_missing".to_string(),
                message: "a token is required for the selected repository".to_string(),
                retry_at: None,
            })
    }
}

impl<A, C> ReadOnlyConnector for GitHubAdapter<A, C>
where
    A: GitHubApi,
    C: CredentialStore,
{
    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            id: "github".to_string(),
            version: "m2".to_string(),
            api_version: "github-rest-v3".to_string(),
            capabilities: vec![
                "read".to_string(),
                "search".to_string(),
                "delta".to_string(),
                "manual_refresh".to_string(),
                "comments".to_string(),
                "issues".to_string(),
                "pull_requests".to_string(),
            ],
            permissions: vec!["repository:read".to_string()],
        }
    }

    fn connection_id(&self) -> &str {
        &self.config.connection_id
    }

    fn list(&self) -> CoreResult<Vec<Snapshot>> {
        self.list_pages()?
            .into_iter()
            .map(|issue| self.snapshot(issue))
            .collect()
    }

    fn fetch(&self, source: &SourceRef) -> CoreResult<Snapshot> {
        if source.provider != "github" || source.account_id != self.config.account_id {
            return Err(CoreError::PermissionDenied {
                provider: "github".to_string(),
                message: "source is outside the selected account scope".to_string(),
            });
        }
        self.list()?
            .into_iter()
            .find(|snapshot| snapshot.source == *source)
            .ok_or_else(|| CoreError::NotFound {
                entity: "github source".to_string(),
                id: source.id(),
            })
    }

    fn connect(&self, scope: &str) -> CoreResult<ConnectorConnection> {
        if scope != self.config.scope() {
            return Err(CoreError::PermissionDenied {
                provider: "github".to_string(),
                message: "requested repository is outside the selected scope".to_string(),
            });
        }
        let _ = self.token()?;
        Ok(ConnectorConnection {
            connection_id: self.config.connection_id.clone(),
            provider: "github".to_string(),
            scope: self.config.scope(),
        })
    }

    fn sync(&self, _cursor: Option<&str>) -> CoreResult<ChangeBatch> {
        Ok(ChangeBatch {
            snapshots: self.list()?,
            tombstones: Vec::new(),
            next_cursor: None,
        })
    }

    fn disconnect(&self) -> CoreResult<()> {
        // Disconnecting a provider does not silently delete a Keychain item;
        // explicit credential deletion belongs to the settings UI/M5.
        Ok(())
    }
}

impl<A, C> GitHubAdapter<A, C>
where
    A: GitHubApi,
    C: CredentialStore,
{
    fn snapshot(&self, issue: GitHubIssue) -> CoreResult<Snapshot> {
        if issue.number == 0 {
            return Err(CoreError::InvalidInput {
                field: "github.issue.number".to_string(),
                message: "must be positive".to_string(),
            });
        }
        let kind = match issue.kind.as_str() {
            "issue" | "pull_request" => issue.kind.clone(),
            other => {
                return Err(CoreError::InvalidInput {
                    field: "github.issue.kind".to_string(),
                    message: format!("unsupported item kind '{other}'"),
                })
            }
        };
        let external_id = format!("{}#{}", self.config.scope(), issue.number);
        let mut body = issue.body.unwrap_or_default();
        for comment in &issue.comments {
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str("Comment by ");
            body.push_str(&comment.author);
            body.push_str(":\n");
            body.push_str(&comment.body);
        }
        let mut snapshot = Snapshot::new(
            SourceRef::new(
                "github",
                self.config.account_id.clone(),
                external_id,
                issue.html_url,
                kind,
            ),
            issue.title,
            body,
            issue.updated_at,
            now_rfc3339(),
        );
        snapshot
            .metadata
            .insert("repository".to_string(), Value::String(self.config.scope()));
        snapshot
            .metadata
            .insert("state".to_string(), Value::String(issue.state));
        if let Some(author) = issue.author {
            snapshot
                .metadata
                .insert("author".to_string(), Value::String(author));
        }
        snapshot.metadata.insert(
            "labels".to_string(),
            Value::Array(issue.labels.into_iter().map(Value::String).collect()),
        );
        snapshot
            .metadata
            .insert("comments_count".to_string(), json!(issue.comments.len()));
        snapshot.links.push(SourceLink {
            target: snapshot.source.url.clone(),
            kind: "canonical".to_string(),
        });
        snapshot.normalize()
    }
}

fn map_github_error(error: GitHubApiError) -> CoreError {
    match error {
        GitHubApiError::Unauthorized { message } | GitHubApiError::Forbidden { message } => {
            CoreError::PermissionDenied {
                provider: "github".to_string(),
                message,
            }
        }
        GitHubApiError::NotFound { message } => CoreError::NotFound {
            entity: "github repository resource".to_string(),
            id: message,
        },
        GitHubApiError::RateLimited { message, retry_at } => CoreError::Provider {
            provider: "github".to_string(),
            code: "rate_limited".to_string(),
            message,
            retry_at,
        },
        GitHubApiError::Server { status, message } => CoreError::Provider {
            provider: "github".to_string(),
            code: format!("http_{status}"),
            message,
            retry_at: None,
        },
        GitHubApiError::Network { message } => CoreError::Provider {
            provider: "github".to_string(),
            code: "network".to_string(),
            message,
            retry_at: None,
        },
        GitHubApiError::Malformed { message } => CoreError::Provider {
            provider: "github".to_string(),
            code: "malformed_response".to_string(),
            message,
            retry_at: None,
        },
    }
}

fn validate_connection_id(value: &str) -> CoreResult<()> {
    if value.is_empty()
        || value.len() > 120
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(CoreError::InvalidInput {
            field: "connection_id".to_string(),
            message: "must contain only letters, numbers, '.', '_' or '-'".to_string(),
        });
    }
    Ok(())
}

fn validate_segment(field: &str, value: &str) -> CoreResult<()> {
    if value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(CoreError::InvalidInput {
            field: field.to_string(),
            message: "contains an unsupported path character".to_string(),
        });
    }
    Ok(())
}

fn default_api_base_url() -> String {
    DEFAULT_API_BASE_URL.to_string()
}

fn default_page_size() -> u32 {
    30
}

fn parse_comments(value: &Value) -> Result<Vec<GitHubComment>, GitHubApiError> {
    let comments = value.as_array().ok_or_else(|| GitHubApiError::Malformed {
        message: "comments response was not an array".to_string(),
    })?;
    comments
        .iter()
        .map(|comment| {
            Ok(GitHubComment {
                author: comment
                    .get("user")
                    .and_then(|user| user.get("login"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                body: comment
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                created_at: comment
                    .get("created_at")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                updated_at: comment
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

pub(super) fn authenticated_login(token: &str) -> CoreResult<String> {
    let response = curl_json("https://api.github.com/user", token).map_err(map_github_error)?;
    response
        .get("login")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| CoreError::Connector {
            message: "GitHub account identity is missing".into(),
        })
}

fn curl_json(url: &str, token: &str) -> Result<Value, GitHubApiError> {
    if token.chars().any(char::is_control) || url.chars().any(char::is_control) {
        return Err(GitHubApiError::Network {
            message: "invalid request configuration".into(),
        });
    }
    let mut config = String::new();
    config.push_str("silent\nshow-error\n");
    config.push_str(&format!("url = {}\n", curl_config_quote(url)));
    config.push_str(&format!(
        "header = {}\n",
        curl_config_quote("Accept: application/vnd.github+json")
    ));
    config.push_str(&format!(
        "header = {}\n",
        curl_config_quote(&format!("Authorization: Bearer {token}"))
    ));
    config.push_str(&format!(
        "user-agent = {}\n",
        curl_config_quote("Aidebook/0.1")
    ));
    config.push_str("write-out = \"\\nAIDEBOOK_STATUS:%{http_code}\"\n");
    let mut child = Command::new("curl")
        .args([
            "-q",
            "--max-time",
            "30",
            "--connect-timeout",
            "10",
            "--max-filesize",
            "8388608",
            "--proto",
            "=https",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| GitHubApiError::Network {
            message: format!("curl could not start: {error}"),
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| GitHubApiError::Network {
            message: "curl stdin was unavailable".to_string(),
        })?
        .write_all(config.as_bytes())
        .map_err(|error| GitHubApiError::Network {
            message: format!("curl request could not be written: {error}"),
        })?;
    let output = child
        .wait_with_output()
        .map_err(|error| GitHubApiError::Network {
            message: format!("curl did not finish: {error}"),
        })?;
    if !output.status.success() {
        return Err(GitHubApiError::Network {
            message: "GitHub request failed or exceeded its limit".into(),
        });
    }
    let text = String::from_utf8(output.stdout).map_err(|_| GitHubApiError::Malformed {
        message: "GitHub response was not UTF-8".to_string(),
    })?;
    let marker = "\nAIDEBOOK_STATUS:";
    let (body, status) = text
        .rsplit_once(marker)
        .ok_or_else(|| GitHubApiError::Malformed {
            message: "GitHub response status was missing".to_string(),
        })?;
    let status = status
        .trim()
        .parse::<u16>()
        .map_err(|_| GitHubApiError::Malformed {
            message: "GitHub response status was invalid".to_string(),
        })?;
    if !(200..300).contains(&status) {
        return Err(match status {
            401 => GitHubApiError::Unauthorized {
                message: "GitHub authentication was rejected".to_string(),
            },
            403 => GitHubApiError::Forbidden {
                message: "GitHub repository permission was denied".to_string(),
            },
            404 => GitHubApiError::NotFound {
                message: url.to_string(),
            },
            429 => GitHubApiError::RateLimited {
                message: "GitHub rate limit exceeded".to_string(),
                retry_at: None,
            },
            status if status >= 500 => GitHubApiError::Server {
                status,
                message: "GitHub server error".to_string(),
            },
            status => GitHubApiError::Network {
                message: format!("GitHub returned HTTP {status}"),
            },
        });
    }
    serde_json::from_str(body).map_err(|error| GitHubApiError::Malformed {
        message: format!("GitHub JSON could not be decoded: {error}"),
    })
}

fn curl_config_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, kind: &str) -> GitHubIssue {
        GitHubIssue {
            number,
            title: format!("Item {number}"),
            body: Some("body".to_string()),
            kind: kind.to_string(),
            state: "open".to_string(),
            author: Some("alice".to_string()),
            updated_at: Some("2026-09-19T00:00:00Z".to_string()),
            html_url: format!("https://github.com/acme/book/{kind}/{number}"),
            comments: vec![GitHubComment {
                author: "bob".to_string(),
                body: "review".to_string(),
                created_at: None,
                updated_at: None,
            }],
            labels: vec!["mvp".to_string()],
        }
    }

    #[test]
    fn selected_scope_paginates_issues_and_comments_without_exposing_token() {
        let first = GitHubPage {
            items: vec![issue(1, "issue")],
            next_page: Some(2),
        };
        let second = GitHubPage {
            items: vec![issue(2, "pull_request")],
            next_page: None,
        };
        let api = FixtureGitHubApi::new([first, second]);
        let credentials = MemoryCredentialStore::new();
        credentials.set("selected", "ghp_fixture_secret").unwrap();
        let config = GitHubConfig::new("alice", "selected", "acme", "book");
        let adapter = GitHubAdapter::new(config, api, credentials).unwrap();
        let snapshots = adapter.list().unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].source.kind, "issue");
        assert_eq!(snapshots[1].source.kind, "pull_request");
        assert!(snapshots[0].body.contains("Comment by bob"));
        assert_eq!(snapshots[0].metadata["state"], "open");
        assert!(adapter.connect("acme/book").is_ok());
        assert!(matches!(
            adapter.connect("other/book"),
            Err(CoreError::PermissionDenied { .. })
        ));
    }

    #[test]
    fn provider_errors_are_structured_and_missing_credentials_are_blocked() {
        let credentials = MemoryCredentialStore::new();
        let config = GitHubConfig::new("alice", "selected", "acme", "book");
        let adapter = GitHubAdapter::new(
            config.clone(),
            FixtureGitHubApi::new([GitHubPage {
                items: vec![],
                next_page: None,
            }]),
            credentials.clone(),
        )
        .unwrap();
        assert!(matches!(
            adapter.list(),
            Err(CoreError::Provider { code, .. }) if code == "credential_missing"
        ));
        credentials.set("selected", "fixture").unwrap();
        let failing = GitHubAdapter::new(
            config,
            FixtureGitHubApi::new([GitHubPage {
                items: vec![],
                next_page: None,
            }])
            .with_error(
                1,
                GitHubApiError::RateLimited {
                    message: "slow down".to_string(),
                    retry_at: Some("2026-09-19T00:01:00Z".to_string()),
                },
            ),
            credentials,
        )
        .unwrap();
        assert!(matches!(
            failing.list(),
            Err(CoreError::Provider { code, retry_at: Some(_), .. }) if code == "rate_limited"
        ));
    }

    #[test]
    fn config_rejects_path_injection_and_bad_page_size() {
        let mut config = GitHubConfig::new("alice", "selected", "acme", "book");
        config.owner = "../other".to_string();
        assert!(
            matches!(config.validate(), Err(CoreError::InvalidInput { field, .. }) if field == "owner")
        );
        config.owner = "acme".to_string();
        config.per_page = 101;
        assert!(
            matches!(config.validate(), Err(CoreError::InvalidInput { field, .. }) if field == "per_page")
        );
    }
}
