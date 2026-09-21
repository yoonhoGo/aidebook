pub mod bridge;
use crate::core::{mcp, CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

const SKILL: &str = include_str!("../../../integrations/aidebook/skills/aidebook-memory/SKILL.md");
const PI: &str = include_str!("../../../integrations/aidebook/pi-extension.ts");
const CODEX_MANIFEST: &str =
    include_str!("../../../integrations/aidebook/.codex-plugin/plugin.json");
const CLAUDE_MANIFEST: &str =
    include_str!("../../../integrations/aidebook/.claude-plugin/plugin.json");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Codex,
    ClaudeCode,
    Hermes,
    Pi,
}
impl Agent {
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::Hermes => "hermes",
            Self::Pi => "pi",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
            Self::Hermes => "Hermes",
            Self::Pi => "Pi",
        }
    }
}
#[derive(Serialize)]
pub struct AgentStatus {
    pub agent: Agent,
    pub label: String,
    pub installed: bool,
    pub managed: bool,
    pub config_path: Option<String>,
    pub skill_path: String,
    pub bundle_path: String,
    pub mode: String,
    pub issue: Option<String>,
}
#[derive(Serialize)]
pub struct InstallResult {
    pub message: String,
    pub backups: Vec<String>,
    pub bundle_path: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Receipt {
    server: Value,
    assets: BTreeMap<PathBuf, String>,
}
struct Change {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}
pub struct AgentInstaller {
    home: PathBuf,
    data: PathBuf,
    executable: PathBuf,
    lock: Mutex<()>,
}
fn error(message: impl Into<String>) -> CoreError {
    CoreError::Connector {
        message: message.into(),
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &Path) -> CoreResult<Option<Vec<u8>>> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(error(format!(
            "심볼릭 링크 설정 파일은 자동 변경하지 않습니다: {}",
            path.display()
        )));
    }
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(error(format!(
            "파일을 읽을 수 없습니다: {}",
            path.display()
        ))),
    }
}
impl AgentInstaller {
    pub fn new(home: PathBuf, data: PathBuf, executable: PathBuf) -> Self {
        Self {
            home,
            data,
            executable,
            lock: Mutex::new(()),
        }
    }
    fn root(&self) -> PathBuf {
        self.data.join("agent-integrations")
    }
    fn binary(&self) -> PathBuf {
        self.root().join("bin/aidebook-agent")
    }
    fn bundle(&self, agent: Agent) -> PathBuf {
        self.root().join(agent.id()).join("aidebook")
    }
    fn receipt_path(&self, agent: Agent) -> PathBuf {
        self.root().join(format!("{}.receipt.json", agent.id()))
    }
    fn config(&self, agent: Agent) -> Option<PathBuf> {
        match agent {
            Agent::Codex => Some(self.home.join(".codex/config.toml")),
            Agent::ClaudeCode => Some(self.home.join(".claude.json")),
            Agent::Hermes => Some(self.home.join(".hermes/config.yaml")),
            Agent::Pi => None,
        }
    }
    fn skill(&self, agent: Agent) -> PathBuf {
        let base = match agent {
            Agent::Codex => ".agents/skills",
            Agent::ClaudeCode => ".claude/skills",
            Agent::Hermes => ".hermes/skills",
            Agent::Pi => ".pi/agent/skills",
        };
        self.home
            .join(base)
            .join(format!("aidebook-{}", agent.id().replace('_', "-")))
            .join("SKILL.md")
    }
    fn server(&self, agent: Agent) -> Value {
        let mut value = json!({"command":self.binary(),"args":["--aidebook-agent","mcp","--data-dir",self.data]});
        if agent == Agent::ClaudeCode {
            value["type"] = json!("stdio");
        }
        value
    }
    fn receipt(&self, agent: Agent) -> CoreResult<Option<Receipt>> {
        read(&self.receipt_path(agent))?
            .map(|v| {
                serde_json::from_slice(&v)
                    .map_err(|_| error("설치 기록을 읽을 수 없습니다. 기존 설정을 보존했습니다."))
            })
            .transpose()
    }
    fn assets(&self, agent: Agent) -> CoreResult<BTreeMap<PathBuf, Vec<u8>>> {
        let mut files = BTreeMap::new();
        let name = format!("aidebook-{}", agent.id().replace('_', "-"));
        files.insert(
            self.skill(agent),
            SKILL
                .replacen("name: aidebook-memory", &format!("name: {name}"), 1)
                .into_bytes(),
        );
        let bundle = self.bundle(agent);
        files.insert(
            bundle.join("skills/aidebook-memory/SKILL.md"),
            SKILL.as_bytes().to_vec(),
        );
        files.insert(
            bundle.join(".mcp.json"),
            serde_json::to_vec_pretty(&json!({"mcpServers":{"aidebook":self.server(agent)}}))
                .unwrap(),
        );
        files.insert(
            bundle.join(".codex-plugin/plugin.json"),
            CODEX_MANIFEST.as_bytes().to_vec(),
        );
        files.insert(
            bundle.join(".claude-plugin/plugin.json"),
            CLAUDE_MANIFEST.as_bytes().to_vec(),
        );
        if agent == Agent::Pi {
            let extension = self.home.join(".pi/agent/extensions/aidebook");
            files.insert(extension.join("index.ts"), PI.as_bytes().to_vec());
            files.insert(extension.join("config.json"), serde_json::to_vec_pretty(&json!({"command":self.binary(),"dataDir":self.data,"tools":mcp::tool_descriptors()})).unwrap());
        }
        Ok(files)
    }
    pub fn status(&self) -> Vec<AgentStatus> {
        [Agent::Codex, Agent::ClaudeCode, Agent::Hermes, Agent::Pi]
            .into_iter()
            .map(|agent| {
                let checked = (|| -> CoreResult<bool> {
                    let Some(receipt) = self.receipt(agent)? else {
                        return Ok(false);
                    };
                    if !self.binary().is_file() {
                        return Err(error("연결 실행 파일이 없습니다. 다시 설치하세요."));
                    }
                    for (path, expected) in self.assets(agent)? {
                        let bytes =
                            read(&path)?.ok_or_else(|| error("설치 파일이 누락되었습니다."))?;
                        if bytes != expected || receipt.assets.get(&path) != Some(&hash(&bytes)) {
                            return Err(error("설치 파일이 변경되었거나 업데이트가 필요합니다."));
                        }
                    }
                    if let Some(path) = self.config(agent) {
                        let text =
                            read(&path)?.ok_or_else(|| error("MCP 설정 파일이 없습니다."))?;
                        let (_, current) = config_edit(agent, &text, None)?;
                        if current.as_ref() != Some(&receipt.server) {
                            return Err(error("Aidebook MCP 항목이 변경되었거나 없습니다."));
                        }
                    }
                    Ok(true)
                })();
                AgentStatus {
                    agent,
                    label: agent.label().into(),
                    installed: matches!(checked, Ok(true)),
                    managed: self.receipt_path(agent).exists(),
                    issue: checked.err().map(|e| e.to_string()),
                    config_path: self.config(agent).map(|p| p.display().to_string()),
                    skill_path: self.skill(agent).display().to_string(),
                    bundle_path: self.bundle(agent).display().to_string(),
                    mode: if agent == Agent::Pi {
                        "스킬 + 확장 플러그인"
                    } else {
                        "스킬 + MCP"
                    }
                    .into(),
                }
            })
            .collect()
    }
    pub fn install(&self, agent: Agent) -> CoreResult<InstallResult> {
        let _guard = self.lock.lock().map_err(|_| error("설치 작업 잠금 오류"))?;
        let receipt = self.receipt(agent)?;
        let server = self.server(agent);
        let assets = self.assets(agent)?;
        let mut changes = Vec::new();
        if let Some(path) = self.config(agent) {
            let before = read(&path)?;
            let text = before.as_deref().unwrap_or(b"");
            let (after, current) = config_edit(agent, text, Some(Some(&server)))?;
            if current.is_some() && current.as_ref() != receipt.as_ref().map(|r| &r.server) {
                return Err(error(
                    "이미 다른 aidebook MCP 설정이 있습니다. 기존 항목을 변경하지 않았습니다.",
                ));
            }
            changes.push(Change {
                path,
                before,
                after: Some(after),
            });
        }
        for (path, bytes) in &assets {
            let before = read(path)?;
            if let Some(existing) = &before {
                if existing != bytes
                    && receipt.as_ref().and_then(|r| r.assets.get(path)) != Some(&hash(existing))
                {
                    return Err(error(format!(
                        "사용자가 변경한 파일은 덮어쓰지 않습니다: {}",
                        path.display()
                    )));
                }
            }
            changes.push(Change {
                path: path.clone(),
                before,
                after: Some(bytes.clone()),
            });
        }
        let next = Receipt {
            server,
            assets: assets.iter().map(|(p, b)| (p.clone(), hash(b))).collect(),
        };
        let path = self.receipt_path(agent);
        changes.push(Change {
            before: read(&path)?,
            path,
            after: Some(serde_json::to_vec_pretty(&next).unwrap()),
        });
        // All config parsing and conflict checks precede the first installation write.
        self.install_binary()?;
        let backups = apply(changes)?;
        Ok(InstallResult {
            message: format!(
                "{} 연결 설정을 설치했습니다. 에이전트의 새 세션에서 사용하세요.",
                agent.label()
            ),
            backups,
            bundle_path: self.bundle(agent).display().to_string(),
        })
    }
    pub fn uninstall(&self, agent: Agent) -> CoreResult<InstallResult> {
        let _guard = self.lock.lock().map_err(|_| error("설치 작업 잠금 오류"))?;
        let receipt = self
            .receipt(agent)?
            .ok_or_else(|| error("Aidebook이 설치한 기록이 없습니다."))?;
        let mut changes = Vec::new();
        let mut retained = 0;
        if let Some(path) = self.config(agent) {
            if let Some(before) = read(&path)? {
                let (after, current) = config_edit(agent, &before, Some(None))?;
                if current.is_some() && current.as_ref() != Some(&receipt.server) {
                    return Err(error("MCP 항목이 수정되어 자동 해제하지 않았습니다."));
                }
                changes.push(Change {
                    path,
                    before: Some(before),
                    after: Some(after),
                });
            }
        }
        // Derive paths from our known target set, never trust arbitrary receipt paths for deletion.
        for path in self.assets(agent)?.keys() {
            if let Some(before) = read(path)? {
                if receipt.assets.get(path) == Some(&hash(&before)) {
                    changes.push(Change {
                        path: path.clone(),
                        before: Some(before),
                        after: None,
                    });
                } else {
                    retained += 1;
                }
            }
        }
        let path = self.receipt_path(agent);
        changes.push(Change {
            before: read(&path)?,
            path,
            after: None,
        });
        let backups = apply(changes)?;
        Ok(InstallResult {
            message: format!(
                "연결을 해제했습니다. 사용자 수정 파일 {retained}개와 기존 메모를 보존했습니다."
            ),
            backups,
            bundle_path: self.bundle(agent).display().to_string(),
        })
    }
    fn install_binary(&self) -> CoreResult<()> {
        let destination = self.binary();
        fs::create_dir_all(destination.parent().unwrap())
            .map_err(|_| error("연결 실행 파일 폴더를 만들 수 없습니다."))?;
        let temp = destination.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::copy(&self.executable, &temp)
            .map_err(|_| error("현재 앱 실행 파일을 복사할 수 없습니다."))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o700))
                .map_err(|_| error("실행 권한을 설정할 수 없습니다."))?;
        }
        fs::rename(&temp, &destination)
            .map_err(|_| error("연결 실행 파일을 설치할 수 없습니다."))?;
        Ok(())
    }
    pub fn probe(&self) -> CoreResult<Value> {
        let mut child = Command::new(self.binary())
            .args(["--aidebook-agent", "probe", "--data-dir"])
            .arg(&self.data)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| error("연결 실행 파일이 없습니다. 먼저 연결을 설치하세요."))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if child
                .try_wait()
                .map_err(|_| error("연결 확인 실패"))?
                .is_some()
            {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error(
                    "응답 시간이 초과되었습니다. Aidebook 앱을 다시 실행하세요.",
                ));
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let output = child
            .wait_with_output()
            .map_err(|_| error("연결 확인 실패"))?;
        if !output.status.success() {
            return Err(error(
                "로컬 코어에 연결하지 못했습니다. 앱 실행 상태를 확인하세요.",
            ));
        }
        serde_json::from_slice(&output.stdout).map_err(|_| error("연결 응답이 올바르지 않습니다."))
    }
}

