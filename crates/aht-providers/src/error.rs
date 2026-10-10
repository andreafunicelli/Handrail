use std::time::Duration;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Provider-level error. Converts into [`aht_core::Error`] at the crate boundary.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    /// 401 / 403: bad or missing API key.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// 429. `retry_after` is parsed from the `retry-after` header when present.
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after: Option<Duration>,
    },
    /// 404: unknown model or endpoint.
    #[error("not found: {0}")]
    NotFound(String),
    /// 5xx (incl. 529 overloaded).
    #[error("server error {status}: {message}")]
    Server { status: u16, message: String },
    /// Other 4xx.
    #[error("bad request ({status}): {message}")]
    BadRequest { status: u16, message: String },
    /// Connection / transport failure.
    #[error("network error: {0}")]
    Network(String),
    /// Malformed or truncated stream / response.
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl Error {
    /// Whether retrying the same request later may succeed.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Error::RateLimited { .. } | Error::Server { .. } | Error::Network(_)
        )
    }

    pub(crate) fn from_status(status: u16, body: &str, retry_after: Option<Duration>) -> Self {
        let message = extract_message(body);
        match status {
            401 | 403 => Error::Auth(message),
            404 => Error::NotFound(message),
            429 => Error::RateLimited {
                message,
                retry_after,
            },
            500..=599 => Error::Server { status, message },
            _ => Error::BadRequest { status, message },
        }
    }
}

/// Pull `error.message` (OpenAI/Anthropic shape) out of a JSON body, else a trimmed text body.
fn extract_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(m) = v
            .pointer("/error/message")
            .or_else(|| v.get("message"))
            .and_then(|m| m.as_str())
        {
            return m.to_string();
        }
        if let Some(m) = v.get("error").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    let t = body.trim();
    if t.is_empty() {
        "(empty response body)".into()
    } else {
        t.chars().take(500).collect()
    }
}

impl From<Error> for aht_core::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Auth(m) => aht_core::Error::Auth(m),
            Error::NotFound(m) => aht_core::Error::NotFound(m),
            Error::Network(m) => aht_core::Error::Network(m),
            other => aht_core::Error::Provider(other.to_string()),
        }
    }
}
