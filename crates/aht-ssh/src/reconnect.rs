//! Exponential-backoff reconnect helper. The app decides when to call it.

use crate::client::{connect_shell, SshParams, SshShell};
use crate::verify::HostKeyVerifier;
use aht_core::terminal::TransportEvent;
use aht_core::{Error, Result};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: f64,
    /// Total attempts including the first; `None` = retry forever.
    pub max_attempts: Option<u32>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            max_attempts: Some(10),
        }
    }
}

impl ReconnectPolicy {
    /// Delay to wait after failed attempt number `failed` (0-based).
    pub fn delay(&self, failed: u32) -> Duration {
        let secs = self.initial_delay.as_secs_f64() * self.multiplier.powi(failed as i32);
        Duration::from_secs_f64(secs.min(self.max_delay.as_secs_f64()))
    }
}

/// Errors worth retrying. Auth failures and host-key problems need the user and are never retried.
pub fn is_retryable(e: &Error) -> bool {
    matches!(e, Error::Network(_) | Error::Io(_) | Error::Other(_))
}

/// Run `op` until it succeeds, a non-retryable error occurs, or attempts run out.
/// `op` receives the 0-based attempt number. The first attempt runs immediately.
pub async fn retry_with_backoff<T, F, Fut>(policy: &ReconnectPolicy, mut op: F) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 0;
    loop {
        match op(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) if !is_retryable(&e) => return Err(e),
            Err(e) => {
                if policy.max_attempts.is_some_and(|m| attempt + 1 >= m) {
                    return Err(e);
                }
                tracing::debug!("attempt {attempt} failed: {e}; backing off");
                tokio::time::sleep(policy.delay(attempt)).await;
                attempt += 1;
            }
        }
    }
}

/// Re-establish an interactive shell with backoff.
pub async fn reconnect(
    params: &SshParams,
    verifier: Arc<dyn HostKeyVerifier>,
    policy: &ReconnectPolicy,
) -> Result<(SshShell, mpsc::Receiver<TransportEvent>)> {
    retry_with_backoff(policy, |_| connect_shell(params, verifier.clone())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn delays_grow_and_cap() {
        let p = ReconnectPolicy::default();
        assert_eq!(p.delay(0), Duration::from_secs(1));
        assert_eq!(p.delay(1), Duration::from_secs(2));
        assert_eq!(p.delay(3), Duration::from_secs(8));
        assert_eq!(p.delay(10), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn retries_until_success_and_stops_on_fatal() {
        let p = ReconnectPolicy {
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
            multiplier: 2.0,
            max_attempts: Some(5),
        };
        let n = AtomicU32::new(0);
        let r = retry_with_backoff(&p, |a| {
            n.fetch_add(1, Ordering::SeqCst);
            async move {
                if a < 2 {
                    Err(Error::Network("down".into()))
                } else {
                    Ok(a)
                }
            }
        })
        .await;
        assert_eq!(r.unwrap(), 2);
        assert_eq!(n.load(Ordering::SeqCst), 3);

        let n = AtomicU32::new(0);
        let r: Result<()> = retry_with_backoff(&p, |_| {
            n.fetch_add(1, Ordering::SeqCst);
            async { Err(Error::HostKeyMismatch("x".into())) }
        })
        .await;
        assert!(matches!(r, Err(Error::HostKeyMismatch(_))));
        assert_eq!(n.load(Ordering::SeqCst), 1);

        let n = AtomicU32::new(0);
        let r: Result<()> = retry_with_backoff(&p, |_| {
            n.fetch_add(1, Ordering::SeqCst);
            async { Err(Error::Network("down".into())) }
        })
        .await;
        assert!(r.is_err());
        assert_eq!(n.load(Ordering::SeqCst), 5);
    }
}
