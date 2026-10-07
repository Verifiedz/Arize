use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use shimmer_core::params::decode;
use shimmer_core::{CommandSpec, Ctx, Error, Execution, LaneConfig, Manifest, Module, Result, TriggerSpec};

use crate::dedup::RunGuard;
use crate::schedule::{self, FetchersConfig, FETCH_OP, TICK_OP};

/// Network-bound and slow, same reasoning as every other fetch-shaped lane (CLAUDE.md §6.1).
pub const LANE: &str = "fetchers";

#[derive(Default)]
pub struct Fetchers {
    /// A scheduled tick and a manual `fetchers.fetch` for the same source must never overlap
    /// (ADR 0028 §5).
    running: RunGuard,
}

#[async_trait]
impl Module for Fetchers {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fetchers".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            namespace: "fetchers".into(),
            // Still nothing emitted (ADR 0028 §3, §9) until a Source exists to find items.
            topics: Vec::new(),
            // Gets the real `ctx.http` once a Source needs it (ADR 0028 §10).
            capabilities: Vec::new(),
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

    async fn fetch(&self, ctx: &Ctx, source: String) -> Result<Value> {
        if source.trim().is_empty() {
            return Err(Error::invalid_params("source must not be empty"));
        }
        let _permit = self
            .running
            .try_acquire(&source)
            .ok_or_else(|| Error::module_error(format!("source '{source}' is already fetching")))?;

        // No `Source` is registered yet (ADR 0028 §10 lands in a later change) -- this
        // records the run so scheduling is fully exercisable end to end, but fetches
        // nothing. Recording happens here, regardless of whether a tick or a manual call
        // started this run, so either one resets the same clock for the next tick.
        schedule::record_run(ctx, &source, ctx.clock.now())?;
        Ok(json!({"source": source, "new_items": 0}))
    }
}

#[cfg(test)]
mod tests {
    use shimmer_core::testing::TestEnv;
    use shimmer_core::ErrorCode;

    use super::*;
    use crate::schedule::IntervalSpec;

    #[test]
    fn manifest_identifies_the_module_with_nothing_declared_yet() {
        let m = Fetchers::default().manifest();
        assert_eq!(m.id.as_str(), "fetchers");
        assert_eq!(m.namespace, "fetchers");
        assert!(m.topics.is_empty());
        assert!(m.capabilities.is_empty());
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

    fn source(enabled: bool, interval_s: u64) -> schedule::SourceConfig {
        schedule::SourceConfig { enabled, interval: IntervalSpec::Seconds(interval_s) }
    }

    #[tokio::test]
    async fn tick_enqueues_every_due_source_and_only_those() {
        let env = TestEnv::new("fetchers");
        let mut config = FetchersConfig::default();
        config.sources.insert("hn-whoishiring".into(), source(true, schedule::MIN_INTERVAL_S));
        config.sources.insert("weworkremotely".into(), source(false, schedule::MIN_INTERVAL_S));

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
        config.sources.insert("hn-whoishiring".into(), source(true, schedule::MIN_INTERVAL_S));

        let fetchers = Fetchers::default();
        let _permit = fetchers.running.try_acquire("hn-whoishiring").unwrap();
        let result = fetchers.run_due(&env.ctx, &config).await.unwrap();

        assert_eq!(result["enqueued"], json!([]));
        assert_eq!(result["already_running"], json!(["hn-whoishiring"]));
        assert!(env.queue.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_records_the_run_and_releases_its_permit_on_success() {
        let env = TestEnv::new("fetchers");
        let fetchers = Fetchers::default();

        let out = fetchers.fetch(&env.ctx, "hn-whoishiring".into()).await.unwrap();
        assert_eq!(out["source"], "hn-whoishiring");

        let last_run = schedule::load_last_run(&env.ctx).unwrap();
        assert_eq!(last_run.get("hn-whoishiring"), Some(&env.ctx.clock.now()));
        // The permit must have been released -- a second fetch of the same source succeeds.
        assert!(!fetchers.running.is_running("hn-whoishiring"));
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
    async fn handle_routes_both_ops_by_name() {
        let env = TestEnv::new("fetchers");
        let fetchers = Fetchers::default();
        let tick = fetchers.handle(TICK_OP, Value::Null, &env.ctx).await.unwrap();
        assert_eq!(tick["enqueued"], json!([]), "no sources configured in ctx.config by default");

        let fetch = fetchers.handle(FETCH_OP, json!({"source": "hn-whoishiring"}), &env.ctx).await.unwrap();
        assert_eq!(fetch["source"], "hn-whoishiring");
    }
}
