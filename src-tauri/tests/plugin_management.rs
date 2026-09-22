use aidebook_lib::core::{
    local_sync::LocalSync, plugins::PluginRegistry, Core, CoreClient, CoreEndpoint, CoreServer,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

struct Fixture {
    root: PathBuf,
    core: Core,
    sync: Arc<LocalSync>,
    client: CoreClient,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(format!("/tmp/ab-plugins-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let core = Core::open(root.join("core.sqlite")).unwrap();
        let registry = Arc::new(Mutex::new(
            PluginRegistry::open(root.join("connections.json")).unwrap(),
        ));
        let sync = Arc::new(LocalSync::new(core.clone(), registry));
        let endpoint = CoreEndpoint::in_data_dir(&root);
        let server = CoreServer::bind(endpoint.clone(), core.clone(), None)
            .unwrap()
            .with_plugins(sync.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = Some(std::thread::spawn(move || {
            server.serve_until(stopped).unwrap()
        }));
        let client = CoreClient::from_endpoint(endpoint).unwrap();
        Self {
            root,
            core,
            sync,
            client,
            stop,
            thread,
        }
    }
    fn vault(&self, id: &str) -> Value {
        let path = self.root.join(format!("{id} notes"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("A.md"), format!("# {id}\nOriginal content")).unwrap();
        json!({"id":id,"provider":"obsidian","label":id,"account":"","scope":path.canonicalize().unwrap(),"auth":"local","auto_sync":false})
    }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_aidebook-cli"))
            .args(args)
            .arg("--data-dir")
            .arg(&self.root)
            .output()
            .unwrap()
    }
    fn cli_json(&self, args: &[&str]) -> Value {
        let result = self.cli(args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    }
    fn mcp(&self, name: &str, arguments: Value) -> Value {
        let mut child = Command::new(env!("CARGO_BIN_EXE_aidebook-cli"))
            .args(["mcp", "serve", "--stdio", "--data-dir"])
            .arg(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{}", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}})).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["result"]["isError"], false, "{response}");
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn cli_mcp_and_app_share_persistent_multi_vault_crud_and_real_refresh() {
    let f = Fixture::new();
    let first = f.vault("existing");
    let second = f.vault("bbros");
    f.client.call("plugins.add", first.clone()).unwrap();
    // Exercise shell-friendly flags, including paths with spaces.
    let added = f.cli_json(&[
        "plugins",
        "add",
        "--id",
        "bbros",
        "--provider",
        "obsidian",
        "--label",
        "BBros",
        "--scope",
        second["scope"].as_str().unwrap(),
        "--auto-sync",
        "false",
    ]);
    assert_eq!(added["auth"], "local");
    assert_eq!(f.sync.connections().unwrap().len(), 2);
    let updated = f.mcp(
        "plugins.update",
        json!({"id":"bbros","changes":{"label":"Company","auto_sync":true}}),
    );
    assert_eq!(updated["label"], "Company");
    assert_eq!(updated["scope"], second["scope"]);
    f.sync.poll_once(&AtomicBool::new(false)).unwrap();
    assert_eq!(f.core.cached_sources("obsidian", "bbros").unwrap().len(), 1);
    assert_eq!(
        f.cli_json(&["plugins", "refresh", "--id", "bbros"])["indexed"],
        1
    );
    assert_eq!(
        f.mcp("plugins.list", json!({})).as_array().unwrap().len(),
        2
    );
    let persisted = PluginRegistry::open(f.root.join("connections.json")).unwrap();
    assert_eq!(persisted.get("bbros").unwrap().label, "Company");
    assert_eq!(
        persisted.get("existing").unwrap().scope,
        first["scope"].as_str().unwrap()
    );
    let cached = f
        .core
        .cached_sources("obsidian", "bbros")
        .unwrap()
        .remove(0);
    let removed = f.mcp("plugins.remove", json!({"id":"bbros"}));
    assert_eq!(removed["cache_preserved"], true);
    assert!(f.core.snapshot(&cached).is_ok());
    fs::write(
        PathBuf::from(second["scope"].as_str().unwrap()).join("Later.md"),
        "# Not collected",
    )
    .unwrap();
    f.sync.poll_once(&AtomicBool::new(false)).unwrap();
    assert_eq!(f.core.cached_sources("obsidian", "bbros").unwrap().len(), 1);
    assert_eq!(
        f.cli_json(&["plugins", "list"]).as_array().unwrap().len(),
        1
    );
    assert_eq!(
        fs::read_to_string(PathBuf::from(second["scope"].as_str().unwrap()).join("A.md")).unwrap(),
        "# bbros\nOriginal content"
    );
}

#[test]
fn invalid_edits_duplicates_credentials_and_unauthenticated_mutations_are_rejected() {
    let f = Fixture::new();
    let input = f.vault("notes");
    f.client.call("plugins.add", input.clone()).unwrap();
    let before = fs::read(f.root.join("connections.json")).unwrap();
    assert!(f.client.call("plugins.add", input.clone()).is_err());
    for changes in [
        json!({"scope":"relative/path"}),
        json!({"label":""}),
        json!({"provider":"github"}),
        json!({"token":"not-a-real-token"}),
        json!({"auth":"token"}),
    ] {
        assert!(f
            .client
            .call("plugins.update", json!({"id":"notes","changes":changes}))
            .is_err());
    }
    let mut credential_input = f.vault("bad");
    credential_input["token"] = json!("not-a-real-token");
    assert!(f.client.call("plugins.add", credential_input).is_err());
    assert!(f
        .client
        .call(
            "plugins.remove",
            json!({"id":"notes","delete_credential":true})
        )
        .is_err());
    assert!(!f
        .cli(&["plugins", "update", "--id", "notes", "--auto-sync", "maybe"])
        .status
        .success());
    let bad = CoreClient::with_token(f.client.endpoint().clone(), "wrong").unwrap();
    assert!(bad.call("plugins.remove", json!({"id":"notes"})).is_err());
    assert_eq!(fs::read(f.root.join("connections.json")).unwrap(), before);
    assert!(f
        .client
        .call("plugins.get", json!({"id":"missing"}))
        .is_err());
    assert!(f
        .client
        .call(
            "plugins.update",
            json!({"id":"missing","changes":{"label":"x"}})
        )
        .is_err());
    assert!(f
        .client
        .call("plugins.remove", json!({"id":"missing"}))
        .is_err());
}

#[test]
fn partial_updates_preserve_other_fields_and_new_scope_is_used_by_next_scan() {
    let f = Fixture::new();
    f.client.call("plugins.add", f.vault("notes")).unwrap();
    f.client
        .call("plugins.refresh", json!({"id":"notes"}))
        .unwrap();
    let next = f.vault("moved");
    let client = f.client.clone();
    let update = std::thread::spawn(move || {
        client
            .call(
                "plugins.update",
                json!({"id":"notes","changes":{"label":"Renamed"}}),
            )
            .unwrap()
    });
    f.cli_json(&[
        "plugins",
        "update",
        "--id",
        "notes",
        "--scope",
        next["scope"].as_str().unwrap(),
        "--auto-sync",
        "true",
    ]);
    update.join().unwrap();
    let saved = f.cli_json(&["plugins", "get", "--id", "notes"]);
    assert_eq!(saved["label"], "Renamed");
    assert_eq!(saved["scope"], next["scope"]);
    f.sync.poll_once(&AtomicBool::new(false)).unwrap();
    let source = f
        .core
        .cached_sources("obsidian", "notes")
        .unwrap()
        .remove(0);
    assert_eq!(f.core.snapshot(&source).unwrap().title, "moved");
    let restored = Arc::new(Mutex::new(
        PluginRegistry::open(f.root.join("connections.json")).unwrap(),
    ));
    let restart = LocalSync::new(f.core.clone(), restored);
    assert_eq!(restart.connection("notes").unwrap().label, "Renamed");
    restart.poll_once(&AtomicBool::new(false)).unwrap();
    assert_eq!(restart.statuses().unwrap()[0].indexed, 1);
}

#[test]
fn unavailable_vault_can_be_renamed_paused_and_removed_without_resolving_its_path() {
    let f = Fixture::new();
    let input = f.vault("offline");
    f.client.call("plugins.add", input.clone()).unwrap();
    fs::rename(input["scope"].as_str().unwrap(), f.root.join("away")).unwrap();
    let edited = f.cli_json(&[
        "plugins",
        "update",
        "--id",
        "offline",
        "--label",
        "Paused",
        "--auto-sync",
        "false",
    ]);
    assert_eq!(edited["auto_sync"], false);
    assert_eq!(edited["scope"], input["scope"]);
    assert!(f
        .client
        .call("plugins.refresh", json!({"id":"offline"}))
        .is_err());
    f.cli_json(&["plugins", "remove", "--id", "offline"]);
    assert!(f.sync.connections().unwrap().is_empty());
}

#[test]
fn atlassian_scopes_roundtrip_through_cli_mcp_and_saved_registry() {
    let f = Fixture::new();
    let legacy = f.client.call("plugins.add", json!({"id":"legacy-jira","provider":"jira","label":"Legacy","account":"me@example.com","scope":"https://example.atlassian.net","project":"TEAM","auth":"token"})).unwrap();
    assert_eq!(legacy["jira_scope"], "project");
    let mine = f.cli_json(&[
        "plugins",
        "add",
        "--id",
        "mine",
        "--provider",
        "jira",
        "--label",
        "My issues",
        "--account",
        "me@example.com",
        "--scope",
        "https://example.atlassian.net",
        "--jira-scope",
        "mine",
        "--jira-include-parents",
        "true",
    ]);
    assert_eq!(mine["jira_scope"], "mine");
    assert_eq!(mine["jira_include_parents"], true);
    let patched = f.mcp(
        "plugins.update",
        json!({"id":"mine","changes":{"jira_include_reporter":true}}),
    );
    assert_eq!(patched["jira_include_parents"], true);
    assert_eq!(patched["jira_include_reporter"], true);
    let pages = f.cli_json(&[
        "plugins",
        "add",
        "--id",
        "pages",
        "--provider",
        "confluence",
        "--label",
        "Pages",
        "--account",
        "me@example.com",
        "--scope",
        "https://example.atlassian.net",
        "--confluence-mode",
        "selected",
        "--confluence-page-ids",
        "123,456",
    ]);
    assert_eq!(pages["confluence_page_ids"], json!(["123", "456"]));
    let updated = f.cli_json(&[
        "plugins",
        "update",
        "--id",
        "pages",
        "--confluence-mode",
        "watched",
    ]);
    assert_eq!(updated["confluence_mode"], "watched");
    assert_eq!(updated["confluence_page_ids"], json!(["123", "456"]));
    let saved = PluginRegistry::open(f.root.join("connections.json")).unwrap();
    assert_eq!(
        serde_json::to_value(saved.get("mine").unwrap()).unwrap()["jira_scope"],
        "mine"
    );
    // Wrong-provider search must fail before credential/network access.
    assert!(f
        .client
        .call(
            "plugins.confluence.search",
            json!({"id":"mine","query":"hello"})
        )
        .is_err());
}
