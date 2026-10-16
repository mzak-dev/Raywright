//! Waiting with a deadline: the loop under every auto-waiting action and `expect`.

use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::error::{Error, Result};

/// Pauses between attempts: about a frame at first (so "stable" means two frames agree),
/// then longer, then every half second.
pub const BACKOFF_MS: [u64; 6] = [16, 32, 64, 128, 256, 500];

/// What one attempt found.
#[derive(Debug)]
pub enum Attempt<T> {
    Done(T),
    /// Not yet; why (shown when time runs out).
    Retry(String),
    /// Stop now, retrying can't help.
    Fail(Error),
}

/// Runs `attempt` until it is done, fails, or `timeout` passes (then
/// [`Error::Timeout`]). Always tries at least once, and once more at the deadline.
pub async fn retry<T, F, Fut>(timeout: Duration, action: &str, target: &str, mut attempt: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Attempt<T>>,
{
    let start = Instant::now();
    let deadline = start + timeout;
    let mut n = 0usize;
    loop {
        let last = match attempt().await {
            Attempt::Done(v) => return Ok(v),
            Attempt::Fail(e) => return Err(e),
            Attempt::Retry(why) => why,
        };
        let now = Instant::now();
        if now >= deadline {
            return Err(Error::Timeout { action: action.to_string(), target: target.to_string(), waited: now - start, last });
        }
        let pause = Duration::from_millis(BACKOFF_MS[n.min(BACKOFF_MS.len() - 1)]);
        n += 1;
        tokio::time::sleep_until((now + pause).min(deadline)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn retries_until_done() {
        let mut calls = 0;
        let v = retry(Duration::from_secs(5), "click", "x", || {
            calls += 1;
            let c = calls;
            async move { if c < 4 { Attempt::Retry(format!("try {c}")) } else { Attempt::Done(c) } }
        })
        .await
        .unwrap();
        assert_eq!(v, 4);
    }

    #[tokio::test(start_paused = true)]
    async fn times_out_with_the_last_reason() {
        let start = Instant::now();
        let e = retry::<(), _, _>(Duration::from_secs(2), "click", "button", || async { Attempt::Retry("not visible".into()) }).await.unwrap_err();
        assert_eq!(start.elapsed(), Duration::from_secs(2));
        match e {
            Error::Timeout { last, action, .. } => {
                assert_eq!(last, "not visible");
                assert_eq!(action, "click");
            }
            other => panic!("{other}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn fail_stops_at_once() {
        let e = retry::<(), _, _>(Duration::from_secs(60), "a", "b", || async { Attempt::Fail(Error::other("boom")) }).await.unwrap_err();
        assert_eq!(e.to_string(), "boom");
    }

    #[tokio::test(start_paused = true)]
    async fn zero_timeout_still_tries_once() {
        let v = retry(Duration::ZERO, "a", "b", || async { Attempt::Done(1) }).await.unwrap();
        assert_eq!(v, 1);
    }
}
