//! The real `HttpBackend` (ADR 0027). Rate limiting and redirects land here; the on-disk
//! response cache is a later slice of the same umbrella.
//!
//! Cross-platform — unlike the launcher, nothing here is `cfg(unix)`-gated.

// Nothing outside this module's own tests constructs a `RealHttpBackend` yet -- the
// `"network"` capability gate that wires it into `Ctx` (ADR 0027 §4) is a later commit in
// this same umbrella. Remove this once that lands.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use shimmer_core::{Clock, Error, HttpBackend, HttpResponse, ModuleId, Result};
use tokio::sync::Semaphore;

/// A cooling-down host is never waited on past this if the real `Retry-After` is longer —
/// the caller gets the last-known failure back immediately instead (ADR 0027 §3).
const DEFAULT_MAX_COOLDOWN_WAIT: Duration = Duration::from_secs(30);
/// Used only when a `429`/`503` carries no usable `Retry-After`.
const DEFAULT_COOLDOWN_SECS: u64 = 30;
const MAX_REDIRECTS: u8 = 10;

/// Per-host limits. Falls back to `HttpConfig`'s `default_*` fields when a host has no entry
/// of its own.
#[derive(Clone, Copy, Debug)]
pub struct HostLimits {
    pub requests_per_sec: f64,
    pub concurrent: usize,
}

/// What `Core::new` builds once from `config.toml`'s `[http]` section (a later slice) and
/// hands to `RealHttpBackend::new`. Defaults are deliberately conservative — a module that
/// opts into `"network"` should not be able to hammer a host by accident.
#[derive(Clone, Debug)]
pub struct HttpConfig {
    pub default_requests_per_sec: f64,
    pub default_concurrent: usize,
    /// Keyed on `host:port`, matching how `RealHttpBackend` buckets everything else.
    pub per_host: HashMap<String, HostLimits>,
    pub timeout: Duration,
    pub max_cooldown_wait: Duration,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            default_requests_per_sec: 2.0,
            default_concurrent: 2,
            per_host: HashMap::new(),
            timeout: Duration::from_secs(30),
            max_cooldown_wait: DEFAULT_MAX_COOLDOWN_WAIT,
        }
    }
}

/// A classic token bucket: starts full, refills continuously at `rate_per_sec`, capped at a
/// small burst (`rate_per_sec`, floor 1 — a very slow rate still allows the first request
/// through immediately rather than stalling it on an empty bucket).
#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    rate_per_sec: f64,
    capacity: f64,
    last_refill: DateTime<Utc>,
}

impl TokenBucket {
    fn new(rate_per_sec: f64, now: DateTime<Utc>) -> Self {
        let capacity = rate_per_sec.max(1.0);
        Self { tokens: capacity, rate_per_sec, capacity, last_refill: now }
    }

    /// Takes one token if available, refilling first for the elapsed time. `Err` carries how
    /// long until one will be.
    fn try_take(&mut self, now: DateTime<Utc>) -> std::result::Result<(), Duration> {
        let elapsed_secs = (now - self.last_refill).num_milliseconds().max(0) as f64 / 1000.0;
        self.tokens = (self.tokens + elapsed_secs * self.rate_per_sec).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else if self.rate_per_sec > 0.0 {
            Err(Duration::from_secs_f64(((1.0 - self.tokens) / self.rate_per_sec).max(0.0)))
        } else {
            Err(Duration::from_secs(u64::MAX / 2))
        }
    }
}

struct HostMutable {
    bucket: TokenBucket,
    cooldown_until: Option<DateTime<Utc>>,
    /// The status (`429`/`503`) that set the current cooldown, so a wait past the cap can
    /// still hand back something meaningful instead of inventing a status.
    last_cooldown_status: Option<u16>,
}

struct HostEntry {
    mutable: Mutex<HostMutable>,
    semaphore: Arc<Semaphore>,
}

/// The gateway never retries (§11.2's principle, applied here too) — one network attempt (or
/// one synthesized cooldown response) per `get` call. Redirects are its own internal loop,
/// not a retry: each hop is a different request to a different URL.
pub struct RealHttpBackend {
    client: reqwest::Client,
    clock: Clock,
    config: HttpConfig,
    hosts: Mutex<HashMap<String, Arc<HostEntry>>>,
}

