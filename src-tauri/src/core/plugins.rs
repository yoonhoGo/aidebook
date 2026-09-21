//! Explicit, persistent connections. Local paths never depend on remote authentication.
use super::{
    types::*, Core, CredentialStore, GitHubAdapter, GitHubConfig, HttpGitHubApi,
    KeychainCredentialStore, MemoryCredentialStore, ObsidianAdapter, ReadOnlyConnector,
    VaultConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Obsidian,
    Github,
    Jira,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    Local,
    GhCli,
    Token,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConnection {
    pub id: String,
    pub provider: Provider,
    pub label: String,
    pub account: String,
    /// Absolute vault path, owner/repository, or Jira Cloud site.
    pub scope: String,
    #[serde(default)]
    pub project: String,
    pub auth: AuthMethod,
    #[serde(default = "default_auto_sync")]
    pub auto_sync: bool,
}
fn default_auto_sync() -> bool {
    true
}
fn invalid(message: &str) -> CoreError {
    CoreError::InvalidInput {
        field: "connection".into(),
        message: message.into(),
    }
}
fn failed(message: &str) -> CoreError {
    CoreError::Connector {
        message: message.into(),
    }
}
fn segment(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
impl PluginConnection {
    pub fn validate(&mut self) -> CoreResult<()> {
        if !segment(&self.id) || self.label.trim().is_empty() {
            return Err(invalid("connection ID and label are required"));
        }
        match self.provider {
            Provider::Obsidian => {
                if !std::path::Path::new(&self.scope).is_absolute() {
                    return Err(invalid("select an absolute vault path"));
                }
                if self.auth != AuthMethod::Local {
                    return Err(invalid("local paths do not require authentication"));
                }
                let adapter =
                    ObsidianAdapter::open(VaultConfig::new(&self.scope, &self.id, &self.id))?;
                self.scope = adapter.root_path().to_string_lossy().into_owned();
            }
            Provider::Github => {
                if !matches!(self.auth, AuthMethod::GhCli | AuthMethod::Token)
                    || !segment(&self.account)
                {
                    return Err(invalid(
                        "select a GitHub account and gh CLI or token authentication",
                    ));
                }
                let (owner, repo) = self
                    .scope
                    .split_once('/')
                    .ok_or_else(|| invalid("use owner/repository"))?;
                GitHubConfig::new(&self.account, &self.id, owner, repo).validate()?;
            }
            Provider::Jira => {
                self.scope = self.scope.trim_end_matches('/').into();
                let tenant = self
                    .scope
                    .strip_prefix("https://")
                    .and_then(|s| s.strip_suffix(".atlassian.net"));
                if !tenant.is_some_and(|s| {
                    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }) {
                    return Err(invalid(
                        "Jira Cloud site must be https://your-site.atlassian.net",
                    ));
                }
                if self.auth != AuthMethod::Token
                    || !self.account.contains('@')
                    || self.account.contains([':', '\r', '\n'])
                {
                    return Err(invalid(
                        "Jira requires an account email and personal API token",
                    ));
                }
                if self.project.is_empty()
                    || !self
                        .project
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                {
                    return Err(invalid("enter an uppercase Jira project key"));
                }
            }
        }
        Ok(())
    }
}

pub struct PluginRegistry {
    path: PathBuf,
    connections: Vec<PluginConnection>,
}
impl PluginRegistry {
    pub fn open(path: PathBuf) -> CoreResult<Self> {
        let connections = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| failed("connection settings could not be decoded"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err(failed("connection settings could not be read")),
        };
        Ok(Self { path, connections })
    }
    pub fn list(&self) -> Vec<PluginConnection> {
        let mut result = self.connections.clone();
        result.sort_by_key(|c| match c.provider {
            Provider::Obsidian => 0,
            Provider::Github => 1,
            Provider::Jira => 2,
        });
        result
    }
    pub fn get(&self, id: &str) -> CoreResult<PluginConnection> {
        self.connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| invalid("connection not found"))
    }
    pub fn add(&mut self, mut connection: PluginConnection) -> CoreResult<PluginConnection> {
        connection.validate()?;
        if self.connections.iter().any(|c| c.id == connection.id) {
            return Err(invalid("connection ID already exists"));
        }
        let mut next = self.connections.clone();
        next.push(connection.clone());
        self.persist(next)?;
        Ok(connection)
    }
    pub fn set_auto_sync(&mut self, id: &str, enabled: bool) -> CoreResult<()> {
        let connection = self.get(id)?;
        if connection.provider != Provider::Obsidian {
            return Err(invalid(
                "automatic refresh currently supports local vaults only",
            ));
        }
        let mut next = self.connections.clone();
        next.iter_mut().find(|c| c.id == id).unwrap().auto_sync = enabled;
        self.persist(next)
    }
    pub fn remove(&mut self, id: &str) -> CoreResult<()> {
        self.get(id)?;
        let next = self
            .connections
            .iter()
            .filter(|c| c.id != id)
            .cloned()
            .collect();
        self.persist(next)
    }
    fn persist(&mut self, next: Vec<PluginConnection>) -> CoreResult<()> {
        let bytes = serde_json::to_vec_pretty(&next)
            .map_err(|_| failed("connection settings could not be encoded"))?;
        let temp = self.path.with_extension("tmp");
        fs::write(&temp, bytes)
            .and_then(|_| fs::rename(&temp, &self.path))
            .map_err(|_| failed("connection settings could not be saved"))?;
        self.connections = next;
        Ok(())
    }
}

pub fn credential_key(id: &str) -> String {
    format!("plugin-{id}")
}
pub fn set_token(connection: &PluginConnection, token: &str) -> CoreResult<()> {
    if connection.auth != AuthMethod::Token {
        return Err(invalid("this connection does not use a stored token"));
    }
    if token.trim().is_empty() || token.chars().any(char::is_control) {
        return Err(invalid("token is empty or contains control characters"));
    }
    KeychainCredentialStore.set(&credential_key(&connection.id), token)
}
fn token(connection: &PluginConnection) -> CoreResult<String> {
    if connection.auth == AuthMethod::GhCli {
        // Never switch the globally active account; inherited tokens must not override --user.
        let output = Command::new("gh")
            .args([
                "auth",
                "token",
                "--hostname",
                "github.com",
                "--user",
                &connection.account,
            ])
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_ENTERPRISE_TOKEN")
            .env_remove("GITHUB_ENTERPRISE_TOKEN")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| {
                failed("GitHub CLI is unavailable; install gh or select token authentication")
            })?;
        if !output.status.success() {
            return Err(failed("no reusable gh login for this account; run gh auth login or select token authentication"));
        }
        let value = String::from_utf8(output.stdout)
            .map_err(|_| failed("gh returned an invalid credential"))?;
        let value = value.trim().to_string();
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(failed("gh returned an invalid credential"));
        }
        return Ok(value);
    }
    KeychainCredentialStore
        .get(&credential_key(&connection.id))?
        .ok_or_else(|| failed("save an API token for this connection first"))
}

