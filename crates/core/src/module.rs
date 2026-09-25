use async_trait::async_trait;
use serde_json::Value;

use crate::ctx::Ctx;
use crate::error::Result;
use crate::event::Event;
use crate::manifest::{CommandSpec, LaneConfig, Manifest};
use crate::trigger::TriggerSpec;

#[async_trait]
pub trait Module: Send + Sync + 'static {
    /// Identity, version, owned data namespace, declared capabilities.
    fn manifest(&self) -> Manifest;

    /// One-time setup: migrations, loading configuration from the store.
    async fn init(&self, ctx: &Ctx) -> Result<()>;

    /// Execution lanes this module needs. Empty = use the shared `default` lane.
    fn lanes(&self) -> Vec<LaneConfig> {
        Vec::new()
    }

    /// Recurring or one-shot triggers registered with the scheduler.
    /// A trigger enqueues a task; it never executes one.
    fn triggers(&self) -> Vec<TriggerSpec> {
        Vec::new()
    }

    /// Operations this module answers. Names are `<module>.<verb>`.
    /// Each declares whether it runs inline or is enqueued.
    fn commands(&self) -> Vec<CommandSpec> {
        Vec::new()
    }

    /// Single entry point for all work: inline IPC requests AND queued tasks.
    /// The daemon decides which by reading the op's `CommandSpec`.
    async fn handle(&self, op: &str, params: Value, ctx: &Ctx) -> Result<Value>;

    /// React to an event emitted by any module, including this one.
    async fn on_event(&self, _ev: &Event, _ctx: &Ctx) -> Result<()> {
        Ok(())
    }
}
