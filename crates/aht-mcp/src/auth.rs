//! Bearer-token registry and the axum middleware that guards `/mcp`.

use aht_core::terminal::Actor;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use std::{
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
};
use subtle::ConstantTimeEq;

/// A per-launch bearer token: 256 random bits, base64url (no padding).
///
/// `Debug` is redacted so tokens never end up in logs by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct McpToken(String);

impl McpToken {
    pub(crate) fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::fill(&mut bytes);
        Self(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
    }

    /// The raw token, to hand to the harness process (e.g. via an env var).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Value for an `Authorization` header: `Bearer <token>`.
    pub fn bearer_header(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

impl fmt::Debug for McpToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("McpToken(<redacted>)")
    }
}

/// Inserted into the HTTP request extensions by the middleware; read by the MCP handler.
#[derive(Clone, Debug)]
pub(crate) struct AuthContext {
    /// Unique per issued token; used to pin an MCP session to the token that opened it.
    pub token_id: u64,
    pub actor: Arc<Actor>,
}

struct Entry {
    token: McpToken,
    ctx: AuthContext,
}

#[derive(Default)]
pub(crate) struct TokenStore {
    entries: RwLock<Vec<Entry>>,
    next_id: AtomicU64,
}

impl TokenStore {
    pub fn issue(&self, actor: Actor) -> McpToken {
        let token = McpToken::generate();
        let token_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let ctx = AuthContext {
            token_id,
            actor: Arc::new(actor),
        };
        self.entries
            .write()
            .expect("token store poisoned")
            .push(Entry {
                token: token.clone(),
                ctx,
            });
        token
    }

    pub fn revoke(&self, token: &McpToken) {
        self.entries
            .write()
            .expect("token store poisoned")
            .retain(|e| !bool::from(e.token.0.as_bytes().ct_eq(token.0.as_bytes())));
    }

    /// Constant-time lookup: every stored token is compared, with no early exit.
    pub fn lookup(&self, presented: &str) -> Option<AuthContext> {
        let entries = self.entries.read().expect("token store poisoned");
        let mut found = None;
        for e in entries.iter() {
            if bool::from(e.token.0.as_bytes().ct_eq(presented.as_bytes())) {
                found = Some(e.ctx.clone());
            }
        }
        found
    }
}

#[derive(Clone)]
pub(crate) struct GuardState {
    pub tokens: Arc<TokenStore>,
    pub port: u16,
}

fn reject(status: StatusCode, msg: &'static str) -> Response {
    let mut resp = (status, msg).into_response();
    if status == StatusCode::UNAUTHORIZED {
        resp.headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    resp
}

/// `Host` must be exactly `127.0.0.1:PORT` or `localhost:PORT` (DNS-rebinding defense).
pub(crate) fn host_allowed(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

/// No `Origin`, or an empty one, is fine (non-browser clients). Otherwise it must be a
/// loopback http(s) origin.
pub(crate) fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if origin.is_empty() {
        return true;
    }
    let origin = origin.to_ascii_lowercase();
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let host = if let Some(v6) = rest.strip_prefix('[') {
        match v6.split_once(']') {
            Some((h, tail)) if tail.is_empty() || is_port(tail) => return h == "::1",
            _ => return false,
        }
    } else {
        match rest.split_once(':') {
            Some((h, p)) if is_port(&format!(":{p}")) => h,
            Some(_) => return false,
            None => rest,
        }
    };
    host == "localhost" || host == "127.0.0.1"
}

fn is_port(tail: &str) -> bool {
    tail.strip_prefix(':')
        .is_some_and(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|t| !t.is_empty())
}

pub(crate) async fn guard(
    State(state): State<GuardState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let headers = req.headers();
    if !host_allowed(headers, state.port) {
        tracing::warn!("mcp: rejected request with disallowed Host header");
        return reject(StatusCode::FORBIDDEN, "forbidden: invalid Host header");
    }
    if !origin_allowed(headers) {
        tracing::warn!("mcp: rejected request with non-local Origin");
        return reject(StatusCode::FORBIDDEN, "forbidden: invalid Origin");
    }
    let Some(ctx) = bearer(headers).and_then(|t| state.tokens.lookup(t)) else {
        tracing::debug!("mcp: rejected request without a valid bearer token");
        return reject(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    req.extensions_mut().insert(ctx);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: header::HeaderName, v: &str) -> HeaderMap {
        let mut m = HeaderMap::new();
        m.insert(name, HeaderValue::from_str(v).unwrap());
        m
    }

    #[test]
    fn token_is_256_bit_base64url() {
        let t = McpToken::generate();
        assert_eq!(t.as_str().len(), 43);
        assert!(t
            .as_str()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_ne!(t, McpToken::generate());
        assert_eq!(format!("{t:?}"), "McpToken(<redacted>)");
    }

    #[test]
    fn store_issue_lookup_revoke() {
        let s = TokenStore::default();
        let a = s.issue(Actor::user());
        let b = s.issue(Actor {
            id: "b".into(),
            label: "B".into(),
            session_scope: None,
        });
        assert_eq!(s.lookup(a.as_str()).unwrap().actor.id, "user");
        assert_eq!(s.lookup(b.as_str()).unwrap().actor.id, "b");
        assert!(s.lookup("nope").is_none());
        assert!(s.lookup("").is_none());
        s.revoke(&a);
        assert!(s.lookup(a.as_str()).is_none());
        assert!(s.lookup(b.as_str()).is_some());
    }

    #[test]
    fn host_checks() {
        assert!(host_allowed(&h(header::HOST, "127.0.0.1:5000"), 5000));
        assert!(host_allowed(&h(header::HOST, "LOCALHOST:5000"), 5000));
        assert!(!host_allowed(&h(header::HOST, "127.0.0.1:5001"), 5000));
        assert!(!host_allowed(&h(header::HOST, "127.0.0.1"), 5000));
        assert!(!host_allowed(&h(header::HOST, "evil.com:5000"), 5000));
        assert!(!host_allowed(&HeaderMap::new(), 5000));
    }

    #[test]
    fn origin_checks() {
        assert!(origin_allowed(&HeaderMap::new()));
        assert!(origin_allowed(&h(header::ORIGIN, "")));
        for ok in [
            "http://localhost",
            "http://localhost:3000",
            "https://127.0.0.1:8443",
            "http://[::1]:80",
        ] {
            assert!(origin_allowed(&h(header::ORIGIN, ok)), "{ok}");
        }
        for bad in [
            "null",
            "http://evil.com",
            "http://localhost.evil.com",
            "http://127.0.0.1.evil.com:80",
            "http://localhost:80@evil.com",
            "file://",
            "tauri://localhost",
            "http://[::2]",
        ] {
            assert!(!origin_allowed(&h(header::ORIGIN, bad)), "{bad}");
        }
    }

    #[test]
    fn bearer_parsing() {
        assert_eq!(bearer(&h(header::AUTHORIZATION, "Bearer abc")), Some("abc"));
        assert_eq!(bearer(&h(header::AUTHORIZATION, "bearer abc")), Some("abc"));
        assert_eq!(bearer(&h(header::AUTHORIZATION, "Basic abc")), None);
        assert_eq!(bearer(&h(header::AUTHORIZATION, "Bearer ")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }
}