pub fn refresh(core: &Core, connection: PluginConnection) -> CoreResult<SourcesRefreshResult> {
    let result = refresh_inner(core, connection.clone());
    if let Err(error) = &result {
        let provider = match connection.provider {
            Provider::Obsidian => "obsidian",
            Provider::Github => "github",
            Provider::Jira => "jira",
        };
        let _ = core.record_sync_failure(
            &connection.id,
            provider,
            &connection.scope,
            "plugin_refresh_failed",
            error.to_string(),
            None,
            None,
        );
    }
    result
}

fn refresh_inner(
    core: &Core,
    mut connection: PluginConnection,
) -> CoreResult<SourcesRefreshResult> {
    connection.validate()?;
    match connection.provider {
        Provider::Obsidian => core.sources_refresh(&ObsidianAdapter::open(VaultConfig::new(
            &connection.scope,
            &connection.id,
            &connection.id,
        ))?),
        Provider::Github => {
            let (owner, repo) = connection
                .scope
                .split_once('/')
                .ok_or_else(|| invalid("use owner/repository"))?;
            let credentials = MemoryCredentialStore::new();
            let credential = token(&connection)?;
            if !super::github::authenticated_login(&credential)?
                .eq_ignore_ascii_case(&connection.account)
            {
                return Err(invalid("GitHub credential belongs to a different account"));
            }
            credentials.set(&connection.id, &credential)?;
            core.sources_refresh(&GitHubAdapter::new(
                GitHubConfig::new(&connection.account, &connection.id, owner, repo),
                HttpGitHubApi,
                credentials,
            )?)
        }
        Provider::Jira => core.sources_refresh(&JiraConnector { connection }),
    }
}

