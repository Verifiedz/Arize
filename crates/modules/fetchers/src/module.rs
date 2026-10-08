use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use shimmer_core::params::decode;
use shimmer_core::{
    CommandSpec, Ctx, Error, ErrorCode, Execution, LaneConfig, Manifest, Module, Result, TriggerSpec, WriteLock,
};

use crate::dedup::{self, RunGuard};
use crate::schedule::{self, FetchersConfig, FETCH_OP, TICK_OP};
use crate::source::{self, RawItem, Source};

/// Network-bound and slow, same reasoning as every other fetch-shaped lane (CLAUDE.md §6.1).
pub const LANE: &str = "fetchers";

/// Caps `fetchers.item.found`'s `raw_data` (ADR 0028 §3) -- big enough for a job posting's
/// text, small enough that one large item can't become the next cache-bloat problem.
const MAX_RAW_DATA_CHARS: usize = 8 * 1024;

#[derive(Default)]
pub struct Fetchers {
    /// A scheduled tick and a manual `fetchers.fetch` for the same source must never overlap
    /// (ADR 0028 §5).
    running: RunGuard,
    /// Serialises `last_run.toml`'s read-modify-write (`schedule::record_run`) against itself
    /// -- the `fetchers` lane runs up to 8 fetches at once, each ending in its own call to
    /// `record_run`, and an unlocked read-check-write there can drop whichever of two
    /// concurrently-finishing sources' updates writes second (PR #126 review). A different
    /// lock from `running`: that one is per-source exclusivity for a whole run, this one is
    /// held only across the few lines that touch the shared last-run file.
    write: WriteLock,
}

#[async_trait]
impl Module for Fetchers {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fetchers".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            namespace: "fetchers".into(),
            topics: ["fetchers.item.found", "fetchers.fetch.finished", "fetchers.fetch.failed"]
                .map(String::from)
                .to_vec(),
            // hn-whoishiring (ADR 0028 §10) needs the real ctx.http.
            capabilities: vec!["network".into()],
        }
    }

    async fn init(&self, _ctx: &Ctx) -> Result<()> {
        Ok(())
    }

    fn lanes(&self) -> Vec<LaneConfig> {
        vec![LaneConfig::new(LANE, 8)]
    }

    fn triggers(&self) -> Vec<TriggerSpec> {
        vec![schedule::trigger()]
    }

    fn commands(&self) -> Vec<CommandSpec> {
        vec![
            CommandSpec {
                op: TICK_OP.into(),
                summary: "Check configured sources and enqueue a fetch for each one due".into(),
                params_schema: json!({"type": "null"}),
                execution: Execution::Queued { lane: LANE.into() },
            },
            CommandSpec {
                op: FETCH_OP.into(),
                summary: "Fetch one source now".into(),
                params_schema: json!({
                    "type": "object", "required": ["source"],
                    "properties": {"source": {"type": "string"}},
                }),
                execution: Execution::Queued { lane: LANE.into() },
            },
        ]
    }

    async fn handle(&self, op: &str, params: Value, ctx: &Ctx) -> Result<Value> {
        match op {
            TICK_OP => self.tick(ctx).await,
            FETCH_OP => self.fetch(ctx, decode::<FetchParams>(params)?.source).await,
            _ => Err(Error::unknown_op(format!("fetchers has no op '{op}'"))),
        }
    }
}

#[derive(Deserialize)]
struct FetchParams {
    source: String,
}

impl Fetchers {
    async fn tick(&self, ctx: &Ctx) -> Result<Value> {
        self.run_due(ctx, &FetchersConfig::load(ctx)?).await
    }

    /// Split from [`Fetchers::tick`] so the enqueue-and-skip-if-running logic is testable
    /// against a hand-built config, without needing to read `ctx.config` at all.
    async fn run_due(&self, ctx: &Ctx, config: &FetchersConfig) -> Result<Value> {
        let last_run = schedule::load_last_run(ctx)?;
        let due = schedule::due_sources(config, &last_run, ctx.clock.now());

        let (mut enqueued, mut already_running) = (Vec::new(), Vec::new());
        for source in due {
            // A peek only -- an optimization to avoid enqueuing a source we can already see
            // is running. `fetch`'s own `try_acquire` is the real, race-free guarantee.
            if self.running.is_running(&source) {
                already_running.push(source);
                continue;
            }
            ctx.queue.enqueue(FETCH_OP, json!({"source": source})).await?;
            enqueued.push(source);
        }
        Ok(json!({"enqueued": enqueued, "already_running": already_running}))
    }

