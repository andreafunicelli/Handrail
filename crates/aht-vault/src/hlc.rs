//! Hybrid logical clock: wall-clock milliseconds + logical counter + device id tiebreak.
//! Timestamps are totally ordered (field order: `wall_ms`, `counter`, `device`).

use aht_core::DeviceId;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hlc {
    pub wall_ms: u64,
    pub counter: u32,
    pub device: DeviceId,
}

pub(crate) fn wall_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Generates monotonically increasing [`Hlc`] timestamps for one device.
#[derive(Clone, Debug)]
pub struct HlcClock {
    device: DeviceId,
    wall_ms: u64,
    counter: u32,
}

impl HlcClock {
    pub fn new(device: DeviceId) -> Self {
        Self {
            device,
            wall_ms: 0,
            counter: 0,
        }
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// Next timestamp for a local event.
    pub fn now(&mut self) -> Hlc {
        self.tick(wall_now_ms())
    }

    /// Next timestamp given an explicit physical time (used by `now` and tests).
    pub fn tick(&mut self, physical_ms: u64) -> Hlc {
        if physical_ms > self.wall_ms {
            self.wall_ms = physical_ms;
            self.counter = 0;
        } else {
            self.bump_counter();
        }
        self.current()
    }

    /// Merge a timestamp seen from another device (or from disk) so later local events sort after it.
    pub fn observe(&mut self, remote: &Hlc) {
        if remote.wall_ms > self.wall_ms {
            self.wall_ms = remote.wall_ms;
            self.counter = remote.counter;
        } else if remote.wall_ms == self.wall_ms && remote.counter > self.counter {
            self.counter = remote.counter;
        }
    }

    fn bump_counter(&mut self) {
        if self.counter == u32::MAX {
            self.wall_ms += 1;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
    }

    fn current(&self) -> Hlc {
        Hlc {
            wall_ms: self.wall_ms,
            counter: self.counter,
            device: self.device,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_even_if_wall_clock_goes_back() {
        let mut c = HlcClock::new(DeviceId::new());
        let a = c.tick(1000);
        let b = c.tick(1000);
        let d = c.tick(500);
        let e = c.tick(2000);
        assert!(a < b && b < d && d < e);
        assert_eq!((d.wall_ms, d.counter), (1000, 2));
        assert_eq!((e.wall_ms, e.counter), (2000, 0));
    }

    #[test]
    fn observe_moves_clock_forward() {
        let mut local = HlcClock::new(DeviceId::new());
        let mut remote = HlcClock::new(DeviceId::new());
        local.tick(100);
        let r = remote.tick(5000);
        local.observe(&r);
        let next = local.tick(200);
        assert!(next > r);
        assert_eq!((next.wall_ms, next.counter), (5000, 1));
    }

    #[test]
    fn device_breaks_ties_and_total_order() {
        let d1 = DeviceId::new();
        let d2 = DeviceId::new();
        let a = Hlc {
            wall_ms: 1,
            counter: 0,
            device: d1,
        };
        let b = Hlc { device: d2, ..a };
        assert_ne!(a, b);
        assert_eq!(a < b, d1 < d2);
        let c = Hlc { counter: 1, ..a };
        assert!(c > a && c > b);
    }

    #[test]
    fn serde_round_trip() {
        let mut c = HlcClock::new(DeviceId::new());
        let h = c.now();
        let s = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<Hlc>(&s).unwrap(), h);
    }
}