struct JiraConnector {
    connection: PluginConnection,
}
impl ReadOnlyConnector for JiraConnector {
    fn manifest(&self) -> ConnectorManifest {
        ConnectorManifest {
            id: "jira".into(),
            version: "0.1.0".into(),
            api_version: "1".into(),
            capabilities: vec!["list".into(), "fetch".into()],
            permissions: vec!["read:jira-work".into()],
        }
    }
    fn connection_id(&self) -> &str {
        &self.connection.id
    }
    fn fetch(&self, source: &SourceRef) -> CoreResult<Snapshot> {
        self.list()?
            .into_iter()
            .find(|s| &s.source == source)
            .ok_or_else(|| failed("issue not found in selected project"))
    }
    fn list(&self) -> CoreResult<Vec<Snapshot>> {
        let credential = token(&self.connection)?;
        jira_list_with(&self.connection, |body| {
            jira_request(&self.connection, &credential, body)
        })
    }
}
fn jira_list_with(
    c: &PluginConnection,
    mut request: impl FnMut(&Value) -> CoreResult<Value>,
) -> CoreResult<Vec<Snapshot>> {
    let mut snapshots = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..1000 {
        let mut body = json!({"jql": format!("project = {} ORDER BY key ASC", c.project), "maxResults": 100,
                "fields": ["summary", "status", "updated"]});
        if let Some(value) = &cursor {
            body["nextPageToken"] = json!(value);
        }
        let response = request(&body)?;
        snapshots.extend(jira_snapshots(c, &response)?);
        if response.get("isLast").and_then(Value::as_bool) == Some(true) {
            return Ok(snapshots);
        }
        cursor = response
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::to_string);
        match &cursor {
            None => {
                return Err(failed(
                    "Jira pagination is incomplete; existing cache retained",
                ))
            }
            Some(value) if !seen.insert(value.clone()) => {
                return Err(failed("Jira repeated a pagination cursor"))
            }
            _ => (),
        }
    }
    Err(failed(
        "Jira pagination limit reached; existing cache retained",
    ))
}

