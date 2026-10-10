use aht_sync::{sync_once, Change, GitHubBackend, SyncBackend, SyncError, SyncState};
use aht_vault::{KdfParams, Setting, Vault};
use base64::Engine;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REF: &str = "/repos/me/vault/git/ref/heads/main";

fn backend(s: &MockServer) -> GitHubBackend {
    GitHubBackend::new("me", "vault", "main", "tok").with_base_url(s.uri())
}

async fn mount_head(s: &MockServer, entries: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(REF))
        .and(header("user-agent", "Handrail"))
        .and(header("authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object": {"sha": "c0"}})))
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/me/vault/git/commits/c0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tree": {"sha": "t0"}})))
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/me/vault/git/trees/t0"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"truncated": false, "tree": entries})),
        )
        .mount(s)
        .await;
}

fn blob(p: &str, sha: &str) -> serde_json::Value {
    json!({"path": p, "type": "blob", "sha": sha, "mode": "100644"})
}

/// Mounts the four write endpoints, each expected `n` times.
async fn mount_writes(s: &MockServer, blobs: u64, ref_status: u16) {
    Mock::given(method("POST"))
        .and(path("/repos/me/vault/git/blobs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "newblob"})))
        .expect(blobs)
        .mount(s)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/me/vault/git/trees"))
        .and(body_partial_json(json!({"base_tree": "t0"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "t1"})))
        .expect(1)
        .mount(s)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/me/vault/git/commits"))
        .and(body_partial_json(json!({"tree": "t1", "parents": ["c0"]})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "c1"})))
        .expect(1)
        .mount(s)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/repos/me/vault/git/refs/heads/main"))
        .and(body_partial_json(json!({"sha": "c1", "force": false})))
        .respond_with(ResponseTemplate::new(ref_status).set_body_json(json!({"message": "x"})))
        .expect(1)
        .mount(s)
        .await;
}

#[tokio::test]
async fn list_and_get() {
    let s = MockServer::start().await;
    mount_head(
        &s,
        json!([blob("manifest.json", "m1"), blob("items/a.enc", "a1"),
               {"path": "items", "type": "tree", "sha": "x", "mode": "040000"}]),
    )
    .await;
    let content = base64::engine::general_purpose::STANDARD.encode(b"secret bytes");
    let (l, r) = content.split_at(4);
    Mock::given(method("GET"))
        .and(path("/repos/me/vault/git/blobs/a1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"content": format!("{l}\n{r}\n"), "encoding": "base64"})),
        )
        .mount(&s)
        .await;
    let b = backend(&s);
    let list = b.list().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].path, "items/a.enc");
    assert_eq!(list[0].etag, "a1");
    assert_eq!(b.get("items/a.enc").await.unwrap(), b"secret bytes");
    assert!(matches!(b.get("nope").await, Err(SyncError::NotFound(_))));
}

#[tokio::test]
async fn batch_is_one_commit() {
    let s = MockServer::start().await;
    mount_head(&s, json!([blob("items/old.enc", "o1")])).await;
    mount_writes(&s, 2, 200).await;
    let b = backend(&s);
    let out = b
        .commit_changes(vec![
            Change {
                path: "items/old.enc".into(),
                bytes: b"v2".to_vec(),
                expected: Some("o1".into()),
            },
            Change {
                path: "items/new.enc".into(),
                bytes: b"v1".to_vec(),
                expected: None,
            },
        ])
        .await
        .unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].etag, "newblob");
}

