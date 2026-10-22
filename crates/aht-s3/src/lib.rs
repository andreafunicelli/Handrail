//! A small S3 client for the file browser: list buckets and folders, download, upload, copy and
//! delete objects. Requests are signed with SigV4 and work with AWS and the S3-compatible stores
//! (Cloudflare R2, Backblaze B2, MinIO, Wasabi...).

mod sign;
mod xml;

use aht_core::{Error, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH};
use reqwest::{Body, Method, Response, Url};
use sign::{EMPTY_SHA256, UNSIGNED_PAYLOAD};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Bytes read per upload chunk.
const CHUNK: usize = 256 * 1024;
/// Largest single PUT S3 accepts (multipart uploads are not supported).
const MAX_PUT: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct S3Config {
    /// Service URL such as `https://<account>.r2.cloudflarestorage.com`; empty for AWS.
    pub endpoint: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// `endpoint/bucket/key` instead of `bucket.endpoint/key` (MinIO and most self-hosted stores).
    pub path_style: bool,
}

impl std::fmt::Debug for S3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .field("path_style", &self.path_style)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BucketInfo {
    pub name: String,
    pub created: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    /// Name below the listed prefix.
    pub name: String,
    pub key: String,
    pub size: u64,
    pub modified: Option<u32>,
}

/// One level of a bucket: the "folders" (common prefixes, without the trailing `/`) and objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub folders: Vec<String>,
    pub objects: Vec<ObjectInfo>,
}

#[derive(Debug)]
pub struct S3Client {
    cfg: S3Config,
    http: reqwest::Client,
    /// Bucket regions found on AWS, where a bucket only answers in its own region.
    regions: Mutex<HashMap<String, String>>,
}

fn net(e: impl std::fmt::Display) -> Error {
    Error::Network(e.to_string())
}

fn io_err(e: std::io::Error) -> Error {
    Error::Other(format!("S3 transfer: {e}"))
}

/// The error an S3 failure response stands for.
fn s3_error(status: u16, body: &str) -> Error {
    let code = xml::first(body, "Code").unwrap_or_default();
    let message = xml::first(body, "Message").unwrap_or_else(|| format!("HTTP {status}"));
    let text = if code.is_empty() {
        message
    } else {
        format!("{code}: {message}")
    };
    match (status, code.as_str()) {
        (_, "NoSuchKey" | "NoSuchBucket") | (404, _) => Error::NotFound(text),
        (_, "AccessDenied" | "InvalidAccessKeyId" | "SignatureDoesNotMatch") | (401 | 403, _) => {
            Error::Denied(text)
        }
        _ => Error::Other(format!("S3 {text}")),
    }
}

async fn check(resp: Response) -> Result<Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    Err(s3_error(status, &body))
}

/// What one request addresses.
struct Target<'a> {
    bucket: Option<&'a str>,
    key: &'a str,
    query: &'a [(&'a str, &'a str)],
}

impl S3Client {
    pub fn new(cfg: S3Config) -> Result<Self> {
        if cfg.access_key_id.trim().is_empty() || cfg.secret_access_key.is_empty() {
            return Err(Error::Invalid("S3 needs an access key and a secret".into()));
        }
        let http = reqwest::Client::builder()
            // A redirect would need a new signature; S3 says where to go in the error instead.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(net)?;
        Ok(Self {
            cfg,
            http,
            regions: Mutex::new(HashMap::new()),
        })
    }

    pub fn config(&self) -> &S3Config {
        &self.cfg
    }

    fn is_aws(&self) -> bool {
        self.cfg.endpoint.trim().is_empty()
    }

    fn region_default(&self) -> &str {
        match self.cfg.region.trim() {
            "" => "us-east-1",
            r => r,
        }
    }

    fn endpoint(&self, region: &str) -> Result<Url> {
        let raw = if self.is_aws() {
            format!("https://s3.{region}.amazonaws.com")
        } else {
            let e = self.cfg.endpoint.trim().trim_end_matches('/');
            if e.contains("://") {
                e.to_string()
            } else {
                format!("https://{e}")
            }
        };
        let url =
            Url::parse(&raw).map_err(|e| Error::Invalid(format!("S3 endpoint {raw}: {e}")))?;
        if url.host_str().is_none() {
            return Err(Error::Invalid(format!("S3 endpoint {raw} has no host")));
        }
        Ok(url)
    }

    /// Path-style addressing for `bucket`: always on request, and for names with dots (which
    /// break the TLS wildcard of virtual hosts).
    fn path_style(&self, bucket: &str) -> bool {
        (self.cfg.path_style && !self.is_aws()) || bucket.contains('.')
    }

