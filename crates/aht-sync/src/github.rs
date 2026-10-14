//! GitHub backend using the REST Git Data API so a whole sync is a single commit.
//!
//! Endpoints used (all under `https://api.github.com`, configurable for tests):
//! - `GET  /repos/{o}/{r}/git/ref/heads/{branch}`      current head
//! - `GET  /repos/{o}/{r}/git/commits/{sha}`            head commit -> tree sha
//! - `GET  /repos/{o}/{r}/git/trees/{sha}?recursive=1`  list (etag = blob sha)
//! - `GET  /repos/{o}/{r}/git/blobs/{sha}`              get (base64)
//! - `POST /repos/{o}/{r}/git/blobs`                    create blobs
//! - `POST /repos/{o}/{r}/git/trees`                    tree with `base_tree`
//! - `POST /repos/{o}/{r}/git/commits`                  one commit per sync
//! - `PATCH /repos/{o}/{r}/git/refs/heads/{branch}`     `force: false`; 422 => `Conflict`
//! - `PUT  /repos/{o}/{r}/contents/{path}`              first file of an empty repo (the Git
//!   Data API refuses empty repositories)
//! - `GET  /repos/{o}/{r}`, `POST /user/repos`          `ensure_repo` / `create_private_repo_at`
//!
//! The branch must already exist unless the repository is completely empty.

use crate::backend::{Change, RemoteEntry, SyncBackend};
use crate::error::{Result, SyncError};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

pub const DEFAULT_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = "Handrail";

pub struct GitHubBackend {
    client: reqwest::Client,
    base: String,
    owner: String,
    repo: String,
    branch: String,
    token: String,
    /// path -> blob sha from the last tree listing (used by `get`).
    blobs: Mutex<HashMap<String, String>>,
}

impl fmt::Debug for GitHubBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubBackend")
            .field("owner", &self.owner)
            .field("repo", &self.repo)
            .field("branch", &self.branch)
            .field("token", &"<redacted>")
            .finish()
    }
}

struct Head {
    commit: String,
    tree: String,
}

fn new_client() -> reqwest::Client {
    // Sync runs while the vault is locked: a stalled request must not hang the app.
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("reqwest client")
}

fn authed(client: &reqwest::Client, token: &str, method: Method, url: String) -> RequestBuilder {
    client
        .request(method, url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", USER_AGENT)
}

fn header_u64(r: &Response, name: &str) -> Option<u64> {
    r.headers().get(name)?.to_str().ok()?.trim().parse().ok()
}

/// Turn a rate-limit response into `RateLimited`, a 401 into `Auth`; pass everything else on.
async fn check(resp: Response) -> Result<(StatusCode, Value)> {
    let status = resp.status();
    let remaining = header_u64(&resp, "x-ratelimit-remaining");
    let retry_after = header_u64(&resp, "retry-after");
    let reset = header_u64(&resp, "x-ratelimit-reset");
    let text = resp.text().await?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let msg = body["message"].as_str().unwrap_or("").to_lowercase();
    let limited = status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && (remaining == Some(0) || retry_after.is_some() || msg.contains("rate limit")));
    if limited {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let secs = retry_after
            .or_else(|| reset.map(|r| r.saturating_sub(now)))
            .unwrap_or(60);
        return Err(SyncError::RateLimited {
            retry_after_secs: secs,
        });
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(SyncError::Auth("GitHub rejected the token".into()));
    }
    Ok((status, body))
}

fn http_err(status: StatusCode, body: &Value) -> SyncError {
    SyncError::Http(format!(
        "{status}: {}",
        body["message"].as_str().unwrap_or("request failed")
    ))
}

fn str_field<'a>(v: &'a Value, path: &[&str]) -> Result<&'a str> {
    let mut cur = v;
    for p in path {
        cur = &cur[*p];
    }
    cur.as_str()
        .ok_or_else(|| SyncError::Protocol(format!("missing field {}", path.join("."))))
}

/// `POST /user/repos` with `private: true` against `base` (for tests).
pub async fn create_private_repo_at(base: &str, token: &str, name: &str) -> Result<()> {
    let client = new_client();
    let url = format!("{}/user/repos", base.trim_end_matches('/'));
    let resp = authed(&client, token, Method::POST, url)
        .json(&json!({ "name": name, "private": true, "auto_init": false }))
        .send()
        .await?;
    let (status, body) = check(resp).await?;
    if status.is_success() {
        Ok(())
    } else {
        Err(http_err(status, &body))
    }
}