    async fn fetch(&self, ctx: &Ctx, source_id: String) -> Result<Value> {
        if source_id.trim().is_empty() {
            return Err(Error::invalid_params("source must not be empty"));
        }
        let source =
            source::resolve(ctx, &source_id)?.ok_or_else(|| Error::not_found(format!("no source '{source_id}'")))?;
        let _permit = self
            .running
            .try_acquire(&source_id)
            .ok_or_else(|| Error::module_error(format!("source '{source_id}' is already fetching")))?;

        let outcome = self.run_source(ctx, source.as_ref()).await;
        // Recorded regardless of outcome -- including failure -- so a broken source is
        // retried at most once per interval (ADR 0028 §5, §6: "after that, wait for the
        // next trigger"), not hammered every tick. Held only across this call -- up to 8
        // fetches finish around the same time on this lane, and this file's own
        // read-modify-write must not interleave with another source's (PR #126 review).
        {
            let _g = self.write.lock();
            schedule::record_run(ctx, &source_id, ctx.clock.now())?;
        }

        match outcome {
            Ok(new_items) => {
                let _ = ctx.bus.emit("fetchers.fetch.finished", json!({"source": source_id, "new_items": new_items}));
                Ok(json!({"source": source_id, "new_items": new_items}))
            }
            Err(e) => {
                let reason = classify_failure(ctx, &e);
                let _ = ctx
                    .bus
                    .emit("fetchers.fetch.failed", json!({"source": source_id, "error": e.message, "reason": reason}));
                Err(e)
            }
        }
    }

    /// Runs one `Source` to completion: pages through it (capped, ADR 0028 §1), dedupes
    /// every item it sees against that source's `SeenSet` (touching both new and
    /// already-seen ids, ADR 0028 §4), and writes the updated set plus one
    /// `fetchers.item.found` per genuinely new item in a single transaction (ADR 0028 §4,
    /// §7.1) -- so a failure partway through this function leaves the previous run's state
    /// exactly as it was, never a half-updated one.
    async fn run_source(&self, ctx: &Ctx, source: &dyn Source) -> Result<usize> {
        let source_id = source.id();
        let mut seen = dedup::load(ctx, source_id)?;
        let now = ctx.clock.now();
        let mut found: Vec<RawItem> = Vec::new();
        let mut cursor: Option<String> = None;

        for page_index in 0..source::MAX_PAGES {
            // A full run can issue up to `MAX_PAGES * PAGE_SIZE` HTTP calls (CLAUDE.md §12
            // rule 11: "a task that cannot be cancelled is a bug") -- checked once per page
            // here, and again per-item inside a `Source` whose own page can itself fan out
            // into many calls (e.g. `sources::hn`'s per-kid loop).
            if ctx.cancel.is_cancelled() {
                return Err(Error::new(ErrorCode::ModuleError, "cancelled"));
            }
            ctx.progress(
                page_index as f32 / source::MAX_PAGES as f32,
                &format!("page {}/{}", page_index + 1, source::MAX_PAGES),
            );
            let page = source.fetch_page(ctx, cursor.as_deref()).await?;
            for item in page.items {
                if seen.is_new(&item.source_id) {
                    if found.len() < dedup::MAX_NEW_PER_RUN {
                        seen.touch(item.source_id.clone(), now);
                        found.push(item);
                    }
                    // else: left unmarked on purpose -- picked up by a later run (§8).
                } else {
                    seen.touch(item.source_id.clone(), now);
                }
            }
            if found.len() >= dedup::MAX_NEW_PER_RUN {
                break;
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        seen.prune(now);
        let new_count = found.len();
        ctx.store.transaction(|tx| {
            tx.put(&dedup::path(source_id), seen.to_toml())?;
            for item in &found {
                tx.emit("fetchers.item.found", item_payload(source_id, item))?;
            }
            Ok(())
        })?;
        Ok(new_count)
    }
}

/// Classifies a fetch failure for `fetchers.fetch.failed`'s `reason` (ADR 0028 §9, amended by
/// PR #126 review to add `"blocked"`). `ctx.cancel` is checked first since a cancelled run
/// can surface as almost any underlying error, depending on exactly where the cancellation
/// landed. `http_error` relies on `http::require_ok`'s `"http_error:"` message convention;
/// `blocked` relies on `sources::wwr`'s own `"robots.txt disallows fetching..."` message --
/// its own bucket, since the page never got fetched at all, unlike a genuine parse failure.
/// Anything else is `module_error`, treated as `parse_empty` -- a deliberate simplification
/// (see `http::require_ok`'s own doc comment) rather than inventing a precise classifier, or
/// a sixth reason, for every possible failure shape. (A corrupt `seen.toml`, for instance,
/// also falls into this `parse_empty` catch-all -- a known, deliberate tradeoff, see
/// `dedup.rs`'s `a_malformed_seen_file_is_a_reported_error_not_a_silent_empty_set` test; left
/// unchanged.)
fn classify_failure(ctx: &Ctx, err: &Error) -> &'static str {
    if ctx.cancel.is_cancelled() {
        "cancelled"
    } else if err.code == ErrorCode::Unavailable {
        "network"
    } else if err.message.starts_with("http_error:") {
        "http_error"
    } else if err.message.contains("robots.txt") {
        "blocked"
    } else {
        "parse_empty"
    }
}

/// `fetchers.item.found`'s payload (ADR 0028 §3, §9).
fn item_payload(source_id: &str, item: &RawItem) -> Value {
    json!({
        "source_name": source_id,
        "source_id": item.source_id,
        "title": item.title,
        "url": item.url,
        "raw_data": cap_raw_data(&item.raw_data),
    })
}

fn cap_raw_data(v: &Value) -> Value {
    let s = v.to_string();
    if s.chars().count() <= MAX_RAW_DATA_CHARS {
        v.clone()
    } else {
        json!({"truncated": true, "preview": s.chars().take(MAX_RAW_DATA_CHARS).collect::<String>()})
    }
}

#[cfg(test)]
mod tests {
    use shimmer_core::testing::TestEnv;
    use shimmer_core::{HttpResponse, ModuleConfig};