impl RealHttpBackend {
    pub fn new(config: HttpConfig, clock: Clock) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::internal(format!("building http client: {e}")))?;
        Ok(Self { client, clock, config, hosts: Mutex::new(HashMap::new()) })
    }

    fn host_entry(&self, host_key: &str) -> Arc<HostEntry> {
        let mut hosts = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        hosts
            .entry(host_key.to_owned())
            .or_insert_with(|| {
                let limits = self.config.per_host.get(host_key).copied().unwrap_or(HostLimits {
                    requests_per_sec: self.config.default_requests_per_sec,
                    concurrent: self.config.default_concurrent,
                });
                Arc::new(HostEntry {
                    mutable: Mutex::new(HostMutable {
                        bucket: TokenBucket::new(limits.requests_per_sec, self.clock.now()),
                        cooldown_until: None,
                        last_cooldown_status: None,
                    }),
                    semaphore: Arc::new(Semaphore::new(limits.concurrent.max(1))),
                })
            })
            .clone()
    }

    /// Rate limit, wait out any cooldown (or synthesize past it), then send exactly one
    /// request. Never follows a redirect itself — the caller's loop decides that.
    async fn send_one(&self, module: &ModuleId, url: &reqwest::Url) -> Result<HopOutcome> {
        let host_key = host_key(url)?;
        let entry = self.host_entry(&host_key);

        let cooldown_remaining = {
            let m = entry.mutable.lock().unwrap_or_else(|e| e.into_inner());
            m.cooldown_until.and_then(|until| {
                let now = self.clock.now();
                (until > now).then(|| (until - now).to_std().unwrap_or(Duration::ZERO))
            })
        };
        if let Some(remaining) = cooldown_remaining {
            if remaining > self.config.max_cooldown_wait {
                let status =
                    entry.mutable.lock().unwrap_or_else(|e| e.into_inner()).last_cooldown_status.unwrap_or(503);
                tracing::debug!(module = %module, host = %host_key, remaining_s = remaining.as_secs(), "http: cooling down past the wait cap, not sending");
                return Ok(HopOutcome::Synthesized(HttpResponse {
                    status,
                    body: Vec::new(),
                    from_cache: false,
                    stale: false,
                    retry_after_secs: Some(remaining.as_secs().max(1)),
                }));
            }
            tokio::time::sleep(remaining).await;
        }

        loop {
            let wait = {
                let mut m = entry.mutable.lock().unwrap_or_else(|e| e.into_inner());
                match m.bucket.try_take(self.clock.now()) {
                    Ok(()) => break,
                    Err(wait) => wait,
                }
            };
            tokio::time::sleep(wait).await;
        }

        let _permit = entry
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::internal("http concurrency limiter closed unexpectedly"))?;

        tracing::debug!(module = %module, url = %url.as_str(), "http: sending");
        let resp = self.client.get(url.clone()).send().await.map_err(|e| Error::unavailable(format!("{url}: {e}")))?;
        Ok(HopOutcome::Response(resp))
    }

    /// Reads the body and, on a `429`/`503`, records this host's cooldown before returning.
    async fn finish(&self, url: &reqwest::Url, resp: reqwest::Response) -> Result<HttpResponse> {
        let status = resp.status().as_u16();
        let retry_after = retry_after_secs(&resp);
        if status == 429 || status == 503 {
            self.record_cooldown(url, status, retry_after)?;
        }
        let body = resp.bytes().await.map_err(|e| Error::unavailable(format!("{url}: {e}")))?.to_vec();
        Ok(HttpResponse {
            status,
            body,
            from_cache: false,
            stale: false,
            retry_after_secs: (status == 429 || status == 503).then(|| retry_after.unwrap_or(DEFAULT_COOLDOWN_SECS)),
        })
    }

    fn record_cooldown(&self, url: &reqwest::Url, status: u16, retry_after_secs: Option<u64>) -> Result<()> {
        let host_key = host_key(url)?;
        let entry = self.host_entry(&host_key);
        let secs = retry_after_secs.unwrap_or(DEFAULT_COOLDOWN_SECS);
        let mut m = entry.mutable.lock().unwrap_or_else(|e| e.into_inner());
        m.cooldown_until = Some(self.clock.now() + chrono::Duration::seconds(secs as i64));
        m.last_cooldown_status = Some(status);
        Ok(())
    }
}

enum HopOutcome {
    Response(reqwest::Response),
    Synthesized(HttpResponse),
}

