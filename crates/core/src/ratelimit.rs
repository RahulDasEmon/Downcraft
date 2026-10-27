//! Shared pacing limiter (the idea behind FluxDown's global token bucket, in blocking form).
//!
//! Every chunk a worker receives *reserves* its transmission time on a timeline; the caller
//! sleeps until its slot ends. Because all workers reserve on the same timeline, the combined
//! rate honours the limit no matter how many connections are open, and a limit changed at
//! runtime applies to the very next chunk. Up to 250 ms of idle credit is kept so normal
//! scheduling jitter does not lose throughput.
use parking_lot::Mutex;
use std::time::{Duration, Instant};

const BURST: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct RateLimiter {
    next: Mutex<Option<Instant>>,
}

impl RateLimiter {
    /// Books `n` bytes at `limit_bps` (0 = unlimited) and returns how long the caller must wait.
    pub fn reserve(&self, n: u64, limit_bps: u64) -> Duration {
        let mut g = self.next.lock();
        if limit_bps == 0 || n == 0 {
            *g = None;
            return Duration::ZERO;
        }
        let now = Instant::now();
        let floor = now.checked_sub(BURST).unwrap_or(now);
        let start = g.map_or(floor, |t| t.max(floor));
        let end = start + Duration::from_secs_f64(n as f64 / limit_bps as f64);
        *g = Some(end);
        end.saturating_duration_since(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let l = RateLimiter::default();
        assert_eq!(l.reserve(10_000_000, 0), Duration::ZERO);
    }

    #[test]
    fn waits_accumulate_across_callers() {
        let l = RateLimiter::default();
        // 1 MB/s: after the 250 ms idle credit, each 100 kB chunk books 100 ms more.
        let mut last = Duration::ZERO;
        for _ in 0..20 {
            last = l.reserve(100_000, 1_000_000);
        }
        // 2 s of data booked, 250 ms of it free.
        assert!(last > Duration::from_millis(1500) && last < Duration::from_millis(2000), "{last:?}");
    }
}