fn jira_snapshots(c: &PluginConnection, response: &Value) -> CoreResult<Vec<Snapshot>> {
    response
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| failed("Jira issue list is missing"))?
        .iter()
        .map(|issue| {
            let key = issue
                .get("key")
                .and_then(Value::as_str)
                .filter(|key| segment(key) && key.starts_with(&format!("{}-", c.project)))
                .ok_or_else(|| failed("Jira returned an invalid issue key"))?;
            let fields = &issue["fields"];
            let summary = fields["summary"]
                .as_str()
                .ok_or_else(|| failed("Jira summary is missing"))?;
            Ok(Snapshot::new(
                SourceRef::new(
                    "jira",
                    &c.account,
                    format!("{}:{key}", c.scope),
                    format!("{}/browse/{key}", c.scope),
                    "issue",
                ),
                summary,
                fields["status"]["name"].as_str().unwrap_or(""),
                fields["updated"].as_str().map(str::to_string),
                now_rfc3339(),
            ))
        })
        .collect()
}
fn quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
fn jira_request(c: &PluginConnection, token: &str, body: &Value) -> CoreResult<Value> {
    if token.chars().any(char::is_control) {
        return Err(invalid("invalid token"));
    }
    let config = format!("url = {}\nuser = {}\nheader = \"Content-Type: application/json\"\ndata = {}\nwrite-out = \"\\nAIDEBOOK_STATUS:%{{http_code}}\"\n",
        quote(&format!("{}/rest/api/3/search/jql", c.scope)), quote(&format!("{}:{token}", c.account)), quote(&body.to_string()));
    let mut child = Command::new("curl")
        .args([
            "-q",
            "--silent",
            "--show-error",
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
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| failed("Jira transport could not start"))?;
    if child
        .stdin
        .take()
        .ok_or_else(|| failed("Jira request unavailable"))?
        .write_all(config.as_bytes())
        .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(failed("Jira request could not be sent"));
    }
    let output = child
        .wait_with_output()
        .map_err(|_| failed("Jira request failed"))?;
    if !output.status.success() {
        return Err(failed("Jira network request failed or exceeded its limit"));
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| failed("Jira returned invalid text"))?;
    let (body, status) = text
        .rsplit_once("\nAIDEBOOK_STATUS:")
        .ok_or_else(|| failed("Jira response status missing"))?;
    if status != "200" {
        return Err(failed(match status {
            "401" => "Jira authentication expired or was rejected",
            "403" => "Jira project access denied",
            "429" => "Jira rate limit exceeded; retry later",
            _ => "Jira request was rejected",
        }));
    }
    serde_json::from_str(body).map_err(|_| failed("Jira returned invalid JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection(id: &str, provider: Provider, scope: &str) -> PluginConnection {
        let local = provider == Provider::Obsidian;
        PluginConnection {
            id: id.into(),
            provider,
            label: id.into(),
            account: "test@example.com".into(),
            scope: scope.into(),
            project: "TEST".into(),
            auto_sync: true,
            auth: if local {
                AuthMethod::Local
            } else {
                AuthMethod::Token
            },
        }
    }
    #[test]
    fn multiple_paths_persist_and_local_always_sorts_first() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let path = root.join("connections.json");
        let mut registry = PluginRegistry::open(path.clone()).unwrap();
        registry
            .add(connection(
                "jira",
                Provider::Jira,
                "https://example.atlassian.net",
            ))
            .unwrap();
        for id in ["vault-a", "vault-b"] {
            let vault = root.join(id);
            fs::create_dir(&vault).unwrap();
            fs::write(vault.join("same.md"), id).unwrap();
            registry
                .add(connection(id, Provider::Obsidian, vault.to_str().unwrap()))
                .unwrap();
        }
        let restored = PluginRegistry::open(path).unwrap();
        assert_eq!(restored.list()[0].id, "vault-a");
        assert_eq!(restored.list().len(), 3);
        let core = Core::open(root.join("core.sqlite")).unwrap();
        for c in restored
            .list()
            .into_iter()
            .filter(|c| c.provider == Provider::Obsidian)
        {
            assert_eq!(refresh(&core, c).unwrap().indexed, 1);
        }
        registry.remove("vault-a").unwrap();
        assert!(registry.get("vault-b").is_ok());
        assert!(registry.get("jira").is_ok());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn same_repository_supports_distinct_accounts_and_rejects_id_overwrite() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let path = root.join("connections.json");
        let mut registry = PluginRegistry::open(path.clone()).unwrap();
        for account in ["alice", "bob"] {
            let mut c = connection(account, Provider::Github, "team/repository");
            c.account = account.into();
            c.auth = AuthMethod::GhCli;
            registry.add(c.clone()).unwrap();
            assert!(registry.add(c).is_err());
        }
        assert_eq!(PluginRegistry::open(path).unwrap().list().len(), 2);
        registry.remove("alice").unwrap();
        assert_eq!(registry.get("bob").unwrap().account, "bob");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn jira_paginates_and_rejects_partial_or_repeated_pages() {
        let c = connection("jira", Provider::Jira, "https://example.atlassian.net");
        let mut page = 0;
        let snapshots = jira_list_with(&c, |body| {
            page += 1;
            if page == 1 { assert!(body.get("nextPageToken").is_none()); }
            else { assert_eq!(body["nextPageToken"], "next"); }
            Ok(json!({"issues":[{"key":format!("TEST-{page}"),"fields":{"summary":"Issue"}}],"isLast":page==2,"nextPageToken":"next"}))
        }).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert!(jira_list_with(&c, |_| Ok(
            json!({"issues":[],"isLast":false,"nextPageToken":"same"})
        ))
        .is_err());
        assert!(jira_list_with(&c, |_| Ok(json!({"issues":[],"isLast":false}))).is_err());
    }
    #[test]
    fn jira_rejects_untrusted_hosts_and_maps_site_identity() {
        for host in [
            "http://example.atlassian.net",
            "https://example.atlassian.net.evil.com",
            "https://u:p@example.atlassian.net",
            "https://example.atlassian.net/x",
        ] {
            assert!(connection("jira", Provider::Jira, host).validate().is_err());
        }
        let c = connection("jira", Provider::Jira, "https://example.atlassian.net");
        let result = jira_snapshots(&c, &json!({"issues":[{"key":"TEST-1","fields":{"summary":"Issue","status":{"name":"Open"}}}]})).unwrap();
        assert!(result[0]
            .source
            .external_id
            .contains("example.atlassian.net"));
        assert!(jira_snapshots(
            &c,
            &json!({"issues":[{"key":"OTHER-1","fields":{"summary":"Wrong project"}}]})
        )
        .is_err());
    }
}