#[async_trait]
impl HttpBackend for RealHttpBackend {
    async fn get(&self, module: &ModuleId, url: &str) -> Result<HttpResponse> {
        let mut current = parse_checked_url(url)?;
        for hop in 0..=MAX_REDIRECTS {
            match self.send_one(module, &current).await? {
                HopOutcome::Synthesized(r) => return Ok(r),
                HopOutcome::Response(resp) => {
                    if resp.status().is_redirection() {
                        if hop == MAX_REDIRECTS {
                            return Err(Error::unavailable(format!("too many redirects fetching {url}")));
                        }
                        if let Some(next) = redirect_target(&current, &resp) {
                            current = next;
                            continue;
                        }
                    }
                    return self.finish(&current, resp).await;
                }
            }
        }
        unreachable!("the loop above always returns before exhausting its range")
    }
}

fn parse_checked_url(url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).map_err(|e| Error::invalid_params(format!("invalid url '{url}': {e}")))?;
    check_scheme(&parsed)?;
    Ok(parsed)
}

fn check_scheme(url: &reqwest::Url) -> Result<()> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(Error::invalid_params(format!("unsupported url scheme '{other}' in '{url}'; only http/https"))),
    }
}

fn host_key(url: &reqwest::Url) -> Result<String> {
    let host = url.host_str().ok_or_else(|| Error::invalid_params(format!("http url has no host: {url}")))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::invalid_params(format!("http url has no known port: {url}")))?;
    Ok(format!("{host}:{port}"))
}

fn redirect_target(base: &reqwest::Url, resp: &reqwest::Response) -> Option<reqwest::Url> {
    let location = resp.headers().get(reqwest::header::LOCATION)?.to_str().ok()?;
    let next = base.join(location).ok()?;
    (check_scheme(&next).is_ok()).then_some(next)
}