/// Returns a rewritten document and its previous Aidebook entry. Only that entry is owned.
/// None = inspect; Some(Some(server)) = install; Some(None) = remove.
fn config_edit(
    agent: Agent,
    bytes: &[u8],
    update: Option<Option<&Value>>,
) -> CoreResult<(Vec<u8>, Option<Value>)> {
    let text = std::str::from_utf8(bytes).map_err(|_| error("설정 파일이 UTF-8이 아닙니다."))?;
    let empty = text.trim().is_empty();
    let key = if agent == Agent::ClaudeCode {
        "mcpServers"
    } else {
        "mcp_servers"
    };
    let mut root: Value = if empty {
        json!({})
    } else {
        match agent {
            Agent::Codex => toml_edit::de::from_str(text)
                .map_err(|_| error("Codex TOML 설정을 해석할 수 없습니다. 변경하지 않았습니다."))?,
            Agent::Hermes => {
                let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(text).map_err(|_| {
                    error("Hermes YAML 설정을 해석할 수 없습니다. 변경하지 않았습니다.")
                })?;
                serde_json::to_value(yaml)
                    .map_err(|_| error("지원하지 않는 Hermes YAML 형식입니다."))?
            }
            _ => serde_json::from_str(text).map_err(|_| {
                error("Claude JSON 설정을 해석할 수 없습니다. 변경하지 않았습니다.")
            })?,
        }
    };
    let map = root
        .as_object_mut()
        .ok_or_else(|| error("설정의 최상위 값은 객체여야 합니다."))?;
    if map.get(key).is_some_and(|v| !v.is_object()) {
        return Err(error("기존 MCP 설정은 객체여야 합니다."));
    }
    let current = map.get(key).and_then(|m| m.get("aidebook")).cloned();
    if update.is_none() {
        return Ok((bytes.to_vec(), current));
    }
    let update = update.unwrap();
    if let Some(value) = update {
        map.entry(key)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .insert("aidebook".into(), value.clone());
    } else if let Some(servers) = map.get_mut(key) {
        servers.as_object_mut().unwrap().remove("aidebook");
    }
    let output = match agent {
        Agent::Codex => {
            let mut doc = text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| error("Codex TOML 설정 오류"))?;
            if let Some(value) = update {
                if doc.get(key).is_some_and(|v| !v.is_table()) {
                    return Err(error(
                        "인라인 MCP 테이블은 수동 변환 후 설치하세요. 기존 설정을 보존했습니다.",
                    ));
                }
                if doc.get(key).is_none() {
                    doc[key] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                let table =
                    toml_edit::ser::to_document(value).map_err(|_| error("MCP 설정 생성 실패"))?;
                doc[key]["aidebook"] = toml_edit::Item::Table(table.as_table().clone());
            } else if let Some(table) = doc.get_mut(key).and_then(|v| v.as_table_mut()) {
                table.remove("aidebook");
            }
            doc.to_string()
        }
        Agent::Hermes => {
            serde_yaml_ng::to_string(&root).map_err(|_| error("YAML 설정 생성 실패"))?
        }
        _ => serde_json::to_string_pretty(&root).map_err(|_| error("JSON 설정 생성 실패"))?,
    };
    Ok((output.into_bytes(), current))
}
fn atomic_write(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    fs::create_dir_all(path.parent().ok_or_else(|| error("잘못된 설치 경로"))?)
        .map_err(|_| error("설치 폴더 생성 실패"))?;
    let temp = path.with_extension(format!("aidebook-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
        return Err(error(format!("파일 저장 실패: {}", path.display())));
    }
    Ok(())
}
fn apply(changes: Vec<Change>) -> CoreResult<Vec<String>> {
    let changes: Vec<_> = changes
        .into_iter()
        .filter(|c| c.before != c.after)
        .collect();
    for c in &changes {
        if read(&c.path)? != c.before {
            return Err(error(
                "설정이 다른 프로그램에서 변경되었습니다. 다시 시도하세요.",
            ));
        }
    }
    let mut backups = Vec::new();
    let mut completed: Vec<&Change> = Vec::new();
    for c in &changes {
        let result = (|| {
            if read(&c.path)? != c.before {
                return Err(error("설정이 설치 중 변경되었습니다."));
            }
            if let Some(before) = &c.before {
                let backup = c
                    .path
                    .with_extension(format!("aidebook-backup-{}", uuid::Uuid::new_v4()));
                atomic_write(&backup, before)?;
                backups.push(backup.display().to_string());
            }
            match &c.after {
                Some(bytes) => atomic_write(&c.path, bytes),
                None => fs::remove_file(&c.path).map_err(|_| error("설치 파일 해제 실패")),
            }
        })();
        if let Err(failure) = result {
            for done in completed.into_iter().rev() {
                if read(&done.path).ok() == Some(done.after.clone()) {
                    match &done.before {
                        Some(bytes) => {
                            let _ = atomic_write(&done.path, bytes);
                        }
                        None => {
                            let _ = fs::remove_file(&done.path);
                        }
                    }
                }
            }
            return Err(failure);
        }
        completed.push(c);
    }
    Ok(backups)
}