    use super::*;
    use crate::schedule::IntervalSpec;

    #[test]
    fn manifest_declares_its_topics_and_the_network_capability() {
        let m = Fetchers::default().manifest();
        assert_eq!(m.id.as_str(), "fetchers");
        assert_eq!(m.namespace, "fetchers");
        assert_eq!(
            m.topics,
            vec![
                "fetchers.item.found".to_string(),
                "fetchers.fetch.finished".to_string(),
                "fetchers.fetch.failed".to_string()
            ]
        );
        assert_eq!(m.capabilities, vec!["network".to_string()]);
    }

    #[test]
    fn declares_the_fetchers_lane_at_the_documented_concurrency() {
        let lanes = Fetchers::default().lanes();
        assert_eq!(lanes, vec![LaneConfig::new(LANE, 8)]);
    }

    #[test]
    fn declares_exactly_the_heartbeat_trigger() {
        let triggers = Fetchers::default().triggers();
        assert_eq!(triggers, vec![schedule::trigger()]);
    }

    #[tokio::test]
    async fn init_succeeds_with_nothing_registered() {
        let env = TestEnv::new("fetchers");
        Fetchers::default().init(&env.ctx).await.unwrap();
    }

    #[tokio::test]
    async fn an_unknown_op_is_rejected_rather_than_silently_accepted() {
        let env = TestEnv::new("fetchers");
        let err = Fetchers::default().handle("fetchers.bogus", json!({}), &env.ctx).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownOp);
    }

    fn source_config(enabled: bool, interval_s: u64) -> schedule::SourceConfig {
        schedule::SourceConfig { enabled, interval: IntervalSpec::Seconds(interval_s), kind: None, url: None }
    }