#[tokio::test]
async fn etag_mismatch_is_conflict_before_any_write() {
    let s = MockServer::start().await;
    mount_head(&s, json!([blob("items/a.enc", "a1")])).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&s)
        .await;
    let err = backend(&s)
        .put("items/a.enc", b"x", Some("stale"))
        .await
        .unwrap_err();
    assert!(matches!(err, SyncError::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn non_fast_forward_is_conflict() {
    let s = MockServer::start().await;
    mount_head(&s, json!([])).await;
    mount_writes(&s, 1, 422).await;
    let err = backend(&s)
        .put("items/a.enc", b"x", None)
        .await
        .unwrap_err();
    assert!(matches!(err, SyncError::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn empty_repo_first_commit_uses_contents_api() {
    let s = MockServer::start().await;
    // Empty repo: no ref. After the first PUT /contents the ref exists.
    Mock::given(method("GET"))
        .and(path(REF))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(json!({"message": "Git Repository is empty."})),
        )
        .up_to_n_times(2)
        .mount(&s)
        .await;
    Mock::given(method("PUT"))
        .and(path("/repos/me/vault/contents/items/a.enc"))
        .and(body_partial_json(json!({"branch": "main"})))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({"content": {"sha": "first"}})),
        )
        .expect(1)
        .mount(&s)
        .await;
    let b = backend(&s);
    assert!(b.list().await.unwrap().is_empty());
    // second ref lookup (inside commit) is still empty, then it exists
    mount_head(&s, json!([blob("items/a.enc", "first")])).await;
    mount_writes(&s, 1, 200).await;
    let out = b
        .commit_changes(vec![
            Change {
                path: "items/a.enc".into(),
                bytes: b"1".to_vec(),
                expected: None,
            },
            Change {
                path: "items/b.enc".into(),
                bytes: b"2".to_vec(),
                expected: None,
            },
        ])
        .await
        .unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].etag, "first");
}

#[tokio::test]
async fn rate_limit_reports_retry_after() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(REF))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "17"))
        .mount(&s)
        .await;
    assert!(matches!(
        backend(&s).list().await,
        Err(SyncError::RateLimited {
            retry_after_secs: 17
        })
    ));

    let s = MockServer::start().await;
    let reset = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 120;
    Mock::given(method("GET"))
        .and(path(REF))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", reset.to_string())
                .set_body_json(json!({"message": "API rate limit exceeded"})),
        )
        .mount(&s)
        .await;
    match backend(&s).list().await {
        Err(SyncError::RateLimited { retry_after_secs }) => {
            assert!(
                (100..=120).contains(&retry_after_secs),
                "{retry_after_secs}"
            )
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn bad_token_is_auth_error() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"message": "Bad credentials"})),
        )
        .mount(&s)
        .await;
    assert!(matches!(backend(&s).list().await, Err(SyncError::Auth(_))));
}

#[tokio::test]
async fn ensure_repo_creates_private_repo_when_missing() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/me/vault"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/user/repos"))
        .and(body_partial_json(json!({"name": "vault", "private": true})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"name": "vault"})))
        .expect(1)
        .mount(&s)
        .await;
    assert!(backend(&s).ensure_repo().await.unwrap());

    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/me/vault"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"private": true})))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&s)
        .await;
    assert!(!backend(&s).ensure_repo().await.unwrap());
}

/// A vault already in sync with the remote makes zero write calls.
#[tokio::test]
async fn up_to_date_sync_makes_no_writes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut vault, _sk) = Vault::create(dir.path(), "pw", KdfParams::insecure_fast()).unwrap();
    vault
        .put(&Setting {
            key: "k".into(),
            value: json!(1),
        })
        .unwrap();
    let mut state = SyncState::default();
    let mut entries = Vec::new();
    for (i, f) in vault.raw_files().unwrap().iter().enumerate() {
        let etag = format!("sha{i}");
        state.remote_etags.insert(f.path.clone(), etag.clone());
        state.pushed_sha.insert(f.path.clone(), f.sha256.clone());
        entries.push(blob(&f.path, &etag));
    }
    let s = MockServer::start().await;
    mount_head(&s, json!(entries)).await;
    // The manifest is already known; any blob GET or write would be a bug.
    for m in ["POST", "PATCH", "PUT", "DELETE"] {
        Mock::given(method(m))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&s)
            .await;
    }
    Mock::given(method("GET"))
        .and(path_regex_blobs())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&s)
        .await;
    let b = backend(&s);
    let r1 = sync_once(&mut vault, &b, &mut state).await.unwrap();
    let r2 = sync_once(&mut vault, &b, &mut state).await.unwrap();
    assert_eq!((r1.pulled, r1.pushed), (0, 0));
    assert_eq!((r2.pulled, r2.pushed), (0, 0));
}

fn path_regex_blobs() -> wiremock::matchers::PathRegexMatcher {
    wiremock::matchers::path_regex(r"^/repos/me/vault/git/blobs/.*")
}

#[tokio::test]
async fn repo_exists_is_true_false_or_an_error() {
    for (status, want) in [(200, Some(true)), (404, Some(false)), (401, None)] {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/me/vault"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({})))
            .mount(&s)
            .await;
        assert_eq!(backend(&s).repo_exists().await.ok(), want, "{status}");
    }
}
