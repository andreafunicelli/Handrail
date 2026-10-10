//! Shared HTTP plumbing: retrying sender, SSE pump, `/models` helper.

use crate::error::{Error, Result};
use crate::sse::{SseEvent, SseParser};
use crate::types::{ModelInfo, ProviderEvent};
use futures::stream::{BoxStream, StreamExt};
use reqwest::{RequestBuilder, Response};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Longest we are willing to wait before the single automatic retry.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);

/// Turns SSE events of one provider dialect into neutral events.
pub(crate) trait Decoder: Send + 'static {
    fn feed(&mut self, ev: &SseEvent) -> Result<Vec<ProviderEvent>>;
    /// The byte stream ended. Flush anything pending, or error if the stream was truncated.
    fn finish(&mut self) -> Result<Vec<ProviderEvent>>;
}

#[derive(Clone)]
pub(crate) struct Http {
    client: reqwest::Client,
    pub retry_backoff: Duration,
}

impl Http {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(120))
            .user_agent(concat!("AIHubTerm/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client");
        Self {
            client,
            retry_backoff: Duration::from_millis(800),
        }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Send a request; on a retryable failure retry exactly once (before any body was consumed).
    pub async fn send(
        &self,
        build: impl Fn() -> RequestBuilder,
        cancel: &CancellationToken,
    ) -> Result<Response> {
        let mut attempt = 0;
        loop {
            let err = match self.send_once(build()).await {
                Ok(r) => return Ok(r),
                Err(e) => e,
            };
            let wait = match &err {
                Error::RateLimited {
                    retry_after: Some(d),
                    ..
                } => *d,
                _ => self.retry_backoff,
            };
            if attempt >= 1 || !err.retryable() || wait > MAX_RETRY_WAIT {
                return Err(err);
            }
            attempt += 1;
            tracing::warn!(error = %err, ?wait, "retrying provider request");
            tokio::select! {
                _ = cancel.cancelled() => return Err(err),
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }

    async fn send_once(&self, rb: RequestBuilder) -> Result<Response> {
        let resp = rb
            .send()
            .await
            .map_err(|e| Error::Network(e.without_url().to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok())
            .map(Duration::from_secs_f64);
        let body = resp.text().await.unwrap_or_default();
        Err(Error::from_status(status.as_u16(), &body, retry_after))
    }

    /// GET a `/models`-style endpoint and parse `{"data":[{"id":..,"display_name"?:..}]}`.
    pub async fn list_models(&self, build: impl Fn() -> RequestBuilder) -> Result<Vec<ModelInfo>> {
        let resp = self.send(build, &CancellationToken::new()).await?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| Error::Protocol(format!("invalid /models response: {e}")))?;
        let arr = v
            .get("data")
            .or_else(|| v.get("models"))
            .and_then(|d| d.as_array())
            .ok_or_else(|| Error::Protocol("/models response has no data array".into()))?;
        Ok(arr
            .iter()
            .filter_map(|m| {
                let id = m.get("id").and_then(|i| i.as_str())?;
                Some(ModelInfo {
                    id: id.to_string(),
                    display_name: m
                        .get("display_name")
                        .and_then(|n| n.as_str())
                        .map(String::from),
                })
            })
            .collect())
    }
}

/// Consume the response body on a background task, decode SSE into neutral events.
/// The stream ends silently when `cancel` fires or the consumer drops it.
pub(crate) fn pump(
    resp: Response,
    mut decoder: impl Decoder,
    cancel: CancellationToken,
) -> BoxStream<'static, Result<ProviderEvent>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<ProviderEvent>>(64);
    tokio::spawn(async move {
        let mut bytes = resp.bytes_stream();
        let mut sse = SseParser::new();
        // Returns true when the stream is finished (Stop sent, error, or receiver gone).
        async fn dispatch(
            decoder: &mut impl Decoder,
            ev: &SseEvent,
            tx: &tokio::sync::mpsc::Sender<Result<ProviderEvent>>,
        ) -> bool {
            match decoder.feed(ev) {
                Ok(events) => {
                    for e in events {
                        let stop = matches!(e, ProviderEvent::Stop { .. });
                        if tx.send(Ok(e)).await.is_err() || stop {
                            return true;
                        }
                    }
                    false
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    true
                }
            }
        }
        loop {
            let item = tokio::select! {
                _ = cancel.cancelled() => return,
                item = bytes.next() => item,
            };
            match item {
                Some(Ok(chunk)) => {
                    for ev in sse.push(&chunk) {
                        if dispatch(&mut decoder, &ev, &tx).await {
                            return;
                        }
                    }
                }
                Some(Err(e)) => {
                    let _ = tx
                        .send(Err(Error::Network(e.without_url().to_string())))
                        .await;
                    return;
                }
                None => {
                    if let Some(ev) = sse.finish() {
                        if dispatch(&mut decoder, &ev, &tx).await {
                            return;
                        }
                    }
                    match decoder.finish() {
                        Ok(events) => {
                            for e in events {
                                if tx.send(Ok(e)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
                        }
                    }
                    return;
                }
            }
        }
    });
    futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|i| (i, rx)) }).boxed()
}

/// Parse accumulated tool-call argument JSON; empty means "no arguments".
pub(crate) fn parse_tool_args(buf: &str) -> Result<Value> {
    if buf.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(buf)
        .map_err(|e| Error::Protocol(format!("tool call arguments are not valid JSON: {e}")))
}
