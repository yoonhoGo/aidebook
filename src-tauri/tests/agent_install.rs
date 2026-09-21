use aidebook_lib::{
    agents::{Agent, AgentInstaller},
    core::{Core, CoreEndpoint, CoreServer},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
struct Fixture {
    root: PathBuf,
    home: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from("/tmp").join(format!("aia-{}", uuid::Uuid::new_v4()));
        let home = root.join("home with spaces");
        let data = root.join("app data");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&data).unwrap();
        Self { root, home, data }
    }
    fn installer(&self) -> AgentInstaller {
        AgentInstaller::new(
            self.home.clone(),
            self.data.clone(),
            PathBuf::from(env!("CARGO_BIN_EXE_aidebook")),
        )
    }
    fn write(&self, path: &str, text: &str) {
        let p = self.home.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn all_four_targets_install_idempotently_preserve_settings_and_uninstall_only_owned_entries() {
    let f = Fixture::new();
    f.write(
        ".codex/config.toml",
        "# retain comment\nmodel = 'example'\n[mcp_servers.other]\ncommand = 'other-command'\n",
    );
    f.write(".claude.json",r#"{"theme":"dark","projects":{"untouched":{}},"mcpServers":{"other":{"command":"other-command"}}}"#);
    f.write(
        ".hermes/config.yaml",
        "model: example\nmcp_servers:\n  other:\n    command: other-command\n",
    );
    let installer = f.installer();
    for agent in [Agent::Codex, Agent::ClaudeCode, Agent::Hermes, Agent::Pi] {
        let result = installer.install(agent).unwrap();
        assert!(PathBuf::from(&result.bundle_path)
            .join(".mcp.json")
            .is_file());
        assert!(
            installer
                .status()
                .iter()
                .find(|s| s.agent == agent)
                .unwrap()
                .installed
        );
        assert!(
            installer.install(agent).unwrap().backups.is_empty(),
            "repeat install must not churn configuration"
        );
    }
    let codex = fs::read_to_string(f.home.join(".codex/config.toml")).unwrap();
    assert!(codex.contains("# retain comment"));
    assert!(codex.contains("other-command"));
    let claude: Value =
        serde_json::from_slice(&fs::read(f.home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(claude["theme"], "dark");
    assert!(claude["projects"].get("untouched").is_some());
    let pi: Value = serde_json::from_slice(
        &fs::read(f.home.join(".pi/agent/extensions/aidebook/config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(pi["tools"].as_array().unwrap().len(), 13);
    assert!(!pi["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "candidate.accept"));
    for agent in [Agent::Codex, Agent::ClaudeCode, Agent::Hermes, Agent::Pi] {
        installer.uninstall(agent).unwrap();
    }
    assert!(installer.status().iter().all(|s| !s.installed));
    let claude: Value =
        serde_json::from_slice(&fs::read(f.home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(claude["mcpServers"]["other"]["command"], "other-command");
    assert!(claude["mcpServers"].get("aidebook").is_none());
    let hermes: Value =
        serde_yaml_ng::from_slice(&fs::read(f.home.join(".hermes/config.yaml")).unwrap()).unwrap();
    assert_eq!(hermes["model"], "example");
    assert_eq!(hermes["mcp_servers"]["other"]["command"], "other-command");
}
#[test]
fn conflicting_or_malformed_configuration_never_installs_partial_assets() {
    for original in [
        r#"{"mcpServers":{"aidebook":{"command":"user-owned"}}}"#,
        "{invalid",
    ] {
        let f = Fixture::new();
        f.write(".claude.json", original);
        assert!(f.installer().install(Agent::ClaudeCode).is_err());
        assert_eq!(
            fs::read_to_string(f.home.join(".claude.json")).unwrap(),
            original
        );
        assert!(!f.home.join(".claude/skills").exists());
        assert!(!f
            .data
            .join("agent-integrations/bin/aidebook-agent")
            .exists());
    }
}
#[test]
fn edited_skills_are_not_overwritten_or_removed() {
    let f = Fixture::new();
    let installer = f.installer();
    installer.install(Agent::Pi).unwrap();
    let skill = f.home.join(".pi/agent/skills/aidebook-pi/SKILL.md");
    fs::write(&skill, "user edited skill").unwrap();
    assert!(installer.install(Agent::Pi).is_err());
    installer.uninstall(Agent::Pi).unwrap();
    assert_eq!(fs::read_to_string(skill).unwrap(), "user edited skill");
}
#[test]
fn installed_app_binary_serves_real_mcp_and_pi_calls_without_launching_a_window() {
    let f = Fixture::new();
    let installer = f.installer();
    installer.install(Agent::Codex).unwrap();
    let core = Core::open(f.data.join("test.sqlite")).unwrap();
    let endpoint = CoreEndpoint::in_data_dir(&f.data);
    let server = CoreServer::bind(endpoint, core, None).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = std::thread::spawn(move || server.serve_until(worker_stop).unwrap());
    assert_eq!(installer.probe().unwrap()["connected"], true);
    let binary = f.data.join("agent-integrations/bin/aidebook-agent");
    let mut child = Command::new(&binary)
        .args(["--aidebook-agent", "mcp", "--data-dir"])
        .arg(&f.data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"context.search","arguments":{"query":"empty"}}}),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for r in requests {
        writeln!(stdin, "{r}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert_eq!(
        responses[1]["result"]["tools"].as_array().unwrap().len(),
        13
    );
    assert_eq!(responses[2]["result"]["isError"], false);
    let mut call = Command::new(&binary)
        .args(["--aidebook-agent", "call", "context.search", "--data-dir"])
        .arg(&f.data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    call.stdin
        .take()
        .unwrap()
        .write_all(br#"{"query":"empty"}"#)
        .unwrap();
    assert!(call.wait_with_output().unwrap().status.success());
    let denied = Command::new(&binary)
        .args(["--aidebook-agent", "call", "candidate.accept", "--data-dir"])
        .arg(&f.data)
        .output()
        .unwrap();
    assert!(!denied.status.success());
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    // Discovery is still usable while the app is closed; tool execution reports the unavailable owner.
    let mut offline = Command::new(&binary)
        .args(["--aidebook-agent", "mcp", "--data-dir"])
        .arg(&f.data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        offline.stdin.take().unwrap(),
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})
    )
    .unwrap();
    let output = offline.wait_with_output().unwrap();
    assert!(output.status.success());
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 13);
}