    /// The region `bucket` lives in. On AWS a HEAD on the bucket names it in a header, whatever
    /// the outcome; elsewhere it is the configured region.
    async fn region(&self, bucket: &str) -> String {
        let configured = self.region_default().to_string();
        if !self.is_aws() {
            return configured;
        }
        let cached = self.regions.lock().expect("regions").get(bucket).cloned();
        if let Some(r) = cached {
            return r;
        }
        let target = Target {
            bucket: Some(bucket),
            key: "",
            query: &[],
        };
        let found = self
            .send_in(
                &configured,
                Method::HEAD,
                &target,
                &[],
                None,
                Body::from(""),
                EMPTY_SHA256,
            )
            .await
            .ok()
            .and_then(|r| {
                r.headers()
                    .get("x-amz-bucket-region")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            });
        let region = found.unwrap_or(configured);
        self.regions
            .lock()
            .expect("regions")
            .insert(bucket.to_string(), region.clone());
        region
    }

    /// Sign and send one request; the response may be an error status.
    #[allow(clippy::too_many_arguments)]
    async fn send_in(
        &self,
        region: &str,
        method: Method,
        t: &Target<'_>,
        extra: &[(&str, String)],
        content_length: Option<u64>,
        body: Body,
        payload_hash: &str,
    ) -> Result<Response> {
        let ep = self.endpoint(region)?;
        let mut host = ep.host_str().unwrap_or_default().to_string();
        let virtual_bucket = t.bucket.filter(|b| !self.path_style(b));
        if let Some(b) = virtual_bucket {
            host = format!("{b}.{host}");
        }
        if let Some(port) = ep.port() {
            host = format!("{host}:{port}");
        }
        let key = sign::uri_encode(t.key, true);
        let path = match (t.bucket, virtual_bucket) {
            (Some(b), None) if t.key.is_empty() => format!("/{}", sign::uri_encode(b, false)),
            (Some(b), None) => format!("/{}/{key}", sign::uri_encode(b, false)),
            _ => format!("/{key}"),
        };
        let query = sign::canonical_query(t.query);
        let url = if query.is_empty() {
            format!("{}://{host}{path}", ep.scheme())
        } else {
            format!("{}://{host}{path}?{query}", ep.scheme())
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let amz_date = sign::amz_date(now);
        let mut headers: Vec<(String, String)> = vec![
            ("host".into(), host),
            ("x-amz-content-sha256".into(), payload_hash.to_string()),
            ("x-amz-date".into(), amz_date.clone()),
        ];
        headers.extend(
            extra
                .iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string())),
        );
        headers.sort();
        let auth = sign::authorization(&sign::Request {
            method: method.as_str(),
            path: &path,
            query: &query,
            headers: &headers,
            payload_hash,
            amz_date: &amz_date,
            region,
            access_key_id: self.cfg.access_key_id.trim(),
            secret_access_key: &self.cfg.secret_access_key,
        });
        let mut req = self
            .http
            .request(method, url)
            .header(AUTHORIZATION, auth)
            .header("x-amz-content-sha256", payload_hash)
            .header("x-amz-date", amz_date);
        for (k, v) in extra {
            req = req.header(*k, v.as_str());
        }
        if let Some(len) = content_length {
            req = req.header(CONTENT_LENGTH, len);
        }
        req.body(body).send().await.map_err(net)
    }

    /// A request without a body, failing on an error status.
    async fn call(
        &self,
        method: Method,
        t: Target<'_>,
        extra: &[(&str, String)],
    ) -> Result<Response> {
        let region = match t.bucket {
            Some(b) => self.region(b).await,
            None => self.region_default().to_string(),
        };
        let len = (method == Method::PUT).then_some(0);
        check(
            self.send_in(
                &region,
                method,
                &t,
                extra,
                len,
                Body::from(""),
                EMPTY_SHA256,
            )
            .await?,
        )
        .await
    }

    pub async fn list_buckets(&self) -> Result<Vec<BucketInfo>> {
        let target = Target {
            bucket: None,
            key: "",
            query: &[],
        };
        let body = self
            .call(Method::GET, target, &[])
            .await?
            .text()
            .await
            .map_err(net)?;
        let mut out: Vec<BucketInfo> = xml::all(&body, "Bucket")
            .into_iter()
            .filter_map(|b| {
                Some(BucketInfo {
                    name: xml::first(b, "Name")?,
                    created: xml::first(b, "CreationDate").and_then(|d| sign::parse_timestamp(&d)),
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// One level below `prefix` (empty, or ending in `/`), every page.
    pub async fn list(&self, bucket: &str, prefix: &str) -> Result<Listing> {
        let mut out = Listing::default();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2"), ("delimiter", "/"), ("prefix", prefix)];
            if let Some(t) = &token {
                query.push(("continuation-token", t.as_str()));
            }
            let target = Target {
                bucket: Some(bucket),
                key: "",
                query: &query,
            };
            let body = self
                .call(Method::GET, target, &[])
                .await?
                .text()
                .await
                .map_err(net)?;
            for p in xml::all(&body, "CommonPrefixes") {
                if let Some(name) = xml::first(p, "Prefix")
                    .and_then(|p| {
                        p.strip_prefix(prefix)
                            .map(|n| n.trim_end_matches('/').to_string())
                    })
                    .filter(|n| !n.is_empty())
                {
                    out.folders.push(name);
                }
            }
            for c in xml::all(&body, "Contents") {
                let Some(key) = xml::first(c, "Key") else {
                    continue;
                };
                // The folder's own marker object.
                let Some(name) = key.strip_prefix(prefix).filter(|n| !n.is_empty()) else {
                    continue;
                };
                out.objects.push(ObjectInfo {
                    name: name.to_string(),
                    size: xml::first(c, "Size")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    modified: xml::first(c, "LastModified").and_then(|d| sign::parse_timestamp(&d)),
                    key,
                });
            }
            token = xml::first(&body, "NextContinuationToken")
                .filter(|_| xml::first(&body, "IsTruncated").as_deref() == Some("true"));
            if token.is_none() {
                break;
            }
        }
        out.folders.sort_by_key(|n| n.to_lowercase());
        out.objects.sort_by_key(|o| o.name.to_lowercase());
        Ok(out)
    }

    /// Create the folder marker `prefix` (ending in `/`).
    pub async fn create_folder(&self, bucket: &str, prefix: &str) -> Result<()> {
        let target = Target {
            bucket: Some(bucket),
            key: prefix,
            query: &[],
        };
        self.call(Method::PUT, target, &[]).await.map(|_| ())
    }

    pub async fn delete(&self, bucket: &str, key: &str) -> Result<()> {
        let target = Target {
            bucket: Some(bucket),
            key,
            query: &[],
        };
        self.call(Method::DELETE, target, &[]).await.map(|_| ())
    }

    /// Delete the folder `prefix` (ending in `/`) if nothing but its marker is inside.
    pub async fn delete_folder(&self, bucket: &str, prefix: &str) -> Result<()> {
        let query = [("list-type", "2"), ("prefix", prefix), ("max-keys", "2")];
        let target = Target {
            bucket: Some(bucket),
            key: "",
            query: &query,
        };
        let body = self
            .call(Method::GET, target, &[])
            .await?
            .text()
            .await
            .map_err(net)?;
        if xml::all(&body, "Contents")
            .into_iter()
            .any(|c| xml::first(c, "Key").as_deref() != Some(prefix))
        {
            return Err(Error::Invalid("the folder is not empty".into()));
        }
        self.delete(bucket, prefix).await
    }

    /// Server-side copy of `from` to `to` in the same bucket.
    pub async fn copy(&self, bucket: &str, from: &str, to: &str) -> Result<()> {
        let source = format!(
            "/{}/{}",
            sign::uri_encode(bucket, false),
            sign::uri_encode(from, true)
        );
        let target = Target {
            bucket: Some(bucket),
            key: to,
            query: &[],
        };
        let resp = self
            .call(Method::PUT, target, &[("x-amz-copy-source", source)])
            .await?;
        // A copy can fail after the 200 status went out; the body then holds the error.
        let body = resp.text().await.map_err(net)?;
        if body.contains("<Error>") {
            return Err(s3_error(500, &body));
        }
        Ok(())
    }

    /// Whether object `key` exists.
    pub async fn exists(&self, bucket: &str, key: &str) -> Result<bool> {
        let target = Target {
            bucket: Some(bucket),
            key,
            query: &[],
        };
        match self.call(Method::HEAD, target, &[]).await {
            Ok(_) => Ok(true),
            Err(Error::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Copy object `key` to the new local file `local`. `progress(done, total)` follows the
    /// bytes. A failed download removes the partial file.
    pub async fn download(
        &self,
        bucket: &str,
        key: &str,
        local: &Path,
        progress: impl FnMut(u64, u64) + Send,
    ) -> Result<u64> {
        let mut dst = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(local)
            .await
            .map_err(io_err)?;
        let copied = self.download_to(bucket, key, &mut dst, progress).await;
        if copied.is_err() {
            drop(dst);
            let _ = tokio::fs::remove_file(local).await;
        }
        copied
    }

    /// Stream object `key` into `dst` (a file, or one on a server over SFTP).
    pub async fn download_to<W: AsyncWrite + Unpin + Send>(
        &self,
        bucket: &str,
        key: &str,
        dst: &mut W,
        mut progress: impl FnMut(u64, u64) + Send,
    ) -> Result<u64> {
        let target = Target {
            bucket: Some(bucket),
            key,
            query: &[],
        };
        let mut resp = self.call(Method::GET, target, &[]).await?;
        let total = resp.content_length().unwrap_or(0);
        let mut done = 0u64;
        progress(0, total);
        while let Some(chunk) = resp.chunk().await.map_err(net)? {
            dst.write_all(&chunk).await.map_err(io_err)?;
            done += chunk.len() as u64;
            progress(done, total);
        }
        dst.flush().await.map_err(io_err)?;
        Ok(done)
    }

    /// The whole object `key` in memory, refused when it is (or turns out to be) larger than
    /// `limit` bytes, so a mislabelled huge object is not pulled down.
    pub async fn read_all(&self, bucket: &str, key: &str, limit: u64) -> Result<Vec<u8>> {
        let target = Target {
            bucket: Some(bucket),
            key,
            query: &[],
        };
        let mut resp = self.call(Method::GET, target, &[]).await?;
        let too_big = || {
            Error::Invalid(format!(
                "{key} is larger than {limit} bytes; files that big are not opened"
            ))
        };
        let size = resp.content_length().unwrap_or(0);
        if size > limit {
            return Err(too_big());
        }
        let mut out = Vec::with_capacity(size as usize);
        while let Some(chunk) = resp.chunk().await.map_err(net)? {
            out.extend_from_slice(&chunk);
            if out.len() as u64 > limit {
                return Err(too_big());
            }
        }
        Ok(out)
    }

    /// Upload the local file `local` as `key` (created or replaced), up to 5 GiB.
    pub async fn upload(
        &self,
        bucket: &str,
        key: &str,
        local: &Path,
        progress: impl FnMut(u64, u64) + Send + 'static,
    ) -> Result<u64> {
        let file = tokio::fs::File::open(local).await.map_err(io_err)?;
        let total = file.metadata().await.map_err(io_err)?.len();
        self.upload_from(bucket, key, file, total, progress).await
    }

    /// Upload the `total` bytes `src` gives (a file, or one on a server over SFTP) as `key`
    /// (created or replaced), up to 5 GiB.
    pub async fn upload_from<R: AsyncRead + Unpin + Send + 'static>(
        &self,
        bucket: &str,
        key: &str,
        src: R,
        total: u64,
        mut progress: impl FnMut(u64, u64) + Send + 'static,
    ) -> Result<u64> {
        if total > MAX_PUT {
            return Err(Error::Invalid(
                "files over 5 GiB need a multipart upload, which is not supported yet".into(),
            ));
        }
        progress(0, total);
        let stream = futures::stream::unfold(
            (src, 0u64, progress),
            move |(mut f, done, mut p)| async move {
                let mut buf = vec![0u8; CHUNK];
                match f.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        let done = done + n as u64;
                        p(done, total);
                        Some((Ok::<Vec<u8>, std::io::Error>(buf), (f, done, p)))
                    }
                    Err(e) => Some((Err(e), (f, done, p))),
                }
            },
        );
        let region = self.region(bucket).await;
        let target = Target {
            bucket: Some(bucket),
            key,
            query: &[],
        };
        let resp = self
            .send_in(
                &region,
                Method::PUT,
                &target,
                &[],
                Some(total),
                Body::wrap_stream(stream),
                UNSIGNED_PAYLOAD,
            )
            .await?;
        check(resp).await?;
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_s3_errors() {
        let body = "<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message></Error>";
        assert!(matches!(s3_error(404, body), Error::NotFound(m) if m.contains("NoSuchKey")));
        assert!(matches!(s3_error(403, ""), Error::Denied(_)));
        assert!(matches!(
            s3_error(500, "<Error><Code>InternalError</Code></Error>"),
            Error::Other(_)
        ));
    }

    #[test]
    fn needs_credentials() {
        let cfg = S3Config {
            endpoint: String::new(),
            region: "eu-west-1".into(),
            access_key_id: " ".into(),
            secret_access_key: "x".into(),
            path_style: false,
        };
        assert!(S3Client::new(cfg).is_err());
    }
}
