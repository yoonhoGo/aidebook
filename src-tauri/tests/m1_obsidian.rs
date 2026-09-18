use aidebook_lib::core::{
    AccessStatus, Core, CoreError, ObsidianAdapter, SearchRequest, VaultConfig,
};
use std::fs;

#[test]
fn selected_vault_is_read_only_scoped_and_i_cloud_states_are_visible() {
    let root = std::env::temp_dir().join(format!("aidebook-m1-{}", std::process::id()));
    let outside = root.with_extension("outside");
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(root.join("Projects")).expect("vault");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(
        root.join("Projects/Release.md"),
        "---\ntitle: Release plan\ntags: [release]\nprivate: ignored\n---\n# Release plan\nSee [[Projects/Checklist]] and https://example.com/release.",
    )
    .expect("release");
    fs::write(root.join("Pending.icloud.md"), "").expect("placeholder");
    fs::write(root.join("note conflicted copy.md"), "").expect("conflict");
    let outside_file = outside.join("Outside.md");
    fs::write(&outside_file, "# outside").expect("outside note");

    let adapter = ObsidianAdapter::open(VaultConfig::new(&root, "fixture-vault", "m1-fixture"))
        .expect("selected vault");
    let scan = adapter.scan().expect("scan");
    assert_eq!(scan.snapshots.len(), 3);
    assert_eq!(scan.inaccessible, 2);
    let release = scan
        .snapshots
        .iter()
        .find(|snapshot| snapshot.source.external_id == "Projects/Release.md")
        .expect("release snapshot");
    assert_eq!(release.title, "Release plan");
    assert_eq!(release.metadata.get("private"), None);
    assert_eq!(release.links.len(), 2);
    assert!(scan
        .snapshots
        .iter()
        .filter(|snapshot| !snapshot.access_status.is_searchable())
        .all(|snapshot| snapshot.access_status == AccessStatus::Unavailable));

    let error = adapter
        .read_path(&outside_file)
        .expect_err("outside path must fail");
    assert!(matches!(error, CoreError::PermissionDenied { .. }));

    let core = Core::in_memory().expect("core");
    let ingested = core.ingest_connector(&adapter).expect("ingest vault");
    assert_eq!(ingested.len(), 3);
    let search = core
        .search(SearchRequest {
            query: "release".to_string(),
            provider: Some("obsidian".to_string()),
            kind: Some("note".to_string()),
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: None,
            limit: Some(20),
        })
        .expect("vault search");
    assert_eq!(search.results.len(), 1);
    assert_eq!(search.results[0].source.external_id, "Projects/Release.md");

    fs::remove_dir_all(&root).expect("cleanup root");
    fs::remove_dir_all(&outside).expect("cleanup outside");
}
