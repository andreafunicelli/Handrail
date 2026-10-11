//! GitHub OAuth device flow (RFC 8628): the app shows `user_code` + `verification_uri`, then
//! polls until the user authorizes.
//!
//! Uses the registered "Handrail" GitHub OAuth App (device flow enabled). Its client id is
//! public configuration, not a secret, and the device flow needs no client secret. The
//! `HANDRAIL_GITHUB_CLIENT_ID` environment variable overrides it, e.g. for a fork's own app
//! (see [`client_id`]).

use crate::error::{Result, SyncError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Client id of the registered "Handrail" OAuth App.
pub const HANDRAIL_GITHUB_CLIENT_ID: &str = "Ov23li7s0xtB6dHz5HgF";
pub const DEFAULT_BASE: &str = "https://github.com";
pub const DEFAULT_SCOPE: &str = "repo";

/// Client id from the `HANDRAIL_GITHUB_CLIENT_ID` env var, else the compiled-in one.
pub fn client_id() -> String {
    std::env::var("HANDRAIL_GITHUB_CLIENT_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| HANDRAIL_GITHUB_CLIENT_ID.to_string())
}

/// False for an empty id or a leftover placeholder (e.g. a fork that has not registered its own
/// app yet).
pub fn is_configured(client_id: &str) -> bool {
    let id = client_id.trim();
    !id.is_empty() && !id.starts_with("REPLACE_")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Minimum seconds between polls.
    pub interval: u64,
    pub expires_in: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub enum PollResult {
    Pending,
    /// Poll less often: add 5 seconds to the interval.
    SlowDown,
    Token(String),
    Expired,
    Denied,
}

impl std::fmt::Debug for PollResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PollResult::Pending => write!(f, "Pending"),
            PollResult::SlowDown => write!(f, "SlowDown"),
            PollResult::Token(_) => write!(f, "Token(<redacted>)"),
            PollResult::Expired => write!(f, "Expired"),
            PollResult::Denied => write!(f, "Denied"),
        }
    }
}

async fn post_form(base: &str, path: &str, form: &[(&str, &str)]) -> Result<Value> {
    let client = reqwest::Client::builder().user_agent("Handrail").build()?;
    let resp = client
        .post(format!("{}{path}", base.trim_end_matches('/')))
        .header("Accept", "application/json")
        .form(form)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await?;
    let body: Value = serde_json::from_str(&text)
        .map_err(|_| SyncError::Protocol(format!("non-JSON response ({status})")))?;
    if !status.is_success() && body["error"].is_null() {
        return Err(SyncError::Http(format!(
            "device flow request failed: {status}"
        )));
    }
    Ok(body)
}

/// Begin the flow against the real github.com.
pub async fn start(client_id: &str, scope: &str) -> Result<DeviceCode> {
    start_at(DEFAULT_BASE, client_id, scope).await
}

pub async fn start_at(base: &str, client_id: &str, scope: &str) -> Result<DeviceCode> {
    let body = post_form(
        base,
        "/login/device/code",
        &[("client_id", client_id), ("scope", scope)],
    )
    .await?;
    if let Some(err) = body["error"].as_str() {
        return Err(SyncError::Auth(format!("device flow start failed: {err}")));
    }
    serde_json::from_value(body).map_err(|e| SyncError::Protocol(format!("device code: {e}")))
}

pub async fn poll(client_id: &str, device_code: &str) -> Result<PollResult> {
    poll_at(DEFAULT_BASE, client_id, device_code).await
}

pub async fn poll_at(base: &str, client_id: &str, device_code: &str) -> Result<PollResult> {
    let body = post_form(
        base,
        "/login/oauth/access_token",
        &[
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ],
    )
    .await?;
    if let Some(tok) = body["access_token"].as_str() {
        return Ok(PollResult::Token(tok.to_string()));
    }
    match body["error"].as_str() {
        Some("authorization_pending") => Ok(PollResult::Pending),
        Some("slow_down") => Ok(PollResult::SlowDown),
        Some("expired_token") => Ok(PollResult::Expired),
        Some("access_denied") => Ok(PollResult::Denied),
        Some(other) => Err(SyncError::Auth(format!("device flow error: {other}"))),
        None => Err(SyncError::Protocol(
            "poll response has neither token nor error".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn start_parses_device_code() {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/login/device/code"))
            .and(header("accept", "application/json"))
            .and(body_string_contains("client_id=cid"))
            .and(body_string_contains("scope=repo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dc", "user_code": "ABCD-1234",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 900, "interval": 5
            })))
            .mount(&s)
            .await;
        let d = start_at(&s.uri(), "cid", "repo").await.unwrap();
        assert_eq!(d.user_code, "ABCD-1234");
        assert_eq!(d.interval, 5);
        assert_eq!(d.device_code, "dc");
    }

    #[tokio::test]
    async fn poll_pending_slow_down_then_token() {
        let s = MockServer::start().await;
        for (err, n) in [("authorization_pending", 1u64), ("slow_down", 1)] {
            Mock::given(method("POST"))
                .and(path("/login/oauth/access_token"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({ "error": err })),
                )
                .up_to_n_times(n)
                .expect(n)
                .mount(&s)
                .await;
        }
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .and(body_string_contains("device_code=dc"))
            .and(body_string_contains("grant_type=urn"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({ "access_token": "gho_x", "token_type": "bearer", "scope": "repo" }),
            ))
            .mount(&s)
            .await;
        assert_eq!(
            poll_at(&s.uri(), "cid", "dc").await.unwrap(),
            PollResult::Pending
        );
        assert_eq!(
            poll_at(&s.uri(), "cid", "dc").await.unwrap(),
            PollResult::SlowDown
        );
        assert_eq!(
            poll_at(&s.uri(), "cid", "dc").await.unwrap(),
            PollResult::Token("gho_x".into())
        );
    }

    #[tokio::test]
    async fn poll_expired_and_denied() {
        for (err, want) in [
            ("expired_token", PollResult::Expired),
            ("access_denied", PollResult::Denied),
        ] {
            let s = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({ "error": err })),
                )
                .mount(&s)
                .await;
            assert_eq!(poll_at(&s.uri(), "cid", "dc").await.unwrap(), want);
        }
    }

    #[test]
    fn client_id_defaults_to_the_registered_app() {
        // Not asserting the env override here (process-global); just the fallback.
        if std::env::var("HANDRAIL_GITHUB_CLIENT_ID").is_err() {
            assert_eq!(client_id(), HANDRAIL_GITHUB_CLIENT_ID);
        }
        assert!(is_configured(HANDRAIL_GITHUB_CLIENT_ID));
    }

    #[test]
    fn placeholder_is_not_configured() {
        assert!(!is_configured(
            "REPLACE_WITH_REGISTERED_OAUTH_APP_CLIENT_ID"
        ));
        assert!(!is_configured("  "));
        assert!(is_configured("Ov23liExample0000000"));
    }
}