fn retry_after_secs(resp: &reqwest::Response) -> Option<u64> {
    resp.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use shimmer_core::ErrorCode;

    use super::*;

    const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";

    /// Hand-rolled HTTP/1.1 server on an ephemeral loopback port: serves exactly the given
    /// responses in order, one per accepted connection, and records each request's raw head
    /// so a test can assert on what was actually sent (e.g. `If-None-Match`). No real
    /// network, no new dependency (ADR 0027 §9).
    struct TestServer {
        addr: std::net::SocketAddr,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl TestServer {
        fn start(responses: Vec<&'static str>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_clone = requests.clone();
            std::thread::spawn(move || {
                for response in responses {
                    let Ok((mut stream, _)) = listener.accept() else { return };
                    let mut buf = [0u8; 8192];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    requests_clone.lock().unwrap().push(String::from_utf8_lossy(&buf[..n]).into_owned());
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            Self { addr, requests }
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}{}", self.addr, path)
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }

    fn backend(config: HttpConfig, clock: Clock) -> RealHttpBackend {
        RealHttpBackend::new(config, clock).unwrap()
    }

    fn generous_config() -> HttpConfig {
        HttpConfig { default_requests_per_sec: 1000.0, default_concurrent: 10, ..HttpConfig::default() }
    }

    #[tokio::test]
    async fn non_http_scheme_is_rejected_before_any_network_work() {
        let b = backend(generous_config(), Clock::system());
        let err = b.get(&ModuleId::new("test"), "ftp://example.com/file").await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[tokio::test]
    async fn a_plain_get_round_trips_status_and_body() {
        let server = TestServer::start(vec![OK]);
        let b = backend(generous_config(), Clock::system());
        let resp = b.get(&ModuleId::new("test"), &server.url("/a")).await.unwrap();
        assert_eq!((resp.status, resp.body), (200, b"ok".to_vec()));
        assert!(!resp.from_cache && !resp.stale && resp.retry_after_secs.is_none());
    }

    #[tokio::test]
    async fn the_first_request_to_a_host_never_waits_even_at_a_slow_rate() {
        // The bucket starts full, so the very first request through a fresh host entry must
        // not stall on it, however slow the configured steady rate is.
        let server = TestServer::start(vec![OK]);
        let mut cfg = generous_config();
        cfg.default_requests_per_sec = 0.01;
        let b = backend(cfg, Clock::system());
        tokio::time::timeout(Duration::from_secs(2), b.get(&ModuleId::new("test"), &server.url("/a")))
            .await
            .expect("the first request must not wait for a token")
            .unwrap();
    }

    #[tokio::test]
    async fn two_different_hosts_never_share_a_bucket_or_a_concurrency_cap() {
        let a = TestServer::start(vec![OK]);
        let b_server = TestServer::start(vec![OK]);
        let mut cfg = generous_config();
        cfg.default_requests_per_sec = 0.001; // so a shared bucket would starve the second host
        cfg.default_concurrent = 1;
        let backend = backend(cfg, Clock::system());
        let module = ModuleId::new("test");
        tokio::time::timeout(Duration::from_secs(2), async {
            backend.get(&module, &a.url("/a")).await.unwrap();
            backend.get(&module, &b_server.url("/b")).await.unwrap();
        })
        .await
        .expect("a different host must get its own fresh bucket, not share the first host's");
    }

    #[tokio::test]
    async fn a_429_records_a_cooldown_that_a_bounded_wait_then_honors() {
        let retry_after_1s = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\n\r\n";
        let server = TestServer::start(vec![retry_after_1s, OK]);
        let mut cfg = generous_config();
        cfg.max_cooldown_wait = Duration::from_secs(5); // comfortably above the 1s Retry-After
        let b = backend(cfg, Clock::system());
        let module = ModuleId::new("test");

        let first = b.get(&module, &server.url("/a")).await.unwrap();
        assert_eq!((first.status, first.retry_after_secs), (429, Some(1)));

        // Second call hits the same host while it's cooling down. Since 1s <= the 5s cap,
        // this must actually wait out the cooldown and then succeed for real -- the one
        // place in this test suite where a short, deliberate real wait is unavoidable,
        // since a real wait is exactly the behavior under test.
        let second =
            tokio::time::timeout(Duration::from_secs(3), b.get(&module, &server.url("/a"))).await.unwrap().unwrap();
        assert_eq!((second.status, second.body), (200, b"ok".to_vec()));
        assert_eq!(server.request_count(), 2, "it waited and sent a real second request, not a synthesized one");
    }

    #[tokio::test]
    async fn a_cooldown_past_the_wait_cap_is_synthesized_without_touching_the_network() {
        // A fake, unadvanced clock: the exact remaining-seconds math (no real time may pass
        // between recording the cooldown and checking it) is itself part of what's asserted.
        let clock = Clock::fake(DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap().to_utc());
        let retry_after_1h = "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 3600\r\nContent-Length: 0\r\n\r\n";
        let server = TestServer::start(vec![retry_after_1h]);
        let mut cfg = generous_config();
        cfg.max_cooldown_wait = Duration::from_millis(50);
        let b = backend(cfg, clock);
        let module = ModuleId::new("test");

        let first = b.get(&module, &server.url("/a")).await.unwrap();
        assert_eq!((first.status, first.retry_after_secs), (503, Some(3600)));

        // The cooldown is an hour, the cap is 50ms -- the next call must return instantly
        // with a synthesized response, never reaching the server at all.
        let second =
            tokio::time::timeout(Duration::from_millis(500), b.get(&module, &server.url("/a"))).await.unwrap().unwrap();
        assert_eq!((second.status, second.retry_after_secs), (503, Some(3600)));
        assert_eq!(server.request_count(), 1, "the second call must never touch the network");
    }

    #[tokio::test]
    async fn a_cross_host_redirect_is_followed_and_rate_limited_by_the_new_host() {
        let b_server = TestServer::start(vec![OK]);
        let redirect = format!("HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\n\r\n", b_server.url("/b"));
        let a_server = TestServer::start(vec![Box::leak(redirect.into_boxed_str())]);
        let backend = backend(generous_config(), Clock::system());

        let resp = backend.get(&ModuleId::new("test"), &a_server.url("/a")).await.unwrap();
        assert_eq!((resp.status, resp.body), (200, b"ok".to_vec()));
        assert_eq!(b_server.request_count(), 1, "the redirect target must actually have been requested");
    }

    #[test]
    fn token_bucket_refills_over_time_and_reports_a_bounded_wait() {
        let t0 = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap().to_utc();
        let mut bucket = TokenBucket::new(2.0, t0); // capacity 2.0
        assert_eq!(bucket.try_take(t0), Ok(()));
        assert_eq!(bucket.try_take(t0), Ok(()));
        // Bucket is now empty; immediately retrying must report a wait, not succeed.
        let wait = bucket.try_take(t0).unwrap_err();
        assert!(wait > Duration::ZERO && wait <= Duration::from_secs(1));
        // After a full second at rate 2/s, at least one token is available again.
        assert_eq!(bucket.try_take(t0 + chrono::Duration::seconds(1)), Ok(()));
    }
}
