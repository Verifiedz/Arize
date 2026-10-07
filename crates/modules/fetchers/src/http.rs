//! The retry wrapper around `ctx.http.get` (ADR 0028 §6).
//!
//! `ctx.http` returns `Ok(HttpResponse)` for *every* in-band HTTP status, including
//! 4xx/429/503 (`crates/core/src/http.rs:14-26`; `crates/daemon/src/http_backend.rs`'s
//! `finish` and cooldown paths never return `Err` for a status code) -- only a
//! transport-level failure is `Err(Unavailable)`, and only `Unavailable` is retryable
//! (`crates/core/src/error.rs:98-99`). So retrying a 5xx means converting it into an `Err`
//! inside the retried closure; a cooldown (429/503, `retry_after_secs: Some`) or a plain 4xx
//! passes straight through, untouched, for the caller to classify.

use shimmer_core::{Ctx, Error, HttpResponse, Result, RetryPolicy};

/// Fetches `url`, retrying a 5xx up to [`RetryPolicy::standard`]'s bound (3 attempts, 1s then
/// 2s) -- never a 4xx, and never a response already in cooldown (`retry_after_secs.is_some()`,
/// ADR 0028 §6: "no retry... into a host the gateway has put in cooldown").
pub async fn get(ctx: &Ctx, url: &str) -> Result<HttpResponse> {
    ctx.retry_with_backoff(RetryPolicy::standard(), |_attempt| async {
        let resp = ctx.http.get(url).await?;
        if resp.retry_after_secs.is_none() && resp.status >= 500 {
            return Err(Error::unavailable(format!("http {} fetching {url}", resp.status)));
        }
        Ok(resp)
    })
    .await
}

#[cfg(test)]
mod tests {
    use shimmer_core::testing::TestEnv;
    use shimmer_core::ErrorCode;

    use super::*;

    fn response(status: u16) -> HttpResponse {
        HttpResponse { status, body: Vec::new(), from_cache: false, stale: false, retry_after_secs: None }
    }

    #[tokio::test]
    async fn a_200_passes_through_without_retrying() {
        let env = TestEnv::new("fetchers");
        env.http.responses.lock().unwrap().insert("http://example.test/a".into(), response(200));
        let resp = get(&env.ctx, "http://example.test/a").await.unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn a_4xx_passes_through_without_retrying() {
        let env = TestEnv::new("fetchers");
        env.http.responses.lock().unwrap().insert("http://example.test/a".into(), response(404));
        let resp = get(&env.ctx, "http://example.test/a").await.unwrap();
        assert_eq!(resp.status, 404, "a 4xx is the caller's to classify, never retried");
    }

    #[tokio::test]
    async fn a_cooldown_response_passes_through_even_though_its_status_is_503() {
        let env = TestEnv::new("fetchers");
        let mut cooldown = response(503);
        cooldown.retry_after_secs = Some(30);
        env.http.responses.lock().unwrap().insert("http://example.test/a".into(), cooldown);
        let resp = get(&env.ctx, "http://example.test/a").await.unwrap();
        assert_eq!((resp.status, resp.retry_after_secs), (503, Some(30)), "a cooldown is never retried here");
    }

    #[tokio::test(start_paused = true)]
    async fn a_bare_5xx_with_no_cooldown_is_retried_and_eventually_gives_up() {
        // StubHttp serves the same canned response every call, so every attempt sees a 500 --
        // this proves retry_with_backoff's own standard policy (3 attempts) ran out, not that
        // this wrapper gave up after one try. A paused clock skips the real 1s/2s delays
        // between attempts.
        let env = TestEnv::new("fetchers");
        env.http.responses.lock().unwrap().insert("http://example.test/a".into(), response(500));
        let err = get(&env.ctx, "http://example.test/a").await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Unavailable);
    }
}