impl GitHubBackend {
    pub fn new(
        owner: impl Into<String>,
        repo: impl Into<String>,
        branch: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            client: new_client(),
            base: DEFAULT_API_BASE.into(),
            owner: owner.into(),
            repo: repo.into(),
            branch: branch.into(),
            token: token.into(),
            blobs: Mutex::new(HashMap::new()),
        }
    }

    /// Point at a different API root (GitHub Enterprise, or a mock server in tests).
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_string();
        self
    }

    fn repo_url(&self, rest: &str) -> String {
        format!("{}/repos/{}/{}{rest}", self.base, self.owner, self.repo)
    }

    async fn call(
        &self,
        method: Method,
        rest: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        let mut rb = authed(&self.client, &self.token, method, self.repo_url(rest));
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        check(rb.send().await?).await
    }

    /// The repository's current `(owner, name)`. GitHub redirects reads of a renamed repository
    /// to its new name, but a redirected write is replayed as a GET by the HTTP client and fails,
    /// so callers store the new name. `None` when it does not exist or the token cannot see it.
    pub async fn current_name(&self) -> Result<Option<(String, String)>> {
        let (status, body) = self.call(Method::GET, "", None).await?;
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        let owner = str_field(&body, &["owner", "login"])?.to_string();
        let name = str_field(&body, &["name"])?.to_string();
        Ok(Some((owner, name)))
    }

    /// Create `{owner}/{repo}` as a private repo if it does not exist. Returns whether it was
    /// created. (Assumes the owner is the authenticated user.)
    pub async fn ensure_repo(&self) -> Result<bool> {
        let (status, body) = self.call(Method::GET, "", None).await?;
        if status.is_success() {
            return Ok(false);
        }
        if status != StatusCode::NOT_FOUND {
            return Err(http_err(status, &body));
        }
        create_private_repo_at(&self.base, &self.token, &self.repo).await?;
        Ok(true)
    }

    /// `None` when the repository has no commits / the branch does not exist yet.
    async fn head(&self) -> Result<Option<Head>> {
        let (status, body) = self
            .call(
                Method::GET,
                &format!("/git/ref/heads/{}", self.branch),
                None,
            )
            .await?;
        if status == StatusCode::NOT_FOUND || status == StatusCode::CONFLICT {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        let commit = str_field(&body, &["object", "sha"])?.to_string();
        let (status, body) = self
            .call(Method::GET, &format!("/git/commits/{commit}"), None)
            .await?;
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        let tree = str_field(&body, &["tree", "sha"])?.to_string();
        Ok(Some(Head { commit, tree }))
    }

    async fn entries(&self, head: &Head) -> Result<Vec<RemoteEntry>> {
        let (status, body) = self
            .call(
                Method::GET,
                &format!("/git/trees/{}?recursive=1", head.tree),
                None,
            )
            .await?;
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        if body["truncated"].as_bool() == Some(true) {
            return Err(SyncError::Protocol("remote tree listing truncated".into()));
        }
        let mut out = Vec::new();
        for t in body["tree"].as_array().into_iter().flatten() {
            if t["type"].as_str() == Some("blob") {
                out.push(RemoteEntry {
                    path: str_field(t, &["path"])?.to_string(),
                    etag: str_field(t, &["sha"])?.to_string(),
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        *self.blobs.lock().unwrap() = out
            .iter()
            .map(|e| (e.path.clone(), e.etag.clone()))
            .collect();
        Ok(out)
    }

    /// Apply several writes as ONE commit (create blobs, tree with `base_tree`, commit, fast-forward
    /// ref update). Any etag mismatch or a non-fast-forward ref update yields
    /// `SyncError::Conflict`; the caller should re-list and retry the sync round
    /// ([`crate::sync_once`] does this).
    pub async fn commit_changes(&self, changes: Vec<Change>) -> Result<Vec<RemoteEntry>> {
        let ops = changes
            .into_iter()
            .map(|c| (c.path, Some(c.bytes), c.expected));
        self.commit(ops.collect()).await
    }

    async fn commit(
        &self,
        mut ops: Vec<(String, Option<Vec<u8>>, Option<String>)>,
    ) -> Result<Vec<RemoteEntry>> {
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        let Some(head) = self.head().await? else {
            return self.first_commit(ops).await;
        };
        let current: HashMap<String, String> = self
            .entries(&head)
            .await?
            .into_iter()
            .map(|e| (e.path, e.etag))
            .collect();
        for (path, _, expected) in &ops {
            if current.get(path) != expected.as_ref() {
                return Err(SyncError::Conflict(path.clone()));
            }
        }
        ops.sort_by(|a, b| a.0.cmp(&b.0));
        let mut tree = Vec::new();
        let mut written = Vec::new();
        for (path, bytes, _) in &ops {
            match bytes {
                Some(bytes) => {
                    let (status, body) = self
                        .call(
                            Method::POST,
                            "/git/blobs",
                            Some(json!({ "content": B64.encode(bytes), "encoding": "base64" })),
                        )
                        .await?;
                    if !status.is_success() {
                        return Err(http_err(status, &body));
                    }
                    let sha = str_field(&body, &["sha"])?.to_string();
                    tree.push(
                        json!({ "path": path, "mode": "100644", "type": "blob", "sha": sha }),
                    );
                    written.push(RemoteEntry {
                        path: path.clone(),
                        etag: sha,
                    });
                }
                None => tree.push(
                    json!({ "path": path, "mode": "100644", "type": "blob", "sha": Value::Null }),
                ),
            }
        }
        let (status, body) = self
            .call(
                Method::POST,
                "/git/trees",
                Some(json!({ "base_tree": head.tree, "tree": tree })),
            )
            .await?;
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        let new_tree = str_field(&body, &["sha"])?.to_string();
        let (status, body) = self
            .call(
                Method::POST,
                "/git/commits",
                Some(json!({
                    "message": format!("Handrail sync ({} file(s))", ops.len()),
                    "tree": new_tree,
                    "parents": [head.commit],
                })),
            )
            .await?;
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        let commit = str_field(&body, &["sha"])?.to_string();
        let (status, body) = self
            .call(
                Method::PATCH,
                &format!("/git/refs/heads/{}", self.branch),
                Some(json!({ "sha": commit, "force": false })),
            )
            .await?;
        if status == StatusCode::UNPROCESSABLE_ENTITY {
            return Err(SyncError::Conflict("non-fast-forward ref update".into()));
        }
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        let mut cache = self.blobs.lock().unwrap();
        for (path, bytes, _) in &ops {
            if bytes.is_none() {
                cache.remove(path);
            }
        }
        for w in &written {
            cache.insert(w.path.clone(), w.etag.clone());
        }
        Ok(written)
    }

    /// Empty repository: the Git Data API returns 409, so the first file goes through the
    /// Contents API (which creates the initial commit); the rest is a normal commit.
    async fn first_commit(
        &self,
        mut ops: Vec<(String, Option<Vec<u8>>, Option<String>)>,
    ) -> Result<Vec<RemoteEntry>> {
        ops.retain(|(_, bytes, _)| bytes.is_some());
        if ops.iter().any(|(_, _, expected)| expected.is_some()) {
            return Err(SyncError::Conflict("remote repository is empty".into()));
        }
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        let (path, bytes, _) = ops.remove(0);
        let bytes = bytes.unwrap_or_default();
        let (status, body) = self
            .call(
                Method::PUT,
                &format!("/contents/{path}"),
                Some(json!({
                    "message": "Handrail sync (initial)",
                    "content": B64.encode(&bytes),
                    "branch": self.branch,
                })),
            )
            .await?;
        if status == StatusCode::UNPROCESSABLE_ENTITY {
            return Err(SyncError::Conflict(path)); // someone created it first
        }
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        let mut out = vec![RemoteEntry {
            etag: str_field(&body, &["content", "sha"])?.to_string(),
            path,
        }];
        if !ops.is_empty() {
            out.extend(Box::pin(self.commit(ops)).await?);
        }
        Ok(out)
    }
}

#[async_trait]
impl SyncBackend for GitHubBackend {
    async fn list(&self) -> Result<Vec<RemoteEntry>> {
        match self.head().await? {
            Some(head) => self.entries(&head).await,
            None => {
                self.blobs.lock().unwrap().clear();
                Ok(Vec::new())
            }
        }
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>> {
        let cached = self.blobs.lock().unwrap().get(path).cloned();
        let sha = match cached {
            Some(s) => s,
            None => {
                self.list().await?;
                self.blobs
                    .lock()
                    .unwrap()
                    .get(path)
                    .cloned()
                    .ok_or_else(|| SyncError::NotFound(path.to_string()))?
            }
        };
        let (status, body) = self
            .call(Method::GET, &format!("/git/blobs/{sha}"), None)
            .await?;
        if status == StatusCode::NOT_FOUND {
            return Err(SyncError::NotFound(path.to_string()));
        }
        if !status.is_success() {
            return Err(http_err(status, &body));
        }
        let b64: String = str_field(&body, &["content"])?
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        B64.decode(b64)
            .map_err(|e| SyncError::Protocol(format!("bad base64 in blob: {e}")))
    }

    async fn put(&self, path: &str, bytes: &[u8], expected: Option<&str>) -> Result<String> {
        let mut out = self
            .commit_changes(vec![Change {
                path: path.to_string(),
                bytes: bytes.to_vec(),
                expected: expected.map(str::to_string),
            }])
            .await?;
        out.pop()
            .map(|e| e.etag)
            .ok_or_else(|| SyncError::Protocol("no entry written".into()))
    }

    async fn delete(&self, path: &str, expected: Option<&str>) -> Result<()> {
        self.commit(vec![(path.to_string(), None, expected.map(str::to_string))])
            .await
            .map(|_| ())
    }

    async fn put_batch(&self, changes: Vec<Change>) -> Result<Vec<RemoteEntry>> {
        self.commit_changes(changes).await
    }
}
