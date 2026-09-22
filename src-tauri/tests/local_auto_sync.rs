use aidebook_lib::core::{
    local_sync::LocalSync,
    plugins::{AuthMethod, PluginConnection, PluginRegistry, Provider},
    Core, GraphTraversalRequest, ObsidianAdapter, ReadOnlyConnector, VaultConfig,
};
use std::{
    fs,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

struct Fixture {
    root: PathBuf,
    core: Core,
    registry: Arc<Mutex<PluginRegistry>>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("aidebook-auto-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let core = Core::open(root.join("core.sqlite")).unwrap();
        let registry = Arc::new(Mutex::new(
            PluginRegistry::open(root.join("connections.json")).unwrap(),
        ));
        Self {
            root,
            core,
            registry,
        }
    }
    fn vault(&self, id: &str) -> PathBuf {
        let path = self.root.join(id);
        fs::create_dir_all(&path).unwrap();
        self.registry
            .lock()
            .unwrap()
            .add(PluginConnection {
                id: id.into(),
                provider: Provider::Obsidian,
                label: id.into(),
                account: String::new(),
                scope: path.to_str().unwrap().into(),
                project: String::new(),
                jira_scope: Default::default(),
                jira_include_reporter: false,
                jira_include_parents: false,
                confluence_mode: Default::default(),
                confluence_page_ids: vec![],
                auth: AuthMethod::Local,
                auto_sync: true,
            })
            .unwrap();
        path
    }
    fn service(&self) -> LocalSync {
        LocalSync::new(self.core.clone(), self.registry.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn pass(sync: &LocalSync) {
    sync.poll_once(&AtomicBool::new(false)).unwrap();
}

#[test]
fn polling_updates_content_links_deletions_and_restores_without_writing_the_vault() {
    let f = Fixture::new();
    let vault = f.vault("notes");
    fs::write(vault.join("A.md"), "# Alpha\nSee [[B]]").unwrap();
    fs::write(vault.join("B.md"), "# Beta\noriginal").unwrap();
    let before_modified = fs::metadata(vault.join("A.md"))
        .unwrap()
        .modified()
        .unwrap();
    let source = ObsidianAdapter::open(VaultConfig::new(&vault, "notes", "notes"))
        .unwrap()
        .list()
        .unwrap()
        .into_iter()
        .find(|s| s.source.external_id == "A.md")
        .unwrap()
        .source;
    let sync = f.service();
    pass(&sync);
    let graph = || {
        f.core
            .graph_traverse(GraphTraversalRequest {
                source: Some(source.clone()),
                ..Default::default()
            })
            .unwrap()
    };
    assert_eq!(graph().edges.len(), 1);
    assert_eq!(
        fs::metadata(vault.join("A.md"))
            .unwrap()
            .modified()
            .unwrap(),
        before_modified
    );
    assert_eq!(
        fs::read_to_string(vault.join("A.md")).unwrap(),
        "# Alpha\nSee [[B]]"
    );
    let first_graph = sync.statuses().unwrap()[0].last_graph_at.clone();
    pass(&sync);
    assert_eq!(sync.statuses().unwrap()[0].changed, 0);
    assert_eq!(
        sync.statuses().unwrap()[0].last_graph_at,
        first_graph,
        "unchanged polling does not rebuild the graph"
    );

    fs::write(vault.join("A.md"), "# Alpha\nnew content and [[C]]").unwrap();
    fs::write(vault.join("C.md"), "# Gamma").unwrap();
    fs::remove_file(vault.join("B.md")).unwrap();
    pass(&sync);
    assert!(f
        .core
        .snapshot(&source)
        .unwrap()
        .body
        .contains("new content"));
    assert_eq!(sync.statuses().unwrap()[0].removed, 1);
    assert!(graph().nodes.iter().any(|n| n.source.external_id == "C.md"));
    assert!(!graph().nodes.iter().any(|n| n.source.external_id == "B.md"));
    assert_eq!(f.core.cached_sources("obsidian", "notes").unwrap().len(), 2);

    fs::rename(vault.join("C.md"), vault.join("D.md")).unwrap();
    pass(&sync);
    assert!(
        graph().edges.is_empty(),
        "renamed target cannot keep an obsolete link"
    );
    fs::rename(vault.join("D.md"), vault.join("C.md")).unwrap();
    pass(&sync);
    assert_eq!(
        graph().edges.len(),
        1,
        "a restored file becomes searchable and linked again"
    );
}

#[test]
fn restart_reconciles_offline_deletions_and_pause_is_persistent() {
    let f = Fixture::new();
    let vault = f.vault("notes");
    fs::write(vault.join("A.md"), "# Before").unwrap();
    let sync = f.service();
    pass(&sync);
    sync.set_enabled("notes", false).unwrap();
    fs::write(vault.join("A.md"), "# After").unwrap();
    pass(&sync);
    let source = f
        .core
        .cached_sources("obsidian", "notes")
        .unwrap()
        .remove(0);
    assert_eq!(f.core.snapshot(&source).unwrap().title, "Before");
    let restored = Arc::new(Mutex::new(
        PluginRegistry::open(f.root.join("connections.json")).unwrap(),
    ));
    assert!(!restored.lock().unwrap().get("notes").unwrap().auto_sync);
    let restarted = LocalSync::new(f.core.clone(), restored);
    restarted.refresh("notes", false).unwrap();
    assert_eq!(
        f.core.snapshot(&source).unwrap().title,
        "After",
        "manual refresh still works while paused"
    );
    fs::remove_file(vault.join("A.md")).unwrap();
    restarted.set_enabled("notes", true).unwrap();
    pass(&restarted);
    assert!(
        f.core.snapshot(&source).unwrap().is_deleted,
        "restart must compare against persistent cached sources"
    );
    restarted.remove("notes").unwrap();
    fs::write(vault.join("New.md"), "# Must not read").unwrap();
    pass(&restarted);
    assert!(f
        .core
        .cached_sources("obsidian", "notes")
        .unwrap()
        .is_empty());
}

#[test]
fn unavailable_vault_preserves_cache_and_does_not_block_other_connections() {
    let f = Fixture::new();
    let offline = f.vault("offline");
    let healthy = f.vault("healthy");
    fs::write(offline.join("A.md"), "# Keep me").unwrap();
    fs::write(healthy.join("A.md"), "# Old").unwrap();
    let sync = f.service();
    pass(&sync);
    let source = f
        .core
        .cached_sources("obsidian", "offline")
        .unwrap()
        .remove(0);
    fs::rename(&offline, f.root.join("temporarily-away")).unwrap();
    fs::write(healthy.join("A.md"), "# Updated").unwrap();
    pass(&sync);
    assert!(!f.core.snapshot(&source).unwrap().is_deleted);
    assert_eq!(f.core.snapshot(&source).unwrap().title, "Keep me");
    let healthy_source = f
        .core
        .cached_sources("obsidian", "healthy")
        .unwrap()
        .remove(0);
    assert_eq!(f.core.snapshot(&healthy_source).unwrap().title, "Updated");
    assert!(sync
        .statuses()
        .unwrap()
        .iter()
        .find(|s| s.id == "offline")
        .unwrap()
        .error
        .is_some());
    fs::rename(f.root.join("temporarily-away"), &offline).unwrap();
    pass(&sync);
    assert!(sync
        .statuses()
        .unwrap()
        .iter()
        .find(|s| s.id == "offline")
        .unwrap()
        .error
        .is_none());
    fs::write(healthy.join("A.md"), "# Stopped").unwrap();
    sync.poll_once(&AtomicBool::new(true)).unwrap();
    assert_eq!(f.core.snapshot(&healthy_source).unwrap().title, "Updated");
}
