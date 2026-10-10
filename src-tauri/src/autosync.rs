//! Background sync. While the vault is unlocked and sync is configured with auto-sync on, the
//! vault syncs right after unlock, a few seconds after a local edit, and every few minutes to
//! pick up other devices' changes. Failures back off exponentially.
//!
//! [`Scheduler`] is pure (time is passed in) so the policy is unit-tested; [`spawn`] drives it.

use crate::cmd_misc;
use crate::state::AppState;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// How often the loop looks at the vault.
const TICK: Duration = Duration::from_secs(2);
/// Quiet time after the last local edit before syncing it.
const DEBOUNCE: Duration = Duration::from_secs(3);
/// Pull other devices' changes this often when nothing changes locally.
const INTERVAL: Duration = Duration::from_secs(5 * 60);
const BACKOFF_FIRST: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(15 * 60);

/// What the loop sees on one tick.
#[derive(Clone, Copy, Debug)]
pub struct Observation {
    /// Sync is configured and auto-sync is on.
    pub enabled: bool,
    pub unlocked: bool,
    /// [`aht_vault::Vault::revision`].
    pub revision: u64,
    pub now: Instant,
}

#[derive(Debug, Default)]
pub struct Scheduler {
    /// Vault revision right after the last successful sync. `None` until the first sync after
    /// unlock (or after auto-sync is switched on), which therefore runs at once.
    synced_rev: Option<u64>,
    /// Last revision seen and when it was first seen (debounce of local edits).
    seen_rev: Option<(u64, Instant)>,
    last_attempt: Option<Instant>,
    failures: u32,
}

impl Scheduler {
    pub fn due(&mut self, o: &Observation) -> bool {
        if !o.enabled || !o.unlocked {
            // Start over: the next unlock (or re-enable) syncs immediately.
            *self = Scheduler {
                last_attempt: self.last_attempt,
                ..Scheduler::default()
            };
            return false;
        }
        if self.seen_rev.map(|(r, _)| r) != Some(o.revision) {
            self.seen_rev = Some((o.revision, o.now));
        }
        let since = |t: Instant| o.now.saturating_duration_since(t);
        if self.failures > 0 {
            return self.last_attempt.is_none_or(|t| since(t) >= self.backoff());
        }
        match self.synced_rev {
            None => true,
            Some(r) if r != o.revision => self.seen_rev.is_some_and(|(_, t)| since(t) >= DEBOUNCE),
            Some(_) => self.last_attempt.is_none_or(|t| since(t) >= INTERVAL),
        }
    }

    /// Record the outcome of a sync. `revision_after` is the vault revision once it finished
    /// (merges bump it; those changes came from the remote and need no push).
    pub fn finished(&mut self, ok: bool, revision_after: u64, now: Instant) {
        self.last_attempt = Some(now);
        if ok {
            self.failures = 0;
            self.synced_rev = Some(revision_after);
        } else {
            self.failures = self.failures.saturating_add(1);
        }
    }

    fn backoff(&self) -> Duration {
        let exp = self.failures.saturating_sub(1).min(16);
        BACKOFF_FIRST.saturating_mul(1u32 << exp).min(BACKOFF_MAX)
    }
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut sched = Scheduler::default();
        loop {
            tokio::time::sleep(TICK).await;
            let state = app.state::<AppState>();
            let enabled = {
                let f = cmd_misc::load_sync(&state);
                f.config.is_some() && f.auto
            };
            let (unlocked, revision) = {
                let v = state.vault.lock().await;
                let st = v.status();
                (st.initialized && !st.locked, v.revision())
            };
            let obs = Observation {
                enabled,
                unlocked,
                revision,
                now: Instant::now(),
            };
            if !sched.due(&obs) {
                continue;
            }
            let ok = match cmd_misc::run_sync(&app).await {
                Ok(_) => true,
                Err(e) => {
                    tracing::debug!(error = %e, "background sync failed");
                    false
                }
            };
            let revision_after = state.vault.lock().await.revision();
            sched.finished(ok, revision_after, Instant::now());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, secs: u64) -> Instant {
        t0 + Duration::from_secs(secs)
    }

    fn obs(now: Instant, revision: u64) -> Observation {
        Observation {
            enabled: true,
            unlocked: true,
            revision,
            now,
        }
    }

    #[test]
    fn syncs_on_unlock_then_after_quiet_edits_then_on_interval() {
        let t0 = Instant::now();
        let mut s = Scheduler::default();
        // Locked: nothing.
        let locked = Observation {
            unlocked: false,
            ..obs(t0, 0)
        };
        assert!(!s.due(&locked));
        // Unlock: sync at once.
        assert!(s.due(&obs(at(t0, 1), 0)));
        s.finished(true, 0, at(t0, 1));
        assert!(!s.due(&obs(at(t0, 2), 0)));
        // A local edit: wait until it has been quiet for DEBOUNCE.
        assert!(!s.due(&obs(at(t0, 10), 1)));
        assert!(
            !s.due(&obs(at(t0, 11), 2)),
            "another edit restarts the wait"
        );
        assert!(!s.due(&obs(at(t0, 13), 2)));
        assert!(s.due(&obs(at(t0, 14), 2)));
        s.finished(true, 2, at(t0, 14));
        // Nothing changes: next sync after INTERVAL.
        assert!(!s.due(&obs(at(t0, 14 + 299), 2)));
        assert!(s.due(&obs(at(t0, 14 + 300), 2)));
    }

    #[test]
    fn changes_pulled_by_the_sync_itself_do_not_trigger_another() {
        let t0 = Instant::now();
        let mut s = Scheduler::default();
        assert!(s.due(&obs(t0, 5)));
        // The merge bumped the revision to 9.
        s.finished(true, 9, at(t0, 1));
        for secs in [2, 10, 60] {
            assert!(!s.due(&obs(at(t0, secs), 9)), "{secs}");
        }
    }

    #[test]
    fn failures_back_off_and_success_resets() {
        let t0 = Instant::now();
        let mut s = Scheduler::default();
        assert!(s.due(&obs(t0, 0)));
        s.finished(false, 0, t0);
        assert!(!s.due(&obs(at(t0, 29), 0)));
        assert!(s.due(&obs(at(t0, 30), 0)));
        s.finished(false, 0, at(t0, 30));
        // Second failure: 60 s, even though a local edit is waiting.
        assert!(!s.due(&obs(at(t0, 30 + 59), 1)));
        assert!(s.due(&obs(at(t0, 30 + 60), 1)));
        for _ in 0..20 {
            s.finished(false, 1, at(t0, 100));
        }
        assert_eq!(s.backoff(), BACKOFF_MAX);
        s.finished(true, 1, at(t0, 200));
        assert!(!s.due(&obs(at(t0, 201), 1)));
    }

    #[test]
    fn disabling_resets_so_reenabling_syncs_at_once_but_keeps_backoff_clock() {
        let t0 = Instant::now();
        let mut s = Scheduler::default();
        assert!(s.due(&obs(t0, 0)));
        s.finished(true, 0, t0);
        let off = Observation {
            enabled: false,
            ..obs(at(t0, 5), 0)
        };
        assert!(!s.due(&off));
        assert!(s.due(&obs(at(t0, 6), 0)));
    }
}
