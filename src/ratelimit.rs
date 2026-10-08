use std::time::Duration;
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};
use tokio::time::{sleep_until, Instant};

/// Client-side throttle shared by every request an `NseClient` makes.
///
/// Two limits apply together:
/// - **pacing** — request start times are spaced at least `1 / requests_per_sec` apart,
/// - **concurrency** — at most `max_concurrent` requests are in flight at once.
///
/// When NSE pushes back (403/429), [`RateLimiter::cool_down`] delays every
/// queued request so the whole client backs off instead of hammering on.
pub struct RateLimiter {
    permits: Semaphore,
    min_gap: Duration,
    next_slot: Mutex<Instant>,
}

impl RateLimiter {
    /// `requests_per_sec <= 0` (or non-finite) disables pacing.
    pub fn new(requests_per_sec: f64, max_concurrent: usize) -> Self {
        let min_gap = if requests_per_sec.is_finite() && requests_per_sec > 0.0 {
            Duration::from_secs_f64(1.0 / requests_per_sec)
        } else {
            Duration::ZERO
        };
        Self {
            permits: Semaphore::new(max_concurrent.max(1)),
            min_gap,
            next_slot: Mutex::new(Instant::now()),
        }
    }

    /// Wait for a slot to send one HTTP request.  Hold the returned permit
    /// until the response has been read.
    pub async fn acquire(&self) -> SemaphorePermit<'_> {
        self.acquire_weighted(1).await
    }

    /// Like [`Self::acquire`] but books `requests` pacing slots, for an
    /// operation that sends several requests back to back.
    pub async fn acquire_weighted(&self, requests: u32) -> SemaphorePermit<'_> {
        let permit = self.permits.acquire().await.expect("semaphore is never closed");
        let start_at = {
            let mut next = self.next_slot.lock().await;
            let slot = (*next).max(Instant::now());
            *next = slot + self.min_gap * requests.max(1);
            slot
        };
        sleep_until(start_at).await;
        permit
    }

    /// Push every not-yet-started request back by at least `pause`.
    pub async fn cool_down(&self, pause: Duration) {
        let mut next = self.next_slot.lock().await;
        let until = Instant::now() + pause;
        if *next < until {
            *next = until;
        }
    }
}

/// Exponential backoff with ±25 % jitter: `base * 2^attempt`, capped at `max`.
pub fn backoff(base: Duration, attempt: u32, max: Duration) -> Duration {
    let exp = base.saturating_mul(1u32 << attempt.min(16)).min(max);
    // Cheap jitter without pulling in `rand`: sub-second clock nanos.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let factor = 0.75 + (nanos % 1000) as f64 / 2000.0; // 0.75 ..= 1.25
    exp.mul_f64(factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn paces_requests() {
        let rl = RateLimiter::new(4.0, 8); // 250 ms apart
        let t0 = Instant::now();
        for _ in 0..5 {
            drop(rl.acquire().await);
        }
        // 5 requests → 4 gaps of 250 ms.
        assert_eq!(t0.elapsed(), Duration::from_millis(1000));
    }

    #[tokio::test(start_paused = true)]
    async fn cool_down_delays_next_request() {
        let rl = RateLimiter::new(0.0, 1);
        let t0 = Instant::now();
        rl.cool_down(Duration::from_secs(3)).await;
        drop(rl.acquire().await);
        assert_eq!(t0.elapsed(), Duration::from_secs(3));
    }

    #[test]
    fn backoff_is_capped_and_jittered() {
        let max = Duration::from_secs(30);
        for attempt in 0..20 {
            let d = backoff(Duration::from_millis(500), attempt, max);
            assert!(d <= max.mul_f64(1.25));
        }
        let first = backoff(Duration::from_millis(500), 0, max);
        assert!(first >= Duration::from_millis(375) && first <= Duration::from_millis(625));
    }
}
