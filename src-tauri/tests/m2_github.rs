use aidebook_lib::core::{
    Core, CoreError, CredentialStore, FixtureGitHubApi, GitHubAdapter, GitHubApiError,
    GitHubComment, GitHubConfig, GitHubIssue, GitHubPage, MemoryCredentialStore, ReadOnlyConnector,
    SearchRequest,
};

fn issue(number: u64) -> GitHubIssue {
    GitHubIssue {
        number,
        title: format!("Release issue {number}"),
        body: Some("body from GitHub".to_string()),
        kind: if number == 2 {
            "pull_request".to_string()
        } else {
            "issue".to_string()
        },
        state: "open".to_string(),
        author: Some("fixture-user".to_string()),
        updated_at: Some("2026-09-19T00:00:00Z".to_string()),
        html_url: format!("https://github.com/acme/book/issues/{number}"),
        comments: vec![GitHubComment {
            author: "reviewer".to_string(),
            body: "comment body".to_string(),
            created_at: None,
            updated_at: None,
        }],
        labels: vec!["release".to_string()],
    }
}

fn adapter(
    api: FixtureGitHubApi,
    credentials: &MemoryCredentialStore,
) -> GitHubAdapter<FixtureGitHubApi, MemoryCredentialStore> {
    GitHubAdapter::new(
        GitHubConfig::new("fixture-account", "fixture-connection", "acme", "book"),
        api,
        credentials.clone(),
    )
    .expect("valid GitHub fixture config")
}

#[test]
fn selected_repository_refreshes_pages_and_preserves_last_success_on_failure() {
    let credentials = MemoryCredentialStore::new();
    credentials
        .set("fixture-connection", "fixture-token")
        .expect("fixture credential");
    let first_api = FixtureGitHubApi::new([
        GitHubPage {
            items: vec![issue(1)],
            next_page: Some(2),
        },
        GitHubPage {
            items: vec![issue(2)],
            next_page: None,
        },
    ]);
    let core = Core::in_memory().expect("core");
    let first = adapter(first_api, &credentials);
    let refresh = core.sources_refresh(&first).expect("first refresh");
    assert_eq!(refresh.attempted, 2);
    assert_eq!(refresh.indexed, 2);
    assert_eq!(refresh.sync_state.status, "succeeded");
    let fetched_at = core
        .search(SearchRequest {
            query: "release".to_string(),
            provider: Some("github".to_string()),
            kind: None,
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: None,
            limit: Some(20),
        })
        .expect("search")
        .results[0]
        .fetched_at
        .clone();

    let failing = adapter(
        FixtureGitHubApi::new([GitHubPage {
            items: vec![],
            next_page: None,
        }])
        .with_error(
            1,
            GitHubApiError::Forbidden {
                message: "repository access revoked".to_string(),
            },
        ),
        &credentials,
    );
    assert!(matches!(
        core.sources_refresh(&failing),
        Err(CoreError::PermissionDenied { provider, .. }) if provider == "github"
    ));
    let state = core
        .connections_status("fixture-connection")
        .expect("sync state");
    assert_eq!(state.status, "failed");
    assert!(state.last_success_at.is_some());
    assert_eq!(state.last_error_code.as_deref(), Some("permission_denied"));
    let after = core
        .search(SearchRequest {
            query: "release".to_string(),
            provider: Some("github".to_string()),
            kind: None,
            source_updated_after: None,
            source_updated_before: None,
            max_age_seconds: None,
            limit: Some(20),
        })
        .expect("search after failed refresh");
    assert_eq!(after.results.len(), 2);
    assert!(after
        .results
        .iter()
        .all(|result| result.fetched_at == fetched_at));
}

#[test]
fn credential_and_scope_boundaries_are_explicit() {
    let credentials = MemoryCredentialStore::new();
    let adapter = adapter(
        FixtureGitHubApi::new([GitHubPage {
            items: vec![issue(1)],
            next_page: None,
        }]),
        &credentials,
    );
    assert!(matches!(
        adapter.list(),
        Err(CoreError::Provider { code, .. }) if code == "credential_missing"
    ));
    credentials
        .set("fixture-connection", "fixture-token")
        .expect("fixture credential");
    assert!(matches!(
        adapter.connect("acme/other"),
        Err(CoreError::PermissionDenied { provider, .. }) if provider == "github"
    ));
    assert!(adapter.connect("acme/book").is_ok());
    assert!(adapter
        .fetch(&aidebook_lib::core::SourceRef::new(
            "github",
            "other-account",
            "acme/book#1",
            "https://github.com/acme/book/issues/1",
            "issue",
        ))
        .is_err());
    credentials
        .delete("fixture-connection")
        .expect("delete credential");
    assert!(matches!(
        adapter.list(),
        Err(CoreError::Provider { code, .. }) if code == "credential_missing"
    ));
}