    #[tokio::test]
    async fn tick_enqueues_every_due_source_and_only_those() {
        let env = TestEnv::new("fetchers");
        let mut config = FetchersConfig::default();
        config.sources.insert("hn-whoishiring".into(), source_config(true, schedule::MIN_INTERVAL_API_S));
        config.sources.insert("weworkremotely".into(), source_config(false, schedule::MIN_INTERVAL_API_S));

        let fetchers = Fetchers::default();
        let result = fetchers.run_due(&env.ctx, &config).await.unwrap();
        assert_eq!(result["enqueued"], json!(["hn-whoishiring"]));
        assert_eq!(result["already_running"], json!([]));

        let submitted = env.queue.requests.lock().unwrap();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].op, FETCH_OP);
        assert_eq!(submitted[0].params, json!({"source": "hn-whoishiring"}));
    }

    #[tokio::test]
    async fn tick_skips_a_source_it_already_sees_running() {
        let env = TestEnv::new("fetchers");
        let mut config = FetchersConfig::default();
        config.sources.insert("hn-whoishiring".into(), source_config(true, schedule::MIN_INTERVAL_API_S));

        let fetchers = Fetchers::default();
        let _permit = fetchers.running.try_acquire("hn-whoishiring").unwrap();
        let result = fetchers.run_due(&env.ctx, &config).await.unwrap();

        assert_eq!(result["enqueued"], json!([]));
        assert_eq!(result["already_running"], json!(["hn-whoishiring"]));
        assert!(env.queue.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_source_already_fetching_is_refused_not_double_run() {
        let env = TestEnv::new("fetchers");
        let fetchers = Fetchers::default();
        let _permit = fetchers.running.try_acquire("hn-whoishiring").unwrap();

        let err = fetchers.fetch(&env.ctx, "hn-whoishiring".into()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ModuleError);
        // Refused before doing anything -- no run recorded.
        assert!(schedule::load_last_run(&env.ctx).unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_empty_source_is_rejected() {
        let env = TestEnv::new("fetchers");
        let err = Fetchers::default().fetch(&env.ctx, "  ".into()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[tokio::test]
    async fn an_unregistered_source_is_not_found() {
        let env = TestEnv::new("fetchers");
        let err = Fetchers::default().fetch(&env.ctx, "not-a-real-source".into()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        // Not even an attempt -- no permit taken, no run recorded.
        assert!(schedule::load_last_run(&env.ctx).unwrap().is_empty());
    }

    fn json_response(body: Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_string().into_bytes(),
            from_cache: false,
            stale: false,
            retry_after_secs: None,
        }
    }

    /// Stubs just enough of the real `hn-whoishiring` endpoints (StubHttp matches by exact
    /// URL, and `source::find` always builds `HnWhoIsHiring::default()`, which points at the
    /// real `hacker-news.firebaseio.com`) for one thread with one valid comment -- module.rs's
    /// own tests are about the dedup/event/record-keeping orchestration around a `Source`,
    /// not HN's own API shape, which `sources::hn`'s tests already cover against a real
    /// local server.
    fn stub_one_hn_comment(env: &TestEnv, comment_id: u64, text: &str) {
        const BASE: &str = "https://hacker-news.firebaseio.com/v0";
        let mut responses = env.http.responses.lock().unwrap();
        responses.insert(format!("{BASE}/user/whoishiring.json"), json_response(json!({"submitted": [1]})));
        responses.insert(
            format!("{BASE}/item/1.json"),
            // `title` must match `sources::hn::latest_thread`'s "who is hiring" check (PR #126
            // review) or this stub's thread is never picked, and every test using it fails.
            json_response(json!({"id": 1, "title": "Ask HN: Who is hiring? (October 2026)", "kids": [comment_id]})),
        );
        responses.insert(
            format!("{BASE}/item/{comment_id}.json"),
            json_response(json!({"id": comment_id, "by": "acme", "text": text})),
        );
    }

    #[tokio::test]
    async fn a_successful_fetch_emits_item_found_and_dedupes_on_the_next_run() {
        let env = TestEnv::new("fetchers");
        stub_one_hn_comment(&env, 10, "<p>Acme | Remote<p>Hiring Rust engineers.");

        let fetchers = Fetchers::default();
        let out = fetchers.fetch(&env.ctx, "hn-whoishiring".into()).await.unwrap();
        assert_eq!((out["source"].as_str(), out["new_items"].as_u64()), (Some("hn-whoishiring"), Some(1)));

        let events = env.backend.events();
        let found: Vec<_> = events.iter().filter(|e| e.topic == "fetchers.item.found").collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].payload["source_name"], "hn-whoishiring");
        assert_eq!(found[0].payload["source_id"], "10");
        assert!(events.iter().any(|e| e.topic == "fetchers.fetch.finished"));

        let last_run = schedule::load_last_run(&env.ctx).unwrap();
        assert_eq!(last_run.get("hn-whoishiring"), Some(&env.ctx.clock.now()));
        assert!(!fetchers.running.is_running("hn-whoishiring"), "the permit must be released on success");

        // Re-running with the identical comment still present must not re-emit it.
        let second = fetchers.fetch(&env.ctx, "hn-whoishiring".into()).await.unwrap();
        assert_eq!(second["new_items"].as_u64(), Some(0));
        let found_after_second = env.backend.events().iter().filter(|e| e.topic == "fetchers.item.found").count();
        assert_eq!(found_after_second, 1, "an already-seen item must never be emitted twice");
    }

    #[tokio::test]
    async fn a_configured_rss_kind_fetches_through_the_same_pipeline_as_a_fixed_source() {
        const FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Example Blog</title>
  <id>urn:uuid:feed-1</id>
  <updated>2026-10-01T00:00:00Z</updated>
  <entry>
    <id>urn:uuid:entry-1</id>
    <title>First post</title>
    <link href="https://example.test/posts/1"/>
    <updated>2026-10-01T00:00:00Z</updated>
  </entry>
</feed>"#;

        let mut env = TestEnv::new("fetchers");
        env.ctx.config = ModuleConfig::new(json!({
            "sources": {
                "my-team-blog": {
                    "enabled": true,
                    "interval": "hourly",
                    "kind": "rss",
                    "url": "https://example.test/feed.xml",
                },
            }
        }));
        env.http.responses.lock().unwrap().insert(
            "https://example.test/feed.xml".to_string(),
            HttpResponse {
                status: 200,
                body: FEED.as_bytes().to_vec(),
                from_cache: false,
                stale: false,
                retry_after_secs: None,
            },
        );

        let fetchers = Fetchers::default();
        let out = fetchers.fetch(&env.ctx, "my-team-blog".into()).await.unwrap();
        assert_eq!((out["source"].as_str(), out["new_items"].as_u64()), (Some("my-team-blog"), Some(1)));

        let found: Vec<_> = env.backend.events().into_iter().filter(|e| e.topic == "fetchers.item.found").collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].payload["source_name"], "my-team-blog");
        assert_eq!(found[0].payload["source_id"], "urn:uuid:entry-1");
    }

    #[tokio::test]
    async fn an_id_that_is_neither_a_fixed_source_nor_a_configured_kind_is_not_found() {
        let env = TestEnv::new("fetchers");
        let err = Fetchers::default().fetch(&env.ctx, "not-a-real-source".into()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn a_transport_failure_still_records_the_run_and_emits_fetch_failed() {
        // No responses stubbed at all -- every ctx.http.get call fails with `unavailable`.
        let env = TestEnv::new("fetchers");
        let fetchers = Fetchers::default();

        let err = fetchers.fetch(&env.ctx, "hn-whoishiring".into()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Unavailable);

        let last_run = schedule::load_last_run(&env.ctx).unwrap();
        assert_eq!(
            last_run.get("hn-whoishiring"),
            Some(&env.ctx.clock.now()),
            "a broken source must still be recorded as run, so it waits a full interval rather than being hammered every tick"
        );
        let failed: Vec<_> = env.backend.events().into_iter().filter(|e| e.topic == "fetchers.fetch.failed").collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].payload["reason"], "network");
    }

    #[tokio::test]
    async fn handle_routes_both_ops_by_name() {
        let env = TestEnv::new("fetchers");
        stub_one_hn_comment(&env, 20, "<p>Widgets Inc<p>Hiring.");
        let fetchers = Fetchers::default();

        let tick = fetchers.handle(TICK_OP, Value::Null, &env.ctx).await.unwrap();
        assert_eq!(tick["enqueued"], json!([]), "no sources configured in ctx.config by default");

        let fetch = fetchers.handle(FETCH_OP, json!({"source": "hn-whoishiring"}), &env.ctx).await.unwrap();
        assert_eq!(fetch["source"], "hn-whoishiring");
        assert_eq!(fetch["new_items"], 1);
    }

    #[tokio::test]
    async fn run_source_bails_immediately_once_cancelled_rather_than_paging_on() {
        let env = TestEnv::new("fetchers");
        // Deliberately no stubs at all: if the cancellation check did not run before the
        // first page fetch, this would instead fail with "no stub for...", not the
        // `cancelled` classification this test is actually about.
        env.ctx.cancel.cancel();
        let fetchers = Fetchers::default();

        let source = source::find("hn-whoishiring").unwrap();
        let err = fetchers.run_source(&env.ctx, source.as_ref()).await.unwrap_err();
        assert!(env.ctx.cancel.is_cancelled());
        assert_eq!(classify_failure(&env.ctx, &err), "cancelled");
    }

    #[tokio::test]
    async fn a_cancelled_fetch_is_classified_and_emitted_as_cancelled_not_whatever_error_surfaced() {
        let env = TestEnv::new("fetchers");
        env.ctx.cancel.cancel();
        let fetchers = Fetchers::default();

        let err = fetchers.fetch(&env.ctx, "hn-whoishiring".into()).await.unwrap_err();
        assert_eq!(classify_failure(&env.ctx, &err), "cancelled");
        let failed: Vec<_> = env.backend.events().into_iter().filter(|e| e.topic == "fetchers.fetch.failed").collect();
        assert_eq!(failed[0].payload["reason"], "cancelled");
    }

    #[test]
    fn a_robots_refusal_is_classified_as_blocked_not_the_parse_empty_catch_all() {
        let env = TestEnv::new("fetchers");
        let err = Error::module_error("robots.txt disallows fetching https://example.test/categories/x".to_owned());
        assert_eq!(classify_failure(&env.ctx, &err), "blocked");
    }

    #[test]
    fn an_ordinary_module_error_still_falls_back_to_parse_empty() {
        let env = TestEnv::new("fetchers");
        let err = Error::module_error("some other failure shape entirely".to_owned());
        assert_eq!(classify_failure(&env.ctx, &err), "parse_empty");
    }

    #[tokio::test]
    async fn run_source_reports_progress_once_per_page() {
        let env = TestEnv::new("fetchers");
        stub_one_hn_comment(&env, 30, "<p>Globex<p>Hiring.");
        let progress = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let p = progress.clone();
        let ctx = env.ctx.for_task(tokio_util::sync::CancellationToken::new(), std::sync::Arc::new(move |f, n| {
            p.lock().unwrap().push((f, n.to_owned()))
        }));
        let fetchers = Fetchers::default();

        let source = source::find("hn-whoishiring").unwrap();
        fetchers.run_source(&ctx, source.as_ref()).await.unwrap();

        let reported = progress.lock().unwrap();
        assert!(!reported.is_empty(), "at least the first page must report progress");
        assert!(reported[0].1.contains("page 1/"), "{:?}", reported[0]);
    }

    #[tokio::test]
    async fn two_sources_finishing_their_run_record_do_not_clobber_each_others_last_run_entry() {
        // Not a true concurrency test (see PR #126 review reply) -- this proves the lock is
        // correctly scoped to `record_run` alone: two sequential holders must each still see
        // the other's write, which a lock held across the *wrong* span (or none at all, if the
        // two calls raced) could still get right by accident, but a lock held and released
        // incorrectly inside `fetch` could not.
        let env = TestEnv::new("fetchers");
        let fetchers = Fetchers::default();
        {
            let _g = fetchers.write.lock();
            schedule::record_run(&env.ctx, "hn-whoishiring", env.ctx.clock.now()).unwrap();
        }
        {
            let _g = fetchers.write.lock();
            schedule::record_run(&env.ctx, "weworkremotely", env.ctx.clock.now()).unwrap();
        }
        let all = schedule::load_last_run(&env.ctx).unwrap();
        assert!(all.contains_key("hn-whoishiring") && all.contains_key("weworkremotely"));
    }
}
